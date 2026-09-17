//! Shared recent-history preamble for multi-party surfaces (#133, #682).
//!
//! Telegram grew this logic first; Discord, Slack and WhatsApp each re-injected
//! their full history window every turn because they never got it (#1618,
//! #1619, #1620). The rendering is identical across surfaces apart from one
//! noun ("group" vs "channel") and the log label, so it lives here once rather
//! than four times.
//!
//! Two independent jobs:
//!
//! 1. **Dedup** against what the live session already holds. After a
//!    compaction the model still has the last N turns in context, so
//!    re-injecting the same messages burns tokens and makes the model answer a
//!    history line instead of the current one.
//! 2. **Framing** (#682), so the block reads as prior context from various
//!    senders rather than as the message being replied to.

use crate::brain::agent::AgentService;
use crate::db::MessageRepository;
use crate::db::models::ChannelMessage as DbChannelMessage;

/// Normalize text for deduplication comparison.
/// Collapses all whitespace sequences and converts to lowercase.
pub(crate) fn normalize_for_dedup(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Check whether `candidate` is present in `live_context_haystacks`.
/// Each haystack is assumed to already be normalized with `normalize_for_dedup`.
pub(crate) fn is_content_in_live_context(
    candidate: &str,
    live_context_haystacks: &[String],
) -> bool {
    let norm = normalize_for_dedup(candidate);
    if norm.is_empty() {
        return false;
    }
    live_context_haystacks.iter().any(|h| h.contains(&norm))
}

/// Frame the recent-history block (#682). Marks the lines as prior context from
/// VARIOUS senders, so the model answers the trailing current message rather
/// than replying to a history sender.
///
/// `noun` is the surface's word for the room: `"group"` on Telegram and
/// WhatsApp, `"channel"` on Discord and Slack.
pub(crate) fn frame_history(history_lines: &str, count: usize, noun: &str) -> String {
    format!(
        "[Recent {noun} history ({count} messages) — prior context from various senders, NOT the \
         person you are replying to now:\n{history_lines}\n--- end history ---]"
    )
}

/// Label the CURRENT sender so the model never addresses them by a name that
/// only appears in the injected history (the bug: the owner was called "Adi"
/// because a different member named Adi was in the history).
///
/// `surface` names the room type for the reader, e.g. `"Telegram group"`,
/// `"Discord channel"`. `role` is "owner" or "user"; `handle` is `" (@name)"`
/// or empty.
pub(crate) fn current_sender_label(
    surface: &str,
    chat_title: &str,
    name: &str,
    handle: &str,
    role: &str,
) -> String {
    format!(
        "[{surface} \"{chat_title}\" — the message below is from {name}{handle} ({role}). \
         Reply to {name}. Any names in the history above belong to OTHER people; never address \
         {name} by a name that appears only in that history.]"
    )
}

/// Render fetched rows as `[HH:MM] sender: content` lines, oldest first.
///
/// `recent()` returns newest-first, so this reverses; a history block read
/// backwards teaches the model the wrong order of events.
pub(crate) fn render_history_lines(messages: &[DbChannelMessage]) -> String {
    messages
        .iter()
        .rev()
        .map(|m| {
            let ts = m.created_at.format("%H:%M");
            format!("[{}] {}: {}", ts, m.sender_name, m.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Normalized content of every session message since the last compaction.
///
/// Anything in here is already in front of the model, so injecting it again is
/// pure waste. Best-effort: a read failure yields an empty haystack list, which
/// degrades to "inject everything" rather than to a lost turn.
pub(crate) async fn live_context_haystacks(
    pool: crate::db::Pool,
    session_id: uuid::Uuid,
) -> Vec<String> {
    let msg_repo = MessageRepository::new(pool);
    let all_msgs = msg_repo
        .find_by_session(session_id)
        .await
        .unwrap_or_default();
    AgentService::messages_from_last_compaction(all_msgs)
        .iter()
        .map(|m| normalize_for_dedup(&m.content))
        .collect()
}

/// Build the framed history preamble for `messages`, or `None` when there is
/// nothing left to inject.
///
/// `None` means "skip injection entirely", which is the whole point: it is
/// returned both when nothing was fetched and when every fetched row is already
/// in live context. `label` prefixes the log lines ("Telegram", "Discord", ...)
/// and `noun` picks the surface's word for the room.
pub(crate) async fn build_preamble(
    pool: crate::db::Pool,
    session_id: uuid::Uuid,
    messages: Vec<DbChannelMessage>,
    noun: &str,
    label: &str,
) -> Option<String> {
    if messages.is_empty() {
        return None;
    }
    let live_haystacks = live_context_haystacks(pool, session_id).await;

    let total_fetched = messages.len();
    let filtered: Vec<_> = messages
        .into_iter()
        .filter(|m| !is_content_in_live_context(&m.content, &live_haystacks))
        .collect();

    if filtered.is_empty() {
        tracing::info!(
            "{label}: all {total_fetched} recent {noun} history messages are already in live session context — skipping injection (#133)"
        );
        return None;
    }

    tracing::info!(
        "{label}: injecting {} uncompacted {noun} history messages (filtered from {total_fetched})",
        filtered.len()
    );
    let lines = render_history_lines(&filtered);
    Some(frame_history(&lines, filtered.len(), noun))
}
