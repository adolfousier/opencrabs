//! Trello Integration
//!
//! Polls Trello board(s) for new card comments, routes them to the AI,
//! and replies by posting comments back on the card.

mod agent;
pub mod client;
pub(crate) mod handler;
pub mod models;

pub use agent::TrelloAgent;
pub use client::TrelloClient;

use tokio::sync::Mutex;

/// Shared Trello state — holds credentials after connect.
/// Read by the `trello_send` tool to perform proactive card operations.
pub struct TrelloState {
    api_key: Mutex<Option<String>>,
    api_token: Mutex<Option<String>>,
    /// Bot's own Trello member ID — set at connect, used to skip own comments.
    bot_member_id: Mutex<Option<String>>,
    connected: Mutex<bool>,
}

impl Default for TrelloState {
    fn default() -> Self {
        Self::new()
    }
}

impl TrelloState {
    pub fn new() -> Self {
        Self {
            api_key: Mutex::new(None),
            api_token: Mutex::new(None),
            bot_member_id: Mutex::new(None),
            connected: Mutex::new(false),
        }
    }

    /// Store credentials once successfully verified.
    pub async fn set_credentials(&self, api_key: String, api_token: String) {
        *self.api_key.lock().await = Some(api_key);
        *self.api_token.lock().await = Some(api_token);
    }

    /// Store the bot's own member ID (set from `GET /members/me`).
    pub async fn set_bot_member_id(&self, id: String) {
        *self.bot_member_id.lock().await = Some(id);
    }

    /// Mark the agent as connected / disconnected.
    pub async fn set_connected(&self, val: bool) {
        *self.connected.lock().await = val;
    }

    /// Return `true` if the agent has successfully authenticated.
    pub async fn is_connected(&self) -> bool {
        *self.connected.lock().await
    }

    /// Return the stored API key + token pair, if available.
    pub async fn credentials(&self) -> Option<(String, String)> {
        let key = self.api_key.lock().await.clone()?;
        let token = self.api_token.lock().await.clone()?;
        Some((key, token))
    }

    /// Return the bot's Trello member ID.
    pub async fn bot_member_id(&self) -> Option<String> {
        self.bot_member_id.lock().await.clone()
    }
}
