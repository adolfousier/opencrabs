//! session_notify tool — push a message into another live session's queue.
//!
//! Thin agent-facing wrapper over `session_routes::deliver_to_session`, so an
//! orchestrator (e.g. a compiling agent) can hand work back to the sessions
//! whose commits broke the build (issue adolfousier/opencrabs#1203).
//!
//! Sender identity is injected MECHANICALLY from `ToolExecutionContext`
//! — the calling model can neither forge nor omit the `[session-notify
//! from=<uuid>]` header prepended to every delivery.

use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

/// Resolved v2 delivery policy (fork #50).
#[derive(Debug)]
pub(crate) enum DeliveryMode {
    /// Refuse while the target is mid-turn (the failsafe default).
    Now,
    /// Queue for the target's next tool-loop boundary (alias: interrupt=true).
    TurnEnd,
    /// Defer until the target has been quiet for `quiet_for`; `max_delay`
    /// forces delivery into a busy turn (fork #43/#50).
    Quiet {
        quiet_for: Duration,
        max_delay: Duration,
    },
}

/// Tool that pushes a queued user message into another session.
pub struct SessionNotifyTool;

/// Short display form of a session UUID (first 8 hex chars).
fn short_id(id: uuid::Uuid) -> String {
    id.simple().to_string()[..8].to_string()
}

/// Redirect-acknowledgement text for a delivery steered to the channel's
/// current owner (fork #19). Pure so tests can pin the wording: the sender
/// must hear WHERE the message went without a second discovery round.
fn redirect_message(target: uuid::Uuid, occupant: uuid::Uuid) -> String {
    format!(
        "Redirected: session {target} no longer owns its channel — the chat/topic it was \
         bound to is now occupied by session {occupant} (a newer session replaced it, e.g. \
         an idle-timeout reset took over the topic). The message was delivered to {occupant} \
         instead, with provenance framing so the new owner can tell it apart from its own \
         work."
    )
}

/// Confirmation budget for `confirm: true`: how long the sender watches the
/// receiving machinery for a wake before falling back to an honest
/// "routed, unconfirmed" verdict.
const CONFIRM_CAP: Duration = Duration::from_secs(10);

/// Machine-readable send verdict (Notifications v2, fork #50): the external
/// `notify_state` mirrors the internal `Delivery` enum 1:1 — delivered /
/// queued / redirected / refused (+ `notify_reason`, `notify_occupant`, …) —
/// so callers never parse prose. The human text stays in `output` for the
/// calling model; the structured fields ride `metadata`.
pub(crate) fn verdict(
    success: bool,
    state: &str,
    detail: String,
    extra: &[(&str, String)],
) -> ToolResult {
    let mut result = if success {
        ToolResult::success(detail)
    } else {
        ToolResult::error(detail)
    };
    result = result.with_metadata("notify_state".into(), state.into());
    for (key, value) in extra {
        result = result.with_metadata((*key).to_string(), value.clone());
    }
    result
}

/// Post-route confirmation (owner-approved state-diag, 2026-09-01): watch
/// the receiving machinery for a bounded budget instead of reporting
/// "delivered" as a mere queue hand-off. The wake path is verifiable
/// in-process — a channel-registered turn probe flips to true the moment
/// the target's loop starts — so the sender gets `woke` (idle target
/// started a turn), `queued_pending_drain` (already mid-turn; the message
/// injects at its next tool-loop boundary), or an honest `delivered`
/// (routed, but no wake observed within `cap`).
pub(crate) async fn confirm_route(
    target: uuid::Uuid,
    cap: Duration,
) -> (&'static str, String, &'static str) {
    use crate::brain::agent::service::session_routes::turn_probe;
    let mid_turn = |t| turn_probe(t).is_some_and(|probe| probe());

    if mid_turn(target) {
        return (
            "queued_pending_drain",
            "Confirmed queued: the target is mid-turn; the message injects at its next \
             tool-loop boundary."
                .into(),
            "mid_turn",
        );
    }
    let deadline = tokio::time::Instant::now() + cap;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if mid_turn(target) {
            return (
                "woke",
                "Confirmed end-to-end: the target was idle and has started a turn on the \
                 message."
                    .into(),
                "wake_confirmed",
            );
        }
    }
    (
        "delivered",
        format!(
            "Routed to session {target}, but no wake was observed within {}s — the \
             target may be parked (channel not claimed since boot) or slow to pick the \
             message up. Re-check via session_search before resending.",
            cap.as_secs()
        ),
        "unconfirmed",
    )
}

