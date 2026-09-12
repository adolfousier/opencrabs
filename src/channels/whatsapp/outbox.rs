//! Outbound message-id bookkeeping for the WhatsApp channel (#1408).
//!
//! `Client::send_message` returns `SendResult { message_id, to }`, and every
//! call site in this channel used to match `Ok(_)` and drop it. Four features
//! need that id back: edit-in-place streaming (#1408), self-reactions on
//! completion turns (#1409), poll-vote decoding (#1482) and pin/forward
//! (#1484). It is recorded here once, keyed by session, instead of four
//! call sites each growing their own map.
//!
//! The entry also carries the rendered body, because an edit replaces the
//! whole message: to append a streamed chunk we must resend the full text,
//! not the delta.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use tokio::sync::Mutex;
use uuid::Uuid;

/// WhatsApp refuses an edit older than 15 minutes. Checking locally turns a
/// guaranteed-failing round trip into a cheap fallback to a fresh send.
pub(crate) const EDIT_WINDOW: Duration = Duration::from_secs(15 * 60);

/// A message we sent and may still want to edit, react to, or quote.
#[derive(Debug, Clone)]
pub(crate) struct OutboxEntry {
    /// Server-assigned id, from `SendResult::message_id`.
    pub message_id: String,
    /// Full rendered text currently displayed for this message.
    pub body: String,
    /// When the original send landed. Drives [`OutboxEntry::editable`].
    pub sent_at: SystemTime,
}

impl OutboxEntry {
    pub fn new(message_id: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            body: body.into(),
            sent_at: SystemTime::now(),
        }
    }

    /// False once the 15-minute window has closed, so callers append a new
    /// message instead of burning a round trip on an edit the server rejects.
    /// A clock that jumped backwards yields `Err` from `elapsed`; treat that
    /// as still-editable and let the server be the judge.
    pub fn editable(&self) -> bool {
        match self.sent_at.elapsed() {
            Ok(age) => age < EDIT_WINDOW,
            Err(_) => true,
        }
    }
}

/// Per-session record of the last editable message the bot sent.
///
/// One entry per session: streaming only ever edits the message it is
/// currently growing, and a new turn starts a new one.
#[derive(Default)]
pub(crate) struct Outbox {
    entries: Mutex<HashMap<Uuid, OutboxEntry>>,
}

impl Outbox {
    /// Remember a freshly sent message, replacing any previous entry for the
    /// session.
    pub async fn record(&self, session: Uuid, entry: OutboxEntry) {
        self.entries.lock().await.insert(session, entry);
    }

    /// The session's current editable message, if any.
    pub async fn get(&self, session: Uuid) -> Option<OutboxEntry> {
        self.entries.lock().await.get(&session).cloned()
    }

    /// Replace the stored body after a successful edit, so the next append
    /// builds on what the user is actually looking at.
    ///
    /// Returns false when the session has no entry, which means the edit
    /// raced a `clear` and the caller should fall back to a fresh send.
    pub async fn set_body(&self, session: Uuid, body: impl Into<String>) -> bool {
        match self.entries.lock().await.get_mut(&session) {
            Some(entry) => {
                entry.body = body.into();
                true
            }
            None => false,
        }
    }

    /// Forget the session's entry. Called when a turn ends, and whenever an
    /// edit fails, so the rest of that turn degrades to plain appends
    /// instead of retrying a message the server will not accept.
    pub async fn clear(&self, session: Uuid) -> Option<OutboxEntry> {
        self.entries.lock().await.remove(&session)
    }

    /// Number of tracked sessions. Exists for the tests that assert a
    /// replacement does not grow the map; nothing in the channel needs it.
    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.entries.lock().await.len()
    }
}

impl super::WhatsAppState {
    /// Remember the message a session just sent, so the next streamed chunk
    /// can edit it in place instead of posting a new one.
    pub(crate) async fn record_outbound(&self, session_id: Uuid, entry: OutboxEntry) {
        self.outbox.record(session_id, entry).await;
    }

    /// The session's current editable message, if the 15-minute window is
    /// still open. A stale entry is dropped here rather than handed back, so
    /// the caller sees `None` and appends instead.
    pub(crate) async fn editable_outbound(&self, session_id: Uuid) -> Option<OutboxEntry> {
        let entry = self.outbox.get(session_id).await?;
        if entry.editable() {
            return Some(entry);
        }
        tracing::debug!(
            target: "whatsapp",
            session = %session_id,
            message_id = %entry.message_id,
            "outbox entry past the 15-minute edit window; falling back to append"
        );
        self.outbox.clear(session_id).await;
        None
    }

    /// Record the new full text after an edit landed.
    pub(crate) async fn update_outbound_body(&self, session_id: Uuid, body: impl Into<String>) {
        if !self.outbox.set_body(session_id, body).await {
            tracing::debug!(
                target: "whatsapp",
                session = %session_id,
                "outbox entry vanished before the body update; next chunk will send fresh"
            );
        }
    }

    /// Drop the session's entry. Called when a turn ends and whenever an edit
    /// fails, so the remainder of the turn degrades to plain appends.
    pub(crate) async fn clear_outbound(&self, session_id: Uuid) {
        self.outbox.clear(session_id).await;
    }
}
