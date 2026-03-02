//! Slack Integration
//!
//! Runs a Slack bot via Socket Mode alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;

pub use agent::SlackAgent;

use crate::brain::agent::{ApprovalCallback, ToolApprovalInfo};
use slack_morphism::prelude::*;
use std::collections::HashMap;
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
    /// When true, all tool calls are auto-approved for this session (user chose "Always")
    auto_approve_session: Mutex<bool>,
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
            auto_approve_session: Mutex::new(false),
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

    /// Mark the session as auto-approve (user chose "Always").
    pub async fn set_auto_approve_session(&self) {
        *self.auto_approve_session.lock().await = true;
    }

    /// Whether all tool calls should be auto-approved this session.
    pub async fn is_auto_approve_session(&self) -> bool {
        *self.auto_approve_session.lock().await
    }

    /// Build an `ApprovalCallback` that sends a Slack Block Kit message with 3 buttons
    /// (✅ Yes / 🔁 Always (session) / ❌ No) and waits up to 5 min for a click.
    pub fn make_approval_callback(state: Arc<SlackState>) -> ApprovalCallback {
        Arc::new(move |info: ToolApprovalInfo| {
            let state = state.clone();
            Box::pin(async move {
                if state.is_auto_approve_session().await {
                    return Ok(true);
                }

                let client = match state.client().await {
                    Some(c) => c,
                    None => {
                        tracing::warn!("Slack approval: bot not connected");
                        return Ok(false);
                    }
                };

                let bot_token = match state.bot_token().await {
                    Some(t) => t,
                    None => {
                        tracing::warn!("Slack approval: no bot token");
                        return Ok(false);
                    }
                };

                let channel_id = match state.session_channel(info.session_id).await {
                    Some(id) => id,
                    None => match state.owner_channel_id().await {
                        Some(id) => id,
                        None => {
                            tracing::warn!(
                                "Slack approval: no channel_id for session {}",
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
                    "🔐 *Tool Approval Required*\n\nTool: `{}`\nInput:\n```\n{}\n```",
                    info.tool_name,
                    &input_pretty[..input_pretty.len().min(1800)],
                );

                // Build Block Kit blocks: text section + 3-button actions row
                let section = SlackBlock::Section(SlackSectionBlock::new().with_text(
                    SlackBlockText::MarkDown(SlackBlockMarkDownText::new(text.clone())),
                ));
                let approve_btn = SlackBlockButtonElement::new(
                    SlackActionId::new(format!("approve:{}", approval_id)),
                    SlackBlockPlainTextOnly::from(SlackBlockPlainText::new("✅ Yes".to_string())),
                )
                .with_style("primary".to_string());
                let always_btn = SlackBlockButtonElement::new(
                    SlackActionId::new(format!("always:{}", approval_id)),
                    SlackBlockPlainTextOnly::from(SlackBlockPlainText::new(
                        "🔁 Always (session)".to_string(),
                    )),
                );
                let deny_btn = SlackBlockButtonElement::new(
                    SlackActionId::new(format!("deny:{}", approval_id)),
                    SlackBlockPlainTextOnly::from(SlackBlockPlainText::new("❌ No".to_string())),
                )
                .with_style("danger".to_string());
                let actions = SlackBlock::Actions(SlackActionsBlock::new(vec![
                    SlackActionBlockElement::Button(approve_btn),
                    SlackActionBlockElement::Button(always_btn),
                    SlackActionBlockElement::Button(deny_btn),
                ]));

                let content = SlackMessageContent::new()
                    .with_text(text)
                    .with_blocks(vec![section, actions]);
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id.clone()),
                    content,
                );
                let token = SlackApiToken::new(SlackApiTokenValue::from(bot_token.clone()));
                let session = client.open_session(&token);

                let sent = match session.chat_post_message(&request).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("Slack approval: failed to send message: {}", e);
                        return Ok(false);
                    }
                };

                let msg_ts = sent.ts.clone();
                let (tx, rx) = oneshot::channel();
                state.register_pending_approval(approval_id, tx).await;

                match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                    Ok(Ok((approved, always))) => {
                        if always {
                            state.set_auto_approve_session().await;
                        }
                        // Edit the message to show the outcome (remove buttons)
                        let label = if always {
                            "🔁 Always approved (session)"
                        } else if approved {
                            "✅ Approved"
                        } else {
                            "❌ Denied"
                        };
                        let update = SlackApiChatUpdateRequest::new(
                            SlackChannelId::new(channel_id),
                            SlackMessageContent::new().with_text(label.to_string()),
                            msg_ts,
                        );
                        let _ = session.chat_update(&update).await;
                        Ok(approved)
                    }
                    Ok(Err(_)) => Ok(false),
                    Err(_) => {
                        tracing::warn!("Slack approval: 5-minute timeout — auto-denying");
                        let update = SlackApiChatUpdateRequest::new(
                            SlackChannelId::new(channel_id),
                            SlackMessageContent::new()
                                .with_text("⏱️ Approval timed out — denied".to_string()),
                            msg_ts,
                        );
                        let _ = session.chat_update(&update).await;
                        Ok(false)
                    }
                }
            })
        })
    }
}
