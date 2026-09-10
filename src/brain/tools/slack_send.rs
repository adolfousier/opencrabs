//! Slack Send Tool
//!
//! Agent-callable tool for full Slack control: send, reply (thread), react, edit, delete,
//! pin/unpin, message history, channel info, user info, moderation, and more.
//! Always prefer this tool over http_request — credentials are handled securely.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolHints, ToolResult};
use crate::channels::slack::SlackState;
use async_trait::async_trait;
use serde_json::Value;
use slack_morphism::prelude::*;
use std::sync::Arc;

/// MIME type from the file extension, for the external upload POST. Slack
/// only uses it as a hint; unknown extensions fall back to octet-stream.
pub(crate) fn content_type_for(filename: &str) -> &'static str {
    match filename.rsplit('.').next().map(|e| e.to_ascii_lowercase()) {
        Some(ext) => match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "pdf" => "application/pdf",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "csv" => "text/csv",
            "txt" | "md" | "log" => "text/plain",
            "json" => "application/json",
            "zip" => "application/zip",
            "mp4" => "video/mp4",
            "mp3" => "audio/mpeg",
            "ogg" | "oga" => "audio/ogg",
            _ => "application/octet-stream",
        },
        None => "application/octet-stream",
    }
}

/// Tool for comprehensive Slack bot control (16 actions).
pub struct SlackSendTool {
    slack_state: Arc<SlackState>,
}

impl SlackSendTool {
    pub fn new(slack_state: Arc<SlackState>) -> Self {
        Self { slack_state }
    }
}

/// Extract a required non-empty string param, returning ToolResult::error on failure.
#[allow(clippy::result_large_err)]
fn get_str<'a>(input: &'a Value, key: &str) -> std::result::Result<&'a str, ToolResult> {
    match input.get(key).and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => Ok(s),
        _ => Err(ToolResult::error(format!(
            "Missing required parameter '{key}'."
        ))),
    }
}

/// Unwrap channel id or return error ToolResult.
#[allow(clippy::result_large_err)]
fn channel_or_err(id: Option<String>) -> std::result::Result<String, ToolResult> {
    let raw = id.ok_or_else(|| {
        ToolResult::error(
            "No channel_id provided and no owner channel available. \
             The owner must send a message first, or pass channel_id explicitly."
                .to_string(),
        )
    })?;
    if !crate::cron::send_scope::may_send("slack", &raw) {
        let reason = crate::cron::send_scope::refusal_for("slack", &raw);
        tracing::warn!("slack_send: {reason}");
        return Err(ToolResult::error(reason));
    }
    Ok(raw)
}

// Macro to early-return Ok(err_result) when a param helper returns Err.
macro_rules! pget {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => return Ok(e),
        }
    };
}

#[async_trait]
impl Tool for SlackSendTool {
    fn name(&self) -> &str {
        "slack_send"
    }

