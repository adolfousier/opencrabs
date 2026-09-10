//! Session to chat-JID map so a finished background task can resume the
//! chat that started it (#731). WhatsApp keeps no other session-to-target
//! map; the handler registers the pair on every turn.

use uuid::Uuid;

use super::WhatsAppState;

impl WhatsAppState {
    /// Map a session to the chat JID it is being handled in, so a finished
    /// background task can resume that chat (#731). Called on each turn.
    pub async fn register_session_jid(&self, session_id: Uuid, jid: String) {
        self.session_jids
            .lock()
            .await
            .insert(session_id, jid.clone());
        // Reverse ownership map (#148): written beside the forward map so the
        // two cannot drift. Last writer wins — a JID re-registers to its
        // newest owning session.
        self.jid_sessions.lock().await.insert(jid, session_id);
    }

    /// The chat JID a session was last handled in, if known (#731).
    pub async fn session_jid(&self, session_id: Uuid) -> Option<String> {
        self.session_jids.lock().await.get(&session_id).cloned()
    }

    /// Reverse lookup (#148): the session currently bound to a WhatsApp JID,
    /// for `oc://whatsapp/<jid>` resolution. Reads the reverse map kept in
    /// lockstep with the forward map at `register_session_jid`.
    pub async fn session_owner_by_jid(&self, jid: &str) -> Option<Uuid> {
        self.jid_sessions.lock().await.get(jid).copied()
    }
}