/// Depth-3 status check (fork #50): `action: "status"` polls the receipt a
/// send verdict carried. In-memory by design — receipts die with the
/// process, so an unknown id after a restart reports honestly instead of
/// guessing. DELIVERY ≠ QUEUE ACCEPTANCE becomes a query, not a hope: the
/// tool-loop drain point stamps receipts `injected` when the target's queue
/// is actually consumed.
pub(crate) fn status_verdict(input: &Value) -> Result<ToolResult> {
    use crate::brain::agent::service::notify_receipts::{self, ReceiptState};
    let raw = input
        .get("notify_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ToolError::InvalidInput("'notify_id' is required for action 'status'".into())
        })?;
    let id: uuid::Uuid = raw
        .parse()
        .map_err(|_| ToolError::InvalidInput(format!("'notify_id' is not a valid UUID: {raw}")))?;
    match notify_receipts::status(id) {
        None => Ok(verdict(
            false,
            "unknown_id",
            format!(
                "No notification {id} is tracked in this process — receipts are in-memory \
                 and do not survive restarts. A receipt stamped injected before a restart \
                 counted as consumed."
            ),
            &[("notify_id", id.to_string())],
        )),
        Some(receipt) => {
            let mut extra = vec![
                ("notify_id", id.to_string()),
                ("notify_target", receipt.target.to_string()),
                ("queued_at", receipt.queued_at.to_rfc3339()),
            ];
            let detail = match receipt.state {
                ReceiptState::Injected => {
                    let at = receipt
                        .injected_at
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_default();
                    extra.push(("injected_at", at.clone()));
                    format!(
                        "Notification {id} was INJECTED into session {}'s model context at \
                         {at} — the receiving machinery consumed it.",
                        receipt.target
                    )
                }
                ReceiptState::Queued => format!(
                    "Notification {id} is routed to session {} but NOT yet observed at a \
                     tool-loop drain point (queued {}). Delivery ≠ queue acceptance: it \
                     injects when that session's turn hits a boundary.",
                    receipt.target,
                    receipt.queued_at.to_rfc3339()
                ),
            };
            Ok(verdict(true, receipt.state.as_str(), detail, &extra))
        }
    }
}

/// Resolve the v2 delivery policy against the deprecated `interrupt` alias
/// (fork #50). `interrupt=true` was always "queue for the in-flight turn's
/// next tool-loop boundary" — that is mode `turn-end`; unset/false was
/// "refuse while streaming" — mode `now`. Both may be passed only when they
/// agree; a disagreement is an error, never a silent precedence. `quiet`
/// defers until the target has been idle for `quiet_for_secs` (starvation
/// cap `max_delay_secs` forces delivery into a busy turn).
pub(crate) fn resolve_mode(
    mode: Option<&str>,
    interrupt: Option<bool>,
    delivery: Option<&Value>,
) -> Result<DeliveryMode> {
    fn secs(parent: Option<&Value>, key: &str, default: u64) -> Result<Duration> {
        match parent.and_then(|d| d.get(key)) {
            None => Ok(Duration::from_secs(default)),
            Some(v) => {
                let n = v.as_u64().ok_or_else(|| {
                    ToolError::InvalidInput(format!(
                        "delivery.{key} must be a non-negative integer"
                    ))
                })?;
                Ok(Duration::from_secs(n))
            }
        }
    }
    let resolved = match mode {
        None => None,
        Some(known @ ("now" | "turn-end")) => Some(known),
        Some("quiet") => {
            // quiet contradicts interrupt=true by definition: quiet WAITS,
            // turn-end DERAILS. interrupt=false/unset is the natural form.
            if interrupt == Some(true) {
                return Err(ToolError::InvalidInput(
                    "delivery.mode 'quiet' and interrupt=true disagree — quiet defers, \
                     interrupt derails"
                        .into(),
                ));
            }
            let quiet_for = secs(delivery, "quiet_for_secs", 60)?;
            let max_delay = secs(delivery, "max_delay_secs", 1800)?;
            return Ok(DeliveryMode::Quiet {
                quiet_for,
                max_delay,
            });
        }
        Some(other) => {
            return Err(ToolError::InvalidInput(format!(
                "delivery.mode '{other}' is not available yet — use 'now', 'turn-end' or 'quiet'"
            )));
        }
    };
    match (resolved, interrupt) {
        (Some("turn-end"), None | Some(true)) | (None, Some(true)) => Ok(DeliveryMode::TurnEnd),
        (Some("now"), None | Some(false)) | (None, None | Some(false)) => Ok(DeliveryMode::Now),
        _ => Err(ToolError::InvalidInput(
            "delivery.mode and interrupt disagree — pass one, not both".into(),
        )),
    }
}