    fn description(&self) -> &str {
        "Full Slack control: send messages, reply in threads, react with emoji, edit/delete \
         messages, pin/unpin, fetch message history, list channels, get user info, list members, \
         kick users, and set channel topics. Always use slack_send instead of http_request — \
         credentials are handled securely."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "send", "reply", "react", "unreact", "edit", "delete",
                        "pin", "unpin", "get_messages", "get_channel", "list_channels",
                        "get_user", "list_members", "kick_user", "set_topic", "send_blocks",
                        "send_file"
                    ],
                    "description": "The Slack action to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Message text (send, reply, edit) or fallback text (send_blocks)"
                },
                "channel_id": {
                    "type": "string",
                    "description": "Slack channel ID (e.g. 'C12345678'). Omit to use owner's last channel."
                },
                "thread_ts": {
                    "type": "string",
                    "description": "Thread timestamp for reply (e.g. '1503435956.000247')"
                },
                "message_ts": {
                    "type": "string",
                    "description": "Message timestamp for react/unreact/edit/delete/pin/unpin"
                },
                "emoji": {
                    "type": "string",
                    "description": "Reaction emoji name without colons (e.g. 'thumbsup', 'eyes')"
                },
                "user_id": {
                    "type": "string",
                    "description": "Slack user ID for get_user, kick_user (e.g. 'U12345678')"
                },
                "topic": {
                    "type": "string",
                    "description": "Channel topic text for set_topic"
                },
                "blocks": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "Slack Block Kit blocks array for send_blocks (JSON)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of items to fetch for get_messages/list_channels/list_members (default 10)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Local file path to upload (required for send_file)"
                },
                "caption": {
                    "type": "string",
                    "description": "Optional comment/caption for send_file"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: true,
        }
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) if !a.is_empty() => a.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing required 'action' parameter.".to_string(),
                ));
            }
        };

        let client = match self.slack_state.client().await {
            Some(c) => c,
            None => {
                return Ok(ToolResult::error(
                    "Slack is not connected. The bot needs to be running with valid tokens."
                        .to_string(),
                ));
            }
        };

        let bot_token = match self.slack_state.bot_token().await {
            Some(t) => t,
            None => {
                return Ok(ToolResult::error(
                    "Slack bot token not available.".to_string(),
                ));
            }
        };

        let token = SlackApiToken::new(SlackApiTokenValue::from(bot_token));
        let session = client.open_session(&token);

        // Resolve target channel once: explicit param > ambient origin fallback > owner's last channel
        let channel_id_opt: Option<String> =
            if let Some(ch) = input.get("channel_id").and_then(|v| v.as_str()) {
                Some(ch.to_string())
            } else if let Some(origin) = context.origin_target.as_deref() {
                if origin.channel == "slack" {
                    Some(origin.chat_id.clone())
                } else {
                    self.slack_state.owner_channel_id().await
                }
            } else {
                self.slack_state.owner_channel_id().await
            };

        match action.as_str() {
            // ── send ─────────────────────────────────────────────────────────
            "send" => {
                let text = pget!(get_str(&input, "message")).to_string();
                let text = crate::utils::slack_fmt::markdown_to_mrkdwn(&text);
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let chunks = crate::channels::slack::handler::split_message(&text, 3000);
                for chunk in chunks {
                    let request = SlackApiChatPostMessageRequest::new(
                        SlackChannelId::new(channel_id.clone()),
                        SlackMessageContent::new().with_text(chunk.to_string()),
                    );
                    if let Err(e) = session.chat_post_message(&request).await {
                        return Ok(ToolResult::error(format!("Failed to send: {e}")));
                    }
                }
                Ok(ToolResult::success(format!(
                    "Message sent to channel {channel_id}."
                )))
            }

            // ── reply (in thread) ────────────────────────────────────────────
            "reply" => {
                let text = pget!(get_str(&input, "message")).to_string();
                let text = crate::utils::slack_fmt::markdown_to_mrkdwn(&text);
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let thread_ts_str = pget!(get_str(&input, "thread_ts")).to_string();
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id.clone()),
                    SlackMessageContent::new().with_text(text),
                )
                .with_thread_ts(SlackTs(thread_ts_str.clone()));
                match session.chat_post_message(&request).await {
                    Ok(resp) => Ok(ToolResult::success(format!(
                        "Reply sent in thread {} (new ts={}).",
                        thread_ts_str, resp.ts.0
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to reply: {e}"))),
                }
            }

            // ── react ────────────────────────────────────────────────────────
            "react" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_ts_str = pget!(get_str(&input, "message_ts")).to_string();
                let emoji = pget!(get_str(&input, "emoji")).to_string();
                let request = SlackApiReactionsAddRequest {
                    channel: SlackChannelId::new(channel_id),
                    name: SlackReactionName(emoji.clone()),
                    timestamp: SlackTs(message_ts_str.clone()),
                };
                match session.reactions_add(&request).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Reacted with :{emoji}: on message {message_ts_str}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to react: {e}"))),
                }
            }

            // ── unreact ──────────────────────────────────────────────────────
            "unreact" => {
                let message_ts_str = pget!(get_str(&input, "message_ts")).to_string();
                let emoji = pget!(get_str(&input, "emoji")).to_string();
                let request = SlackApiReactionsRemoveRequest {
                    name: SlackReactionName(emoji.clone()),
                    channel: channel_id_opt.map(SlackChannelId::new),
                    file: None,
                    full: None,
                    timestamp: Some(SlackTs(message_ts_str.clone())),
                };
                match session.reactions_remove(&request).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Removed reaction :{emoji}: from message {message_ts_str}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to remove reaction: {e}"))),
                }
            }

            // ── edit ─────────────────────────────────────────────────────────
            "edit" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_ts_str = pget!(get_str(&input, "message_ts")).to_string();
                let text = pget!(get_str(&input, "message")).to_string();
                let request = SlackApiChatUpdateRequest {
                    channel: SlackChannelId::new(channel_id),
                    content: SlackMessageContent::new().with_text(text),
                    ts: SlackTs(message_ts_str.clone()),
                    as_user: None,
                    link_names: None,
                    parse: None,
                    reply_broadcast: None,
                };
                match session.chat_update(&request).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message {message_ts_str} edited."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to edit: {e}"))),
                }
            }

            // ── delete ───────────────────────────────────────────────────────
            "delete" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_ts_str = pget!(get_str(&input, "message_ts")).to_string();
                let request = SlackApiChatDeleteRequest {
                    channel: SlackChannelId::new(channel_id),
                    ts: SlackTs(message_ts_str.clone()),
                    as_user: None,
                };
                match session.chat_delete(&request).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message {message_ts_str} deleted."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to delete: {e}"))),
                }
            }

            // ── pin ──────────────────────────────────────────────────────────
            "pin" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_ts_str = pget!(get_str(&input, "message_ts")).to_string();
                let request = SlackApiPinsAddRequest {
                    channel: SlackChannelId::new(channel_id),
                    timestamp: SlackTs(message_ts_str.clone()),
                };
                match session.pins_add(&request).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message {message_ts_str} pinned."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to pin: {e}"))),
                }
            }

            // ── unpin ────────────────────────────────────────────────────────
            "unpin" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_ts_str = pget!(get_str(&input, "message_ts")).to_string();
                let request = SlackApiPinsRemoveRequest {
                    channel: SlackChannelId::new(channel_id),
                    timestamp: SlackTs(message_ts_str.clone()),
                };
                match session.pins_remove(&request).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message {message_ts_str} unpinned."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to unpin: {e}"))),
                }
            }

            // ── get_messages ─────────────────────────────────────────────────
            "get_messages" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let limit = input
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.min(100) as u16)
                    .unwrap_or(10);
                let request = SlackApiConversationsHistoryRequest {
                    channel: Some(SlackChannelId::new(channel_id.clone())),
                    cursor: None,
                    latest: None,
                    limit: Some(limit),
                    oldest: None,
                    inclusive: None,
                    include_all_metadata: None,
                };
                match session.conversations_history(&request).await {
                    Ok(resp) => {
                        let summary = resp
                            .messages
                            .iter()
                            .map(|m| {
                                let user = m
                                    .sender
                                    .user
                                    .as_ref()
                                    .map(|u| u.0.as_str())
                                    .unwrap_or("bot");
                                let text = m.content.text.as_deref().unwrap_or("[no text]");
                                let preview = &text[..text.floor_char_boundary(80)];
                                format!("[{}] {}: {}", m.origin.ts.0, user, preview)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(ToolResult::success(format!(
                            "Last {} message(s) in {}:\n{}",
                            resp.messages.len(),
                            channel_id,
                            summary
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to fetch messages: {e}"))),
                }
            }

            // ── get_channel ──────────────────────────────────────────────────
            "get_channel" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let request = SlackApiConversationsInfoRequest {
                    channel: SlackChannelId::new(channel_id.clone()),
                    include_locale: None,
                    include_num_members: Some(true),
                };
                match session.conversations_info(&request).await {
                    Ok(resp) => {
                        let ch = &resp.channel;
                        let name = ch.name.as_deref().unwrap_or("(unknown)");
                        let topic = ch
                            .topic
                            .as_ref()
                            .map(|t| t.value.as_str())
                            .unwrap_or("(none)");
                        let members = ch.num_members.unwrap_or(0);
                        Ok(ToolResult::success(format!(
                            "Channel {channel_id}: name=#{name}, topic={topic}, members={members}"
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to get channel info: {e}"
                    ))),
                }
            }

            // ── list_channels ────────────────────────────────────────────────
            "list_channels" => {
                let limit = input
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.min(200) as u16)
                    .unwrap_or(50);
                let request = SlackApiConversationsListRequest {
                    cursor: None,
                    limit: Some(limit),
                    exclude_archived: Some(true),
                    types: None,
                };
                match session.conversations_list(&request).await {
                    Ok(resp) => {
                        let list = resp
                            .channels
                            .iter()
                            .map(|ch| {
                                let name = ch.name.as_deref().unwrap_or("(unknown)");
                                format!("{}: #{}", ch.id.0, name)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(ToolResult::success(format!(
                            "{} channel(s):\n{}",
                            resp.channels.len(),
                            list
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to list channels: {e}"))),
                }
            }

            // ── get_user ─────────────────────────────────────────────────────
            "get_user" => {
                let user_id_str = pget!(get_str(&input, "user_id")).to_string();
                let request = SlackApiUsersInfoRequest {
                    user: SlackUserId(user_id_str.clone()),
                    include_locale: None,
                };
                match session.users_info(&request).await {
                    Ok(resp) => {
                        let u = &resp.user;
                        let name = u
                            .real_name
                            .as_deref()
                            .or(u.name.as_deref())
                            .unwrap_or("(unknown)");
                        let deleted = u.deleted.unwrap_or(false);
                        Ok(ToolResult::success(format!(
                            "User {user_id_str}: name={name}, deleted={deleted}"
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to get user info: {e}"))),
                }
            }

            // ── list_members ─────────────────────────────────────────────────
            "list_members" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let limit = input
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.min(200) as u16)
                    .unwrap_or(50);
                let request = SlackApiConversationsMembersRequest {
                    channel: Some(SlackChannelId::new(channel_id.clone())),
                    cursor: None,
                    limit: Some(limit),
                };
                match session.conversations_members(&request).await {
                    Ok(resp) => {
                        let list = resp
                            .members
                            .iter()
                            .map(|uid| uid.0.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        Ok(ToolResult::success(format!(
                            "{} member(s) in {channel_id}: {list}",
                            resp.members.len()
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to list members: {e}"))),
                }
            }

            // ── kick_user ────────────────────────────────────────────────────
            "kick_user" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let user_id_str = pget!(get_str(&input, "user_id")).to_string();
                let request = SlackApiConversationsKickRequest {
                    channel: SlackChannelId::new(channel_id),
                    user: SlackUserId(user_id_str.clone()),
                };
                match session.conversations_kick(&request).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "User {user_id_str} removed from channel."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to kick user: {e}"))),
                }
            }

            // ── set_topic ────────────────────────────────────────────────────
            "set_topic" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let topic = pget!(get_str(&input, "topic")).to_string();
                let request = SlackApiConversationsSetTopicRequest {
                    channel: SlackChannelId::new(channel_id.clone()),
                    topic: topic.clone(),
                };
                match session.conversations_set_topic(&request).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Topic set to '{topic}' in channel {channel_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to set topic: {e}"))),
                }
            }

            // ── send_blocks ──────────────────────────────────────────────────
            "send_blocks" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let blocks_value = match input.get("blocks") {
                    Some(b) => b.clone(),
                    None => {
                        return Ok(ToolResult::error(
                            "Missing required parameter 'blocks'.".to_string(),
                        ));
                    }
                };
                let blocks: Vec<SlackBlock> = match serde_json::from_value(blocks_value) {
                    Ok(b) => b,
                    Err(e) => {
                        return Ok(ToolResult::error(format!("Invalid blocks JSON: {e}")));
                    }
                };
                let fallback = input
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(blocks message)")
                    .to_string();
                let content = SlackMessageContent::new()
                    .with_text(fallback)
                    .with_blocks(blocks);
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id.clone()),
                    content,
                );
                match session.chat_post_message(&request).await {
                    Ok(resp) => Ok(ToolResult::success(format!(
                        "Blocks message sent to {channel_id} (ts={}).",
                        resp.ts.0
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send blocks: {e}"))),
                }
            }

            "send_file" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let file_path = match input.get("file_path").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => {
                        return Ok(ToolResult::error(
                            "send_file requires 'file_path'.".to_string(),
                        ));
                    }
                };
                let caption = input
                    .get("caption")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let expanded = crate::brain::tools::error::expand_tilde(&file_path);
                match tokio::fs::read(&expanded).await {
                    Ok(bytes) => {
                        let fname = expanded
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("file.bin")
                            .to_string();
                        // Slack removed files.upload; the supported flow is
                        // getUploadURLExternal -> POST bytes -> complete.
                        let get_url = SlackApiFilesGetUploadUrlExternalRequest {
                            filename: fname.clone(),
                            length: bytes.len(),
                            alt_txt: None,
                            snippet_type: None,
                        };
                        let ticket = match session.get_upload_url_external(&get_url).await {
                            Ok(t) => t,
                            Err(e) => {
                                return Ok(ToolResult::error(format!(
                                    "Failed to get Slack upload URL: {e}"
                                )));
                            }
                        };
                        let upload = SlackApiFilesUploadViaUrlRequest {
                            upload_url: ticket.upload_url,
                            content: bytes,
                            content_type: content_type_for(&fname).to_string(),
                        };
                        if let Err(e) = session.files_upload_via_url(&upload).await {
                            return Ok(ToolResult::error(format!(
                                "Failed to upload file bytes to Slack: {e}"
                            )));
                        }
                        let complete = SlackApiFilesCompleteUploadExternalRequest {
                            files: vec![SlackApiFilesComplete {
                                id: ticket.file_id,
                                title: Some(fname.clone()),
                            }],
                            channel_id: Some(SlackChannelId::new(channel_id)),
                            initial_comment: caption,
                            thread_ts: None,
                        };
                        match session.files_complete_upload_external(&complete).await {
                            Ok(_) => Ok(ToolResult::success(format!(
                                "File {fname} uploaded to Slack."
                            ))),
                            Err(e) => Ok(ToolResult::error(format!(
                                "Failed to finalize Slack upload: {e}"
                            ))),
                        }
                    }
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to read file '{}': {e}",
                        file_path
                    ))),
                }
            }

            unknown => Ok(ToolResult::error(format!(
                "Unknown action '{unknown}'. Valid: send, reply, react, unreact, edit, delete, \
                 pin, unpin, get_messages, get_channel, list_channels, get_user, list_members, \
                 kick_user, set_topic, send_blocks, send_file"
            ))),
        }
    }
}
