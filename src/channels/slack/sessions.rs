//! Session to channel-id map so approvals and resumed background results
//! route back to the channel the session was last handled in.

use uuid::Uuid;

use super::SlackState;

impl SlackState {
    /// Record which channel_id corresponds to a given session.
    pub async fn register_session_channel(&self, session_id: Uuid, channel_id: String) {
        self.session_channels
            .lock()
            .await
            .insert(session_id, channel_id.clone());
        // Reverse ownership map (#148): written beside the forward map so the
        // two cannot drift. Last writer wins — a channel re-registers to its
        // newest owning session.
        self.channel_sessions
            .lock()
            .await
            .insert(channel_id, session_id);
    }

    /// Look up the channel_id for a session.
    pub async fn session_channel(&self, session_id: Uuid) -> Option<String> {
        self.session_channels.lock().await.get(&session_id).cloned()
    }

    /// Reverse lookup (#148): the session currently bound to a Slack channel id,
    /// for `oc://slack/<id>` resolution. Reads the reverse map kept in lockstep
    /// with the forward map at `register_session_channel`.
    pub async fn session_owner_by_channel(&self, channel_id: &str) -> Option<Uuid> {
        self.channel_sessions
            .lock()
            .await
            .get(channel_id)
            .copied()
    }
}
