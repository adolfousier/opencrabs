//! Notify receipts (fork #50, confirmation depth 3): the sender's
//! `notify_id` becomes a checkable contract instead of a fire-and-forget
//! hope. The session message queue is per-target, so when the tool-loop
//! drain point fires for a target, every notify queued for that target is
//! consumed by definition — no id field on `QueuedUserMessage` is needed,
//! the drain stamps by target and the sender polls by id.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use uuid::Uuid;

/// Lifecycle of a routed notify as observed by the receiving machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceiptState {
    /// Routed into the queue or the wake path; not yet observed at a drain
    /// point.
    Queued,
    /// Observed at the tool-loop drain point — injected into the model
    /// context between tool iterations.
    Injected,
}

impl ReceiptState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ReceiptState::Queued => "queued",
            ReceiptState::Injected => "injected",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NotifyReceipt {
    pub target: Uuid,
    pub state: ReceiptState,
    pub queued_at: chrono::DateTime<chrono::Utc>,
    pub injected_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn receipts() -> &'static Mutex<HashMap<Uuid, NotifyReceipt>> {
    static RECEIPTS: OnceLock<Mutex<HashMap<Uuid, NotifyReceipt>>> = OnceLock::new();
    RECEIPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record that `id` was routed to `target` and awaits consumption.
pub(crate) fn record_queued(id: Uuid, target: Uuid) {
    let receipt = NotifyReceipt {
        target,
        state: ReceiptState::Queued,
        queued_at: chrono::Utc::now(),
        injected_at: None,
    };
    match receipts().lock() {
        Ok(mut map) => {
            map.insert(id, receipt);
        }
        Err(e) => {
            tracing::error!(
                target: "quiet_delivery",
                "notify receipts poisoned, receipt {id} not recorded: {e}"
            );
        }
    }
}

/// Reserve `id` for `target` BEFORE any delivery happens (#199) — the
/// idempotency leg of the notify retry. `true` means this call is the FIRST
/// sight of the id: the caller owns this notify and may deliver it. `false`
/// means a prior attempt already reserved it, so the notify is delivered or in
/// flight and only its RESPONSE was lost — the caller must report the prior
/// outcome instead of delivering a second copy.
///
/// The reservation lives only in this process, so a daemon restart clears it:
/// a retry that spans a restart delivers again rather than deduping against a
/// dead entry. A poisoned lock FAILS OPEN (`true`) — dropping a notify is
/// worse than the rare duplicate a lost mutex buys.
pub(crate) fn reserve(id: Uuid, target: Uuid) -> bool {
    let receipt = NotifyReceipt {
        target,
        state: ReceiptState::Queued,
        queued_at: chrono::Utc::now(),
        injected_at: None,
    };
    match receipts().lock() {
        Ok(mut map) => match map.entry(id) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(receipt);
                true
            }
            std::collections::hash_map::Entry::Occupied(_) => false,
        },
        Err(e) => {
            tracing::error!(
                target: "quiet_delivery",
                "notify receipts poisoned, receipt {id} not reserved: {e}"
            );
            true
        }
    }
}

/// Drop a reservation whose notify was NOT delivered (#199). A real verdict
/// (`no_route`, `refused_in_flight`) is final and is never retried, so its id
/// must not sit in the store claiming a queued notify — a later
/// `session/notify-status` poll on it would report a delivery that never
/// happened.
pub(crate) fn forget(id: Uuid) {
    if let Ok(mut map) = receipts().lock() {
        map.remove(&id);
    }
}

/// Stamp every queued receipt for `target` as injected. Called at the
/// tool-loop drain point, which consumes the session's whole queue — a
/// queued receipt that is still `Queued` after a drain for its target
/// would be a lie to the sender. Returns the number stamped.
pub(crate) fn mark_injected_for_target(target: Uuid) -> usize {
    let now = chrono::Utc::now();
    match receipts().lock() {
        Ok(mut map) => {
            let mut stamped = 0;
            for receipt in map.values_mut() {
                if receipt.target == target && receipt.state == ReceiptState::Queued {
                    receipt.state = ReceiptState::Injected;
                    receipt.injected_at = Some(now);
                    stamped += 1;
                }
            }
            stamped
        }
        Err(e) => {
            tracing::error!(
                target: "quiet_delivery",
                "notify receipts poisoned, injection stamp lost for {target}: {e}"
            );
            0
        }
    }
}

/// Sender-facing status check by id.
pub(crate) fn status(id: Uuid) -> Option<NotifyReceipt> {
    receipts().lock().ok()?.get(&id).cloned()
}