#[async_trait]
impl Tool for SessionNotifyTool {
    fn name(&self) -> &str {
        "session_notify"
    }

    fn description(&self) -> &str {
        "Push a message to another session's queue in this process. The target \
         drains it at its next tool-loop boundary, or wakes immediately if idle. \
         Refuses while the target is mid-turn unless interrupt=true — do not \
         derail a working session by default. When the target no longer \
         owns its channel (a newer session replaced it on its \
         chat/topic), the message is REDIRECTED to the occupying session \
         with provenance framing, and delivery reports the redirect; \
         interrupt does NOT override that gate. Every delivery carries a \
         mechanical header [session-notify from=<sender session id>]; to reply, \
         call session_notify with target_session set to that id. Discover \
         target ids via session_search list/query."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["send", "status"],
                    "description": "Operation on the notification family. 'send' (default when omitted) delivers; 'status' polls a receipt by notify_id — reports 'injected' (the receiving machinery stamped it at a tool-loop drain point), 'queued' (routed but not yet consumed), or 'unknown_id' (not tracked; receipts are in-memory and do not survive restarts)."
                },
                "target_session": {
                    "type": "string",
                    "description": "UUID of the target session (from session_search list/query, or the from=<id> header of a session_notify you received)"
                },
                "message": {
                    "type": "string",
                    "description": "Text to deliver to the target session (action 'send' only)"
                },
                "notify_id": {
                    "type": "string",
                    "description": "action 'status' only: the notification id from a send/deferred verdict's metadata"
                },
                "delivery": {
                    "type": "object",
                    "description": "Delivery policy. Omit for the default (mode 'now').",
                    "properties": {
                        "mode": {
                            "type": "string",
                            "enum": ["now", "turn-end", "quiet"],
                            "description": "'now' (default): deliver immediately; REFUSES while the target is mid-turn. 'turn-end': queue the message for the target's next tool-loop boundary even while it streams. 'quiet': defer until the target has been idle for quiet_for_secs (any turn activity restarts the clock; max_delay_secs forces delivery into a busy turn so the notice cannot be starved forever); returns a deferred verdict with a notification id."
                        },
                        "quiet_for_secs": {
                            "type": "integer",
                            "description": "quiet mode only: idle window before delivery (default 60)."
                        },
                        "max_delay_secs": {
                            "type": "integer",
                            "description": "quiet mode only: starvation cap — deliver at latest this long after acceptance, even mid-turn (default 1800)."
                        }
                    }
                },
                "confirm": {
                    "type": "boolean",
                    "description": "Verify end-to-end instead of reporting the route alone: after a successful route, spend up to ~10s watching the receiving machinery. The verdict then reports 'woke' (the idle target actually started a turn), 'queued_pending_drain' (the target was already mid-turn; the message injects at its next tool-loop boundary), or 'delivered' (routed, but no wake was observed within the cap). Applies to the 'delivered' and 'redirected' states only."
                },
                "interrupt": {
                    "type": "boolean",
                    "description": "Deprecated alias for delivery.mode: true = 'turn-end', false/unset = 'now'. Prefer delivery.mode; passing both is allowed only when they agree."
                }
            },
            "required": ["target_session"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        // #129 belt-and-braces (owner ruling C): a headless session has no
        // turn-end relay to flush parked notifications — the send would die
        // with the one-shot process. Fail LOUDLY; the final message is the
        // sanctioned report channel.
        if context.headless {
            return Err(ToolError::Execution(
                "session_notify is not available headless — a one-shot process cannot \
                 deliver parked notifications. Put the substance in your final \
                 message; the harness relays it."
                    .into(),
            ));
        }
        let target = input
            .get("target_session")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'target_session' is required".into()))?;
        let target: uuid::Uuid = target.parse().map_err(|_| {
            ToolError::InvalidInput(format!("'target_session' is not a valid UUID: {target}"))
        })?;

        // v2 surface (fork #50): `action` dispatches the verb family. v1
        // shipped "send" only; depth-3 receipts add "status" — a real
        // consumer for the notification ids the send verdicts carry.
        match input
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("send")
        {
            "send" => {}
            "status" => return status_verdict(&input),
            other => {
                return Err(ToolError::InvalidInput(format!(
                    "action '{other}' is not available yet — available: 'send', 'status'"
                )));
            }
        }

        let message = input
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'message' is required".into()))?;
        if message.trim().is_empty() {
            return Ok(ToolResult::error(
                "Refusing to send an empty message".to_string(),
            ));
        }

        // Mechanical sender signature — from execution context, not model text.
        let from = context.session_id;
        let msg = crate::brain::agent::QueuedUserMessage {
            context_text: format!("[session-notify from={from}]\n\n{message}"),
            display_text: format!("📨 notify from {}:\n{message}", short_id(from)),
            origin: crate::brain::agent::PushOrigin::SessionNotify,
            bg_meta: None,
        };

        // Failsafe default (fork #13): an unset interrupt must not derail a
        // session that is mid-turn. v2 (fork #50) re-expresses the knob as
        // the delivery policy — the alias keeps old prompts working.
        let delivery_obj = input.get("delivery");
        let mode = resolve_mode(
            delivery_obj
                .and_then(|d| d.get("mode"))
                .and_then(Value::as_str),
            input.get("interrupt").and_then(Value::as_bool),
            delivery_obj,
        )?;

        use crate::brain::agent::service::notify_receipts;
        use crate::brain::agent::service::quiet_delivery;
        use crate::brain::agent::service::session_routes::{Delivery, deliver_to_session};

        // Quiet mode (fork #43/#50): bank the notice, return the id. The
        // deferred verdict is success-by-contract — accepted, not yet
        // delivered; the id is the cancel/list handle from birth.
        if let DeliveryMode::Quiet {
            quiet_for,
            max_delay,
        } = mode
        {
            let id = quiet_delivery::defer_quiet(target, msg, quiet_for, max_delay);
            // The deferred id is a first-class notification id: status-checkable
            // like any send receipt (stamped injected when the release drains).
            notify_receipts::record_queued(id, target);
            return Ok(verdict(
                true,
                "deferred",
                format!(
                    "Deferred for session {target}: it will deliver once the session has been \
                     quiet for {}s (hard cap {}s). Notification id {id}.",
                    quiet_for.as_secs(),
                    max_delay.as_secs()
                ),
                &[
                    ("notify_target", target.to_string()),
                    ("notify_id", id.to_string()),
                    ("notify_reason", "quiet_window".into()),
                ],
            ));
        }

        let interrupt = matches!(mode, DeliveryMode::TurnEnd);
        let confirm = input
            .get("confirm")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Depth-3 receipt (fork #50): every success-path verdict carries an
        // id the sender can poll with action:"status" — the tool-loop drain
        // point stamps it injected when the target's queue is consumed.
        let notify_id = uuid::Uuid::new_v4();
        match deliver_to_session(target, msg, interrupt) {
            Delivery::Delivered => {
                notify_receipts::record_queued(notify_id, target);
                if confirm {
                    let (state, detail, reason) = confirm_route(target, CONFIRM_CAP).await;
                    return Ok(verdict(
                        true,
                        state,
                        detail,
                        &[
                            ("notify_target", target.to_string()),
                            ("notify_id", notify_id.to_string()),
                            ("notify_reason", reason.into()),
                        ],
                    ));
                }
                Ok(verdict(
                    true,
                    "delivered",
                    format!(
                        "Delivered to session {target}. It will process the message on its next \
                         turn. Poll action:\"status\" with notify_id for the injection stamp."
                    ),
                    &[
                        ("notify_target", target.to_string()),
                        ("notify_id", notify_id.to_string()),
                    ],
                ))
            }
            // Queued, not lost: the target belongs to a channel that has not
            // claimed it since the last restart (#1206). Reporting this as a
            // failure would be the opposite of what happened.
            Delivery::Parked => {
                notify_receipts::record_queued(notify_id, target);
                Ok(verdict(
                    true,
                    "queued",
                    format!(
                        "Queued for session {target}. Its channel has not claimed it since the \
                         last restart, so it will be delivered as soon as that channel next \
                         binds the session. Poll action:\"status\" with notify_id."
                    ),
                    &[
                        ("notify_target", target.to_string()),
                        ("notify_id", notify_id.to_string()),
                        ("notify_reason", "awaiting_channel_claim".into()),
                    ],
                ))
            }
            Delivery::RefusedInFlight { redirected_to } => {
                let who = match redirected_to {
                    Some(to) => format!(
                        "{to} (mid-turn — the message was redirected there because \
                         {target} no longer owns its channel)"
                    ),
                    None => target.to_string(),
                };
                let mut extra = vec![
                    ("notify_target", target.to_string()),
                    ("notify_reason", "mid_turn".to_string()),
                ];
                if let Some(to) = redirected_to {
                    extra.push(("notify_redirected_to", to.to_string()));
                }
                Ok(verdict(
                    false,
                    "refused",
                    format!(
                        "Refused: session {who} is mid-turn (a turn is streaming) and interrupt \
                         was not set — delivering now would derail its current task. Retry when \
                         the session goes idle, or resend with interrupt=true to queue the \
                         message for its in-flight turn's next tool-loop boundary."
                    ),
                    &extra,
                ))
            }
            Delivery::NoRoute => Ok(verdict(
                false,
                "refused",
                format!(
                    "No live route for session {target} in this process — it has not messaged \
                     since boot, or belongs to another instance/profile. Use a2a_send for \
                     cross-instance targets."
                ),
                &[
                    ("notify_target", target.to_string()),
                    ("notify_reason", "no_route".to_string()),
                ],
            )),
            // The target no longer owns its channel (fork #17): the message
            // was redirected to the session that owns it NOW (fork #19) — a
            // success, not a refusal, and the reply names where it went.
            Delivery::Redirected { to } => {
                notify_receipts::record_queued(notify_id, to);
                if confirm {
                    let (state, detail, reason) = confirm_route(to, CONFIRM_CAP).await;
                    return Ok(verdict(
                        true,
                        state,
                        format!("{detail} (redirected to {to} — see notify_occupant)"),
                        &[
                            ("notify_target", target.to_string()),
                            ("notify_id", notify_id.to_string()),
                            ("notify_occupant", to.to_string()),
                            ("notify_reason", reason.into()),
                        ],
                    ));
                }
                Ok(verdict(
                    true,
                    "redirected",
                    redirect_message(target, to),
                    &[
                        ("notify_target", target.to_string()),
                        ("notify_id", notify_id.to_string()),
                        ("notify_occupant", to.to_string()),
                    ],
                ))
            }
        }
    }
}
