//! Slack Integration
//!
//! Runs a Slack bot via Socket Mode alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;

pub use agent::SlackAgent;

use slack_morphism::prelude::SlackHyperClient;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

/// Shared Slack state for proactive messaging.
///
/// Set when the bot connects via Socket Mode.
/// Read by the `slack_send` tool to send messages on demand.
pub struct SlackState {
    client: Mutex<Option<Arc<SlackHyperClient>>>,
    bot_token: Mutex<Option<String>>,
    /// Channel ID of the owner's last message — used as default for proactive sends
    owner_channel_id: Mutex<Option<String>>,
    /// Maps session_id → channel_id for approval routing
    session_channels: Mutex<HashMap<Uuid, String>>,
    /// Pending approval channels: approval_id → oneshot sender of (approved, always)
    pending_approvals: Mutex<HashMap<String, oneshot::Sender<(bool, bool)>>>,
    /// Allowed user IDs — hot-reloadable at runtime when config changes
    allowed_users: Mutex<HashSet<String>>,
}

impl Default for SlackState {
    fn default() -> Self {
        Self::new()
    }
}

impl SlackState {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
            bot_token: Mutex::new(None),
            owner_channel_id: Mutex::new(None),
            session_channels: Mutex::new(HashMap::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            allowed_users: Mutex::new(HashSet::new()),
        }
    }

    /// Replace the allowed users set (called on config reload).
    pub async fn update_allowed_users(&self, users: Vec<String>) {
        *self.allowed_users.lock().await = users.into_iter().collect();
    }

    /// Check if a user ID is in the allowed set.
    pub async fn is_user_allowed(&self, user_id: &str) -> bool {
        let set = self.allowed_users.lock().await;
        set.is_empty() || set.contains(user_id)
    }

    /// Store the connected client, bot token, and optionally the owner's channel.
    pub async fn set_connected(
        &self,
        client: Arc<SlackHyperClient>,
        bot_token: String,
        channel_id: Option<String>,
    ) {
        *self.client.lock().await = Some(client);
        *self.bot_token.lock().await = Some(bot_token);
        if let Some(id) = channel_id {
            *self.owner_channel_id.lock().await = Some(id);
        }
    }

    /// Update the owner's channel ID (called on each owner message).
    pub async fn set_owner_channel(&self, channel_id: String) {
        *self.owner_channel_id.lock().await = Some(channel_id);
    }

    /// Get a clone of the connected client, if any.
    pub async fn client(&self) -> Option<Arc<SlackHyperClient>> {
        self.client.lock().await.clone()
    }

    /// Get the bot token for opening API sessions.
    pub async fn bot_token(&self) -> Option<String> {
        self.bot_token.lock().await.clone()
    }

    /// Get the owner's last channel ID for proactive messaging.
    pub async fn owner_channel_id(&self) -> Option<String> {
        self.owner_channel_id.lock().await.clone()
    }

    /// Check if Slack is currently connected.
    pub async fn is_connected(&self) -> bool {
        self.client.lock().await.is_some()
    }

    /// Record which channel_id corresponds to a given session.
    pub async fn register_session_channel(&self, session_id: Uuid, channel_id: String) {
        self.session_channels
            .lock()
            .await
            .insert(session_id, channel_id);
    }

    /// Look up the channel_id for a session.
    pub async fn session_channel(&self, session_id: Uuid) -> Option<String> {
        self.session_channels.lock().await.get(&session_id).cloned()
    }

    /// Register a pending approval oneshot channel.
    pub async fn register_pending_approval(&self, id: String, tx: oneshot::Sender<(bool, bool)>) {
        self.pending_approvals.lock().await.insert(id, tx);
    }

    /// Resolve a pending approval. Returns true if one existed.
    pub async fn resolve_pending_approval(&self, id: &str, approved: bool, always: bool) -> bool {
        if let Some(tx) = self.pending_approvals.lock().await.remove(id) {
            let _ = tx.send((approved, always));
            true
        } else {
            false
        }
    }
}
