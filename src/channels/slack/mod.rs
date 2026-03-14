//! Slack Integration
//!
//! Runs a Slack bot via Socket Mode alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;

pub use agent::SlackAgent;

use slack_morphism::prelude::SlackHyperClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;
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
    /// Per-session cancel tokens for aborting in-flight agent tasks via /stop
    cancel_tokens: Mutex<HashMap<Uuid, CancellationToken>>,
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
            cancel_tokens: Mutex::new(HashMap::new()),
        }
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

    /// Store a cancel token for a session (before starting agent call).
    /// If a token already exists for this session, cancel it first to abort the
    /// previous in-flight agent call — prevents concurrent uncancellable agents.
    pub async fn store_cancel_token(&self, session_id: Uuid, token: CancellationToken) {
        let mut tokens = self.cancel_tokens.lock().await;
        if let Some(old) = tokens.remove(&session_id) {
            tracing::warn!(
                "Slack: cancelling previous in-flight agent call for session {}",
                session_id
            );
            old.cancel();
        }
        tokens.insert(session_id, token);
    }

    /// Cancel and remove the token for a session. Returns true if a token existed.
    pub async fn cancel_session(&self, session_id: Uuid) -> bool {
        if let Some(token) = self.cancel_tokens.lock().await.remove(&session_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Remove the cancel token after the agent call completes (cleanup).
    /// Only removes if the stored token is already cancelled — prevents a
    /// finishing old call from removing a newer call's live token.
    pub async fn remove_cancel_token(&self, session_id: Uuid) {
        let mut tokens = self.cancel_tokens.lock().await;
        if let Some(token) = tokens.get(&session_id)
            && token.is_cancelled()
        {
            tokens.remove(&session_id);
        }
    }
}
