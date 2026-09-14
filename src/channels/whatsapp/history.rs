//! Bounded WhatsApp history sync (#1525).
//!
//! WhatsApp answers a PDO history request by streaming encrypted sync frames
//! back over the normal socket; the wa crate decrypts them and re-emits the
//! messages through the same event stream, distinguishable only by the
//! per-message markers (`is_offline`, `unavailable_request_id`). Per the
//! decision doc (~/.opencrabs/research/whatsapp-history-sync-decisions-1525.md):
//!
//! - **Store**: the existing `channel_messages` partition the live capture
//!   already uses, keyed by the SAME normalized `chat_id` the live path
//!   formats (#1544), tagged `message_type = "imported"` and deduped by
//!   platform message id.
//! - **Opt-in**: per-chat, OFF by default (`[channels.whatsapp]
//!   history_import_chats`) — a config edit IS the consent gesture, so there
//!   is no global enable and no agent-side write surface.
//! - **Bounds**: hard caps, whichever hits first — 10,000 messages per chat
//!   or 90 days back (the request plan below), plus a window filter at
//!   capture for frames the phone answers outside the range.
//! - **Surface**: read-only search through the `whatsapp_history` tool.
//!
//! The request trigger lives in `agent.rs` (fires on connect); the capture
//! gate lives in `handle_message` (frames must never wake the agent). This
//! module owns only the decisions, as pure functions.

use chrono::{DateTime, Duration, Utc};

/// Cap on messages pulled per chat per request (decision 3).
pub(crate) const HISTORY_IMPORT_MAX_MESSAGES: i32 = 10_000;

/// How far back an import may go (decision 3).
pub(crate) const HISTORY_IMPORT_MAX_AGE_DAYS: i64 = 90;

/// Message type tag distinguishing imported rows from live captures in the
/// shared `channel_messages` store (decision 1).
pub(crate) const IMPORTED_TYPE: &str = "imported";

/// One bounded PDO history request, anchored on a message already stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportPlan {
    pub oldest_msg_id: String,
    pub oldest_from_me: bool,
    pub oldest_ts_ms: i64,
    pub count: i32,
}

/// Decide whether an import request is worth sending for one chat.
///
/// `oldest` is the chat's earliest stored row (platform id, whether the
/// sender is us, its timestamp). Semantics: the phone answers with up to
/// `count` messages OLDER than the anchor.
///
/// - No rows yet: anchor an empty id at `now` — the phone starts from the
///   newest messages (the only supported head).
/// - Anchor already at or before the 90-day cutoff: nothing in-window can
///   remain below it — `None`, do not ask.
/// - Otherwise: full page, capped at [`HISTORY_IMPORT_MAX_MESSAGES`].
pub(crate) fn plan_import(
    oldest: Option<(&str, bool, DateTime<Utc>)>,
    now: DateTime<Utc>,
) -> Option<ImportPlan> {
    let cutoff = now - Duration::days(HISTORY_IMPORT_MAX_AGE_DAYS);
    let (id, from_me, ts) = match oldest {
        Some(row) => row,
        None => ("", false, now),
    };
    if ts <= cutoff {
        return None;
    }
    Some(ImportPlan {
        oldest_msg_id: id.to_string(),
        oldest_from_me: from_me,
        oldest_ts_ms: ts.timestamp_millis(),
        count: HISTORY_IMPORT_MAX_MESSAGES,
    })
}

/// Window filter for frames arriving at the capture gate: the phone may
/// answer with messages older than the 90-day bound; those are dropped.
pub(crate) fn in_window(ts: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    ts > now - Duration::days(HISTORY_IMPORT_MAX_AGE_DAYS)
}

/// Per-chat opt-in (decision 2): exact match on the normalized chat id the
/// live path formats (`Display` of the source JID), never a phone prefix
/// scan — #1544's lesson is that loose key matching merges partitions.
pub(crate) fn opted_in(chats: &[String], chat_id: &str) -> bool {
    chats.iter().any(|c| c == chat_id)
}

/// Whether a stored row's sender identifies us, for the anchor's `from_me`
/// flag. No owner known yet is conservatively `false` (the phone tolerates
/// it; the anchor is still exact on id + timestamp).
pub(crate) fn from_me_of(sender_id: &str, owner: Option<&str>) -> bool {
    owner.is_some_and(|o| !o.is_empty() && sender_id == o)
}

/// Clamp the read-only search tool's knobs to the feature's bounds
/// (decision 3's window applies to retrieval too, and an unbounded
/// `LIMIT` would defeat the point of a soft store).
pub(crate) fn clamp_search(days: i64, limit: i64) -> (i64, i64) {
    (
        days.clamp(1, HISTORY_IMPORT_MAX_AGE_DAYS),
        limit.clamp(1, 100),
    )
}
