//! Durable parking for undelivered pushes (#111).
//!
//! Every chokepoint that parks a push in memory (`PARKED` for
//! restart-recovery reports, `pending_reactions` for Telegram mid-turn
//! follow-ups) is wiped by a process kill. The `notify_queue` table is the
//! durable twin: each in-memory park also writes one row, and each consume
//! site clears the matching rows on delivery — the same posture the
//! tombstone persistence (#73) established.
//!
//! Best-effort by design, both directions:
//! - A failed persist costs a loud error log, the push rides the in-memory
//!   queue alone (exactly the pre-#111 behavior).
//! - A failed clear costs a stale row that boot redelivery replays — a
//!   DUPLICATE after restart, never a lost message.
//!
//! `None` before the DB is initialized (early startup, tests) simply means
//! durable parking is skipped, mirroring [`super::restart_recovery`]'s
//! tombstone posture. Entry points are runtime-agnostic: outside a live
//! tokio runtime (sync tests, pre-boot) the work is skipped with a debug
//! log rather than panicking — a sync context cannot await the DB anyway,
//! and the in-memory queue still holds the push.

use super::types::QueuedUserMessage;
use crate::db::NotifyQueueRepository;
use uuid::Uuid;

/// A row that keeps surviving boots has no clear path. Rows older than this
/// are logged as defect candidates at boot (#111 follow-up, Part C). Age is a
/// schema-free stand-in for a redelivery count, which would have needed a new
/// column — and the shipped table is deliberately unchanged.
const STALE_ROW_SECS: i64 = 24 * 60 * 60;

/// Maximum age a row may survive in notify_queue before being reaped (#182).
/// Rows older than STALE_ROW_SECS (24h) emit warnings; rows older than
/// MAX_ROW_AGE_SECS (72h / 3 days) are dropped at boot so dead routes do not
/// leak rows indefinitely.
pub const MAX_ROW_AGE_SECS: i64 = 3 * STALE_ROW_SECS; // 72h (259,200s)

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}
fn repo() -> Option<NotifyQueueRepository> {
    crate::db::global_pool().map(|p| NotifyQueueRepository::new(p.clone()))
}

/// Runtime-agnostic fire-and-forget: run the future on the tokio runtime
/// when one is live, log-and-drop when none is (sync tests, pre-boot).
fn spawn_if_runtime<F>(future: F, what: &'static str)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(future);
        }
        Err(_) => {
            tracing::debug!(
                target: "background_task",
                "No tokio runtime live: skipping {what} (durable parking unavailable)"
            );
        }
    }
}

/// Persist an undelivered push so a restart cannot lose it (#111).
///
/// Called at the chokepoints that park a push in memory. Best-effort: a
/// failure is logged loudly and the push rides the in-memory queue alone.
pub(crate) fn persist(session_id: Uuid, msg: &QueuedUserMessage) {
    let Some(repo) = repo() else {
        return;
    };
    let (context_text, display_text) = (msg.context_text.clone(), msg.display_text.clone());
    let (origin, bg_meta) = (msg.origin, msg.bg_meta.clone());
    spawn_if_runtime(
        async move {
            if let Err(e) = repo
                .record(
                    Uuid::new_v4(),
                    session_id,
                    &context_text,
                    &display_text,
                    origin,
                    bg_meta.as_ref(),
                )
                .await
            {
                tracing::error!(
                    target: "background_task",
                    "Could not persist undelivered push for session {session_id}: it rides \
                     the in-memory queue alone and the next restart will lose it: {e:#}"
                );
            }
        },
        "notify_queue persist",
    );
}

