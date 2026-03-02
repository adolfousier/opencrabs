//! Discord Integration
//!
//! Runs a Discord bot alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;

pub use agent::DiscordAgent;

use crate::brain::agent::{ApprovalCallback, ToolApprovalInfo};
use serenity::builder::{CreateActionRow, CreateButton, CreateMessage, EditMessage};
use serenity::model::application::ButtonStyle;
use serenity::model::id::ChannelId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
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
    /// When true, all tool calls are auto-approved for this session (user chose "Always")
    auto_approve_session: Mutex<bool>,
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
            auto_approve_session: Mutex::new(false),
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

    /// Mark the session as auto-approve (user chose "Always").
    pub async fn set_auto_approve_session(&self) {
        *self.auto_approve_session.lock().await = true;
    }

    /// Whether all tool calls should be auto-approved this session.
    pub async fn is_auto_approve_session(&self) -> bool {
        *self.auto_approve_session.lock().await
    }

    /// Build an `ApprovalCallback` that sends a Discord message with 3 buttons
    /// (✅ Yes / 🔁 Always (session) / ❌ No) and waits up to 5 min for a click.
    pub fn make_approval_callback(state: Arc<DiscordState>) -> ApprovalCallback {
        Arc::new(move |info: ToolApprovalInfo| {
            let state = state.clone();
            Box::pin(async move {
                if state.is_auto_approve_session().await {
                    return Ok(true);
                }

                let http = match state.http().await {
                    Some(h) => h,
                    None => {
                        tracing::warn!("Discord approval: bot not connected");
                        return Ok(false);
                    }
                };

                let channel_id = match state.session_channel(info.session_id).await {
                    Some(id) => id,
                    None => match state.owner_channel_id().await {
                        Some(id) => id,
                        None => {
                            tracing::warn!(
                                "Discord approval: no channel_id for session {}",
                                info.session_id
                            );
                            return Ok(false);
                        }
                    },
                };

                let approval_id = Uuid::new_v4().to_string();
                let safe_input = crate::utils::redact_tool_input(&info.tool_input);
                let input_pretty = serde_json::to_string_pretty(&safe_input)
                    .unwrap_or_else(|_| safe_input.to_string());
                let text = format!(
                    "🔐 **Tool Approval Required**\n\nTool: `{}`\nInput:\n```json\n{}\n```",
                    info.tool_name,
                    &input_pretty[..input_pretty.len().min(1800)],
                );

                let row = CreateActionRow::Buttons(vec![
                    CreateButton::new(format!("approve:{}", approval_id))
                        .label("✅ Yes")
                        .style(ButtonStyle::Success),
                    CreateButton::new(format!("always:{}", approval_id))
                        .label("🔁 Always (session)")
                        .style(ButtonStyle::Primary),
                    CreateButton::new(format!("deny:{}", approval_id))
                        .label("❌ No")
                        .style(ButtonStyle::Danger),
                ]);

                let mut sent_msg = match ChannelId::new(channel_id)
                    .send_message(
                        &http,
                        CreateMessage::new().content(&text).components(vec![row]),
                    )
                    .await
                {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!("Discord approval: failed to send message: {}", e);
                        return Ok(false);
                    }
                };

                let (tx, rx) = oneshot::channel();
                state.register_pending_approval(approval_id, tx).await;

                match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                    Ok(Ok((approved, always))) => {
                        if always {
                            state.set_auto_approve_session().await;
                        }
                        // Remove buttons from the message
                        let label = if always {
                            "🔁 Always approved (session)"
                        } else if approved {
                            "✅ Approved"
                        } else {
                            "❌ Denied"
                        };
                        let _ = sent_msg
                            .edit(&http, EditMessage::new().content(label).components(vec![]))
                            .await;
                        Ok(approved)
                    }
                    Ok(Err(_)) => Ok(false),
                    Err(_) => {
                        tracing::warn!("Discord approval: 5-minute timeout — auto-denying");
                        let _ = sent_msg
                            .edit(
                                &http,
                                EditMessage::new()
                                    .content("⏱️ Approval timed out — denied")
                                    .components(vec![]),
                            )
                            .await;
                        Ok(false)
                    }
                }
            })
        })
    }
}
