//! Discord Integration
//!
//! Runs a Discord bot alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;

pub use agent::DiscordAgent;

use std::sync::Arc;
use tokio::sync::Mutex;

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
}
