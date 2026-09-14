//! Optional follow-up suggestions from `suggest_options` (#600).
//!
//! Suggestions render as a native-flow card when `interactive_buttons` is on
//! and the set fits the button cap (#1411), and as a numbered text list
//! otherwise. Either way a bare numeric reply selects one (card taps are
//! rewritten to the number), the set is consumed on a valid selection, and it
//! is cleared by the handler on any other message.

use uuid::Uuid;

use super::WhatsAppState;

impl WhatsAppState {
    /// Stash this session's optional follow-up suggestions (#600).
    pub async fn set_pending_followups(&self, session_id: Uuid, options: Vec<String>) {
        self.pending_followups
            .lock()
            .await
            .insert(session_id, options);
    }

    /// If this session has pending suggestions and `reply` parses as a 1-based
    /// option number in range, consume the whole set and return the chosen
    /// suggestion. Returns None otherwise (leaving the set for the caller to
    /// clear on a non-selecting message).
    pub async fn take_followup_by_reply(&self, session_id: Uuid, reply: &str) -> Option<String> {
        let parsed: usize = reply.trim().parse().ok()?;
        if parsed == 0 {
            return None;
        }
        let mut map = self.pending_followups.lock().await;
        let options = map.get(&session_id)?;
        let chosen = options.get(parsed - 1).cloned();
        if chosen.is_some() {
            map.remove(&session_id);
        }
        chosen
    }

    /// Drop this session's pending follow-up suggestions (non-selecting message).
    pub async fn clear_pending_followups(&self, session_id: Uuid) {
        self.pending_followups.lock().await.remove(&session_id);
    }
}