/// Re-offer every persisted push from a previous process (#111).
///
/// Runs from [`super::restart_recovery::recover`] alongside the tombstone
/// redelivery. A row whose push lands on a live route is cleared; one that
/// parks again keeps its row and keeps surviving restarts until the park is
/// genuinely delivered (the consume sites clear it). Returns how many rows
/// were re-offered.
pub(crate) async fn redeliver_persisted() -> usize {
    let Some(repo) = repo() else {
        return 0;
    };
    // Reap rows whose session no longer exists FIRST (#111 follow-up, Part B):
    // no channel can ever claim them, so no consume site can ever clear them,
    // and re-offering them would only re-park a push nobody can receive.
    match repo.clear_dead_sessions().await {
        Ok(n) if n > 0 => tracing::info!(
            target: "background_task",
            "Boot notify queue: reaped={n} row(s) whose session no longer exists"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(
            target: "background_task",
            "Could not reap notify-queue rows for dead sessions: {e:#}"
        ),
    }

    // Reap rows older than MAX_ROW_AGE_SECS (72h) for unclaimed sessions (#182):
    // Sessions that exist in DB but have no active channel route park pushes forever.
    // At boot, reap them to halt table bloat and memory leaks, logging full audit telemetry.
    let cutoff = now_unix().saturating_sub(MAX_ROW_AGE_SECS);
    match repo.reap_stale_unclaimed(cutoff).await {
        Ok(reaped) if !reaped.is_empty() => {
            tracing::info!(
                target: "background_task",
                "Boot notify queue: reaped {} stale unclaimed push(es) older than {}h",
                reaped.len(),
                MAX_ROW_AGE_SECS / 3600
            );
            for row in reaped {
                let age_h = now_unix().saturating_sub(row.created_at) / 3600;
                let clean_text = row.context_text.replace(['\n', '\r'], " ");
                let preview: String = clean_text.trim().chars().take(120).collect();
                tracing::error!(
                    target: "background_task",
                    "Notify queue reaper: dropped undeliverable push {} for unclaimed session {} (age {}h > {}h, origin={:?}): {}",
                    row.id,
                    row.session_id,
                    age_h,
                    MAX_ROW_AGE_SECS / 3600,
                    row.origin,
                    preview
                );
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(
            target: "background_task",
            "Could not reap stale unclaimed notify-queue rows: {e:#}"
        ),
    }
    let rows = match repo.all().await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(
                target: "background_task",
                "Could not read persisted notify-queue rows: {e:#}"
            );
            return 0;
        }
    };
    let mut count = 0usize;
    for row in rows {
        // Origin and bg_meta are preserved: the echo rendering and the
        // receipt path depend on them, and the row IS the message.
        let msg = QueuedUserMessage {
            context_text: row.context_text.clone(),
            display_text: row.display_text.clone(),
            origin: row.origin,
            bg_meta: row.bg_meta.clone(),
        };
        if super::restart_recovery::deliver_or_park(row.session_id, msg)
            && let Err(e) = repo.clear(row.id).await
        {
            // Worst case the push is delivered twice, never zero times.
            tracing::error!(
                target: "background_task",
                "Delivered persisted push {} but could not clear its row; it may be \
                 re-delivered after the next restart: {e:#}",
                row.id
            );
        }
        count += 1;
    }
    count
}

/// Clear the durable twin of a push that was just delivered.
///
/// Exact-content match so any OTHER undelivered push for the same session
/// survives. Fire-and-forget: a failure costs a stale row replayed at next
/// boot — a duplicate, never a loss (#111).
pub(crate) fn clear_on_delivery(session_id: Uuid, msg: &QueuedUserMessage) {
    let Some(repo) = repo() else {
        return;
    };
    let (context_text, display_text) = (msg.context_text.clone(), msg.display_text.clone());
    spawn_if_runtime(
        async move {
            if let Err(e) = repo
                .clear_matching(session_id, &context_text, &display_text)
                .await
            {
                tracing::warn!(
                    target: "background_task",
                    "Could not clear delivered push from the durable notify queue for \
                     session {session_id}: next boot may redeliver it (a duplicate, never \
                     a loss): {e:#}"
                );
            }
        },
        "notify_queue clear",
    );
}
