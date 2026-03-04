//! Discord Integration
//!
//! Runs a Discord bot alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;

pub use agent::DiscordAgent;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Shared Discord state for proactive messaging.
///
/// Set when the bot connects via the `ready` event.
/// Read by the `discord_send` tool to send messages on demand.
pub struct DiscordState {
    http: Mutex<Option<Arc<serenity::http::Http>>>,
    /// Channel ID of the owner's last message — used as default for proactive sends
    owner_channel_id: Mutex<Option<u64>>,
    /// Bot's own user ID — set on ready, used for @mention detection
    bot_user_id: Mutex<Option<u64>>,
    /// Guild ID of the last guild message — needed for guild-scoped actions
    guild_id: Mutex<Option<u64>>,
    /// Maps session_id → channel_id for approval routing
    session_channels: Mutex<HashMap<Uuid, u64>>,
    /// Pending approval channels: approval_id → oneshot sender of (approved, always)
    pending_approvals: Mutex<HashMap<String, oneshot::Sender<(bool, bool)>>>,
    /// Per-session cancel tokens for aborting in-flight agent tasks via /stop
    cancel_tokens: Mutex<HashMap<Uuid, CancellationToken>>,
}

impl Default for DiscordState {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordState {
    pub fn new() -> Self {
        Self {
            http: Mutex::new(None),
            owner_channel_id: Mutex::new(None),
            bot_user_id: Mutex::new(None),
            guild_id: Mutex::new(None),
            session_channels: Mutex::new(HashMap::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            cancel_tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Store the connected HTTP client and optionally set the owner channel.
    pub async fn set_connected(&self, http: Arc<serenity::http::Http>, channel_id: Option<u64>) {
        *self.http.lock().await = Some(http);
        if let Some(id) = channel_id {
            *self.owner_channel_id.lock().await = Some(id);
        }
    }

    /// Update the owner's channel ID (called on each owner message).
    pub async fn set_owner_channel(&self, channel_id: u64) {
        *self.owner_channel_id.lock().await = Some(channel_id);
    }

    /// Get a clone of the HTTP client, if connected.
    pub async fn http(&self) -> Option<Arc<serenity::http::Http>> {
        self.http.lock().await.clone()
    }

    /// Get the owner's last channel ID for proactive messaging.
    pub async fn owner_channel_id(&self) -> Option<u64> {
        *self.owner_channel_id.lock().await
    }

    /// Store the bot's own user ID (set from ready event).
    pub async fn set_bot_user_id(&self, id: u64) {
        *self.bot_user_id.lock().await = Some(id);
    }

    /// Get the bot's user ID for @mention detection.
    pub async fn bot_user_id(&self) -> Option<u64> {
        *self.bot_user_id.lock().await
    }

    /// Store the guild ID from an incoming guild message.
    pub async fn set_guild_id(&self, id: u64) {
        *self.guild_id.lock().await = Some(id);
    }

    /// Get the last-seen guild ID for guild-scoped actions.
    pub async fn guild_id(&self) -> Option<u64> {
        *self.guild_id.lock().await
    }

    /// Check if Discord is currently connected.
    pub async fn is_connected(&self) -> bool {
        self.http.lock().await.is_some()
    }

    /// Record which channel_id corresponds to a given session.
    pub async fn register_session_channel(&self, session_id: Uuid, channel_id: u64) {
        self.session_channels
            .lock()
            .await
            .insert(session_id, channel_id);
    }

    /// Look up the channel_id for a session.
    pub async fn session_channel(&self, session_id: Uuid) -> Option<u64> {
        self.session_channels.lock().await.get(&session_id).copied()
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
    pub async fn store_cancel_token(&self, session_id: Uuid, token: CancellationToken) {
        self.cancel_tokens.lock().await.insert(session_id, token);
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
    pub async fn remove_cancel_token(&self, session_id: Uuid) {
        self.cancel_tokens.lock().await.remove(&session_id);
    }
}
