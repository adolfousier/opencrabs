//! Telegram Bot Integration
//!
//! Runs a Telegram bot alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;

pub use agent::TelegramAgent;

use std::collections::{HashMap, HashSet};
use teloxide::prelude::Bot;
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

/// Shared Telegram state for proactive messaging.
///
/// Set when the bot connects (agent stores Bot) and when the owner
/// sends their first message (handler stores chat_id).
/// Read by the `telegram_send` tool to send messages on demand.
pub struct TelegramState {
    bot: Mutex<Option<Bot>>,
    /// Chat ID of the owner's conversation — used as default for proactive sends
    owner_chat_id: Mutex<Option<i64>>,
    /// Bot's @username — set at startup via get_me(), used for @mention detection in groups
    bot_username: Mutex<Option<String>>,
    /// Maps session_id → Telegram chat_id for approval routing
    session_chats: Mutex<HashMap<Uuid, i64>>,
    /// Pending approval channels: approval_id → oneshot sender of (approved, always).
    pending_approvals: Mutex<HashMap<String, oneshot::Sender<(bool, bool)>>>,
    /// Allowed user IDs — hot-reloadable at runtime when config changes
    allowed_users: Mutex<HashSet<i64>>,
}

impl Default for TelegramState {
    fn default() -> Self {
        Self::new()
    }
}

impl TelegramState {
    pub fn new() -> Self {
        Self {
            bot: Mutex::new(None),
            owner_chat_id: Mutex::new(None),
            bot_username: Mutex::new(None),
            session_chats: Mutex::new(HashMap::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            allowed_users: Mutex::new(HashSet::new()),
        }
    }

    /// Store the connected Bot instance.
    pub async fn set_bot(&self, bot: Bot) {
        *self.bot.lock().await = Some(bot);
    }

    /// Update the owner's chat ID (called on each owner message).
    pub async fn set_owner_chat_id(&self, chat_id: i64) {
        *self.owner_chat_id.lock().await = Some(chat_id);
    }

    /// Get a clone of the Bot, if connected.
    pub async fn bot(&self) -> Option<Bot> {
        self.bot.lock().await.clone()
    }

    /// Get the owner's chat ID for proactive messaging.
    pub async fn owner_chat_id(&self) -> Option<i64> {
        *self.owner_chat_id.lock().await
    }

    /// Store the bot's @username (set at startup via get_me).
    pub async fn set_bot_username(&self, username: String) {
        *self.bot_username.lock().await = Some(username);
    }

    /// Get the bot's @username for mention detection.
    pub async fn bot_username(&self) -> Option<String> {
        self.bot_username.lock().await.clone()
    }

    /// Check if Telegram is currently connected.
    pub async fn is_connected(&self) -> bool {
        self.bot.lock().await.is_some()
    }

    /// Record which chat_id corresponds to a given session (for approval routing).
    pub async fn register_session_chat(&self, session_id: Uuid, chat_id: i64) {
        self.session_chats.lock().await.insert(session_id, chat_id);
    }

    /// Look up the chat_id for a given session_id.
    pub async fn session_chat(&self, session_id: Uuid) -> Option<i64> {
        self.session_chats.lock().await.get(&session_id).copied()
    }

    /// Register a pending approval channel by id.
    pub async fn register_pending_approval(&self, id: String, tx: oneshot::Sender<(bool, bool)>) {
        self.pending_approvals.lock().await.insert(id, tx);
    }

    /// Resolve a pending approval.
    /// `approved` — whether tool is allowed; `always` — auto-approve all future tools.
    /// Returns true if a pending approval existed.
    pub async fn resolve_pending_approval(&self, id: &str, approved: bool, always: bool) -> bool {
        if let Some(tx) = self.pending_approvals.lock().await.remove(id) {
            let _ = tx.send((approved, always));
            true
        } else {
            false
        }
    }

    /// Replace the allowed users set (called on config reload).
    pub async fn update_allowed_users(&self, users: Vec<i64>) {
        let new_set: HashSet<i64> = users.into_iter().collect();
        let mut allowed = self.allowed_users.lock().await;
        if *allowed != new_set {
            tracing::info!(
                "Telegram: allowed users updated: {:?} -> {:?}",
                *allowed,
                new_set
            );
            *allowed = new_set;
        }
    }

    /// Check if a user ID is in the allowed set.
    pub async fn is_user_allowed(&self, user_id: i64) -> bool {
        self.allowed_users.lock().await.contains(&user_id)
    }

    /// Get the number of allowed users.
    pub async fn allowed_user_count(&self) -> usize {
        self.allowed_users.lock().await.len()
    }
}
