//! Telegram Bot Integration
//!
//! Runs a Telegram bot alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;

pub use agent::TelegramAgent;

use crate::brain::agent::{ApprovalCallback, ToolApprovalInfo};
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::Bot;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup};
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
    /// When true, all tool calls are auto-approved for this session (user chose "Always").
    auto_approve_session: Mutex<bool>,
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
            auto_approve_session: Mutex::new(false),
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

    /// Mark the session as auto-approve (user chose "Always").
    pub async fn set_auto_approve_session(&self) {
        *self.auto_approve_session.lock().await = true;
    }

    /// Whether all tool calls should be auto-approved this session.
    pub async fn is_auto_approve_session(&self) -> bool {
        *self.auto_approve_session.lock().await
    }

    /// Build an `ApprovalCallback` that sends an inline-keyboard message to Telegram
    /// and waits (up to 5 min) for the user to tap Yes, Always, or No.
    pub fn make_approval_callback(state: Arc<TelegramState>) -> ApprovalCallback {
        Arc::new(move |info: ToolApprovalInfo| {
            let state = state.clone();
            Box::pin(async move {
                // Auto-approve if user already chose "Always" this session
                if state.is_auto_approve_session().await {
                    return Ok(true);
                }

                // Find the chat this session is active in
                let chat_id = match state.session_chat(info.session_id).await {
                    Some(id) => id,
                    None => match state.owner_chat_id().await {
                        Some(id) => id,
                        None => {
                            tracing::warn!(
                                "Telegram approval: no chat_id for session {}",
                                info.session_id
                            );
                            return Ok(false);
                        }
                    },
                };

                let bot = match state.bot().await {
                    Some(b) => b,
                    None => {
                        tracing::warn!("Telegram approval: bot not connected");
                        return Ok(false);
                    }
                };

                // Build unique approval id
                let approval_id = Uuid::new_v4().to_string();

                // Build inline keyboard — 3 buttons matching TUI: Yes / Always / No
                let keyboard = InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("✅ Yes", format!("approve:{}", approval_id)),
                    InlineKeyboardButton::callback(
                        "🔁 Always (session)",
                        format!("always:{}", approval_id),
                    ),
                    InlineKeyboardButton::callback("❌ No", format!("deny:{}", approval_id)),
                ]]);

                // Format message
                let input_pretty = serde_json::to_string_pretty(&info.tool_input)
                    .unwrap_or_else(|_| info.tool_input.to_string());
                let text = format!(
                    "🔐 <b>Tool Approval Required</b>\n\nTool: <code>{}</code>\nInput:\n<pre>{}</pre>",
                    info.tool_name,
                    html_escape_pre(&input_pretty),
                );

                use teloxide::payloads::SendMessageSetters;
                use teloxide::prelude::Requester;
                use teloxide::types::ParseMode;

                match bot
                    .send_message(ChatId(chat_id), &text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await
                {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("Telegram approval: failed to send message: {}", e);
                        return Ok(false);
                    }
                }

                // Register oneshot channel
                let (tx, rx) = oneshot::channel();
                state.register_pending_approval(approval_id, tx).await;

                // Wait up to 5 minutes
                match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                    Ok(Ok((approved, always))) => {
                        if always {
                            state.set_auto_approve_session().await;
                        }
                        Ok(approved)
                    }
                    Ok(Err(_)) => Ok(false), // channel closed
                    Err(_) => {
                        tracing::warn!("Telegram approval: 5-minute timeout — auto-denying");
                        Ok(false)
                    }
                }
            })
        })
    }
}

/// Escape HTML for use inside <pre> blocks (only & < > needed)
fn html_escape_pre(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
