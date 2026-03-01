//! Discord Send Tool
//!
//! Agent-callable tool for full Discord control: send, reply, react, edit, delete,
//! pin/unpin, threads, embeds, message history, channel listing, and moderation.
//! Always prefer this tool over http_request — credentials are handled securely.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::channels::discord::DiscordState;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool for comprehensive Discord bot control (16 actions).
pub struct DiscordSendTool {
    discord_state: Arc<DiscordState>,
}

impl DiscordSendTool {
    pub fn new(discord_state: Arc<DiscordState>) -> Self {
        Self { discord_state }
    }
}

/// Extract a required non-empty string param, returning ToolResult::error on failure.
fn get_str<'a>(input: &'a Value, key: &str) -> std::result::Result<&'a str, ToolResult> {
    match input.get(key).and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => Ok(s),
        _ => Err(ToolResult::error(format!(
            "Missing required parameter '{key}'."
        ))),
    }
}

/// Parse a required numeric-string param as u64.
fn get_id(input: &Value, key: &str) -> std::result::Result<u64, ToolResult> {
    match input.get(key).and_then(|v| v.as_str()) {
        Some(s) => s.parse::<u64>().map_err(|_| {
            ToolResult::error(format!("Invalid {key} '{s}': must be a numeric string."))
        }),
        None => Err(ToolResult::error(format!(
            "Missing required parameter '{key}'."
        ))),
    }
}

/// Unwrap channel id or return error ToolResult.
fn channel_or_err(id: Option<u64>) -> std::result::Result<u64, ToolResult> {
    id.ok_or_else(|| {
        ToolResult::error(
            "No channel_id provided and no owner channel available. \
             The owner must send a message first, or pass channel_id explicitly."
                .to_string(),
        )
    })
}

/// Unwrap guild id or return error ToolResult.
fn guild_or_err(id: Option<u64>) -> std::result::Result<u64, ToolResult> {
    id.ok_or_else(|| {
        ToolResult::error(
            "No guild ID available. The bot must receive at least one guild message first."
                .to_string(),
        )
    })
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
impl Tool for DiscordSendTool {
    fn name(&self) -> &str {
        "discord_send"
    }

    fn description(&self) -> &str {
        "Full Discord control: send messages, reply, react, edit, delete, pin/unpin, create \
         threads, send embeds, fetch message history, list channels, manage roles, kick and ban \
         members. Always use discord_send instead of http_request — credentials handled securely."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "send", "reply", "react", "unreact", "edit", "delete",
                        "pin", "unpin", "create_thread", "send_embed", "get_messages",
                        "list_channels", "add_role", "remove_role", "kick", "ban"
                    ],
                    "description": "The Discord action to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Message text (send, reply, edit) or embed description (send_embed)"
                },
                "channel_id": {
                    "type": "string",
                    "description": "Discord channel ID (numeric string). Omit to use owner's last channel."
                },
                "message_id": {
                    "type": "string",
                    "description": "Target message ID for reply/react/unreact/edit/delete/pin/unpin/create_thread"
                },
                "emoji": {
                    "type": "string",
                    "description": "Unicode emoji for react/unreact (e.g. \"👍\")"
                },
                "embed_title": {
                    "type": "string",
                    "description": "Title for send_embed"
                },
                "embed_description": {
                    "type": "string",
                    "description": "Body text for send_embed"
                },
                "embed_color": {
                    "type": "integer",
                    "description": "RGB color integer for send_embed (e.g. 0x00FF00 = 65280)"
                },
                "thread_name": {
                    "type": "string",
                    "description": "Thread name for create_thread"
                },
                "user_id": {
                    "type": "string",
                    "description": "Target user ID (numeric string) for add_role/remove_role/kick/ban"
                },
                "role_id": {
                    "type": "string",
                    "description": "Role ID (numeric string) for add_role/remove_role"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of messages to fetch for get_messages (1-100, default 10)"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) if !a.is_empty() => a.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing required 'action' parameter.".to_string(),
                ));
            }
        };

        let http = match self.discord_state.http().await {
            Some(h) => h,
            None => {
                return Ok(ToolResult::error(
                    "Discord is not connected. Run discord_connect first.".to_string(),
                ));
            }
        };

        // Resolve target channel (owner's last channel if not specified)
        let channel_id_opt = if let Some(id_str) = input.get("channel_id").and_then(|v| v.as_str())
        {
            match id_str.parse::<u64>() {
                Ok(id) => Some(id),
                Err(_) => {
                    return Ok(ToolResult::error(format!(
                        "Invalid channel_id '{id_str}': must be a numeric string"
                    )));
                }
            }
        } else {
            self.discord_state.owner_channel_id().await
        };

        let guild_id_opt = self.discord_state.guild_id().await;

        use serenity::model::id::{ChannelId, GuildId, MessageId, RoleId, UserId};

        match action.as_str() {
            // ── send ─────────────────────────────────────────────────────────
            "send" => {
                let text = pget!(get_str(&input, "message")).to_string();
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let channel = ChannelId::new(channel_id);
                let chunks = crate::channels::discord::handler::split_message(&text, 2000);
                for chunk in chunks {
                    if let Err(e) = channel.say(&http, chunk).await {
                        return Ok(ToolResult::error(format!("Failed to send: {e}")));
                    }
                }
                Ok(ToolResult::success(format!(
                    "Message sent to channel {channel_id}."
                )))
            }

            // ── reply ────────────────────────────────────────────────────────
            "reply" => {
                use serenity::builder::CreateMessage;
                use serenity::model::channel::MessageReference;
                let text = pget!(get_str(&input, "message")).to_string();
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_id = pget!(get_id(&input, "message_id"));
                let channel = ChannelId::new(channel_id);
                let reference = MessageReference::from((channel, MessageId::new(message_id)));
                let builder = CreateMessage::new()
                    .content(text.as_str())
                    .reference_message(reference);
                match channel.send_message(&http, builder).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Reply sent to message {message_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to reply: {e}"))),
                }
            }

            // ── react ────────────────────────────────────────────────────────
            "react" => {
                use serenity::model::channel::ReactionType;
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_id = pget!(get_id(&input, "message_id"));
                let emoji = pget!(get_str(&input, "emoji")).to_string();
                let reaction = ReactionType::Unicode(emoji.clone());
                match http
                    .create_reaction(
                        ChannelId::new(channel_id),
                        MessageId::new(message_id),
                        &reaction,
                    )
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Reacted with {emoji} on message {message_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to react: {e}"))),
                }
            }

            // ── unreact ──────────────────────────────────────────────────────
            "unreact" => {
                use serenity::model::channel::ReactionType;
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_id = pget!(get_id(&input, "message_id"));
                let emoji = pget!(get_str(&input, "emoji")).to_string();
                let reaction = ReactionType::Unicode(emoji.clone());
                match http
                    .delete_reaction_me(
                        ChannelId::new(channel_id),
                        MessageId::new(message_id),
                        &reaction,
                    )
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Removed reaction {emoji} from message {message_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to remove reaction: {e}"))),
                }
            }

            // ── edit ─────────────────────────────────────────────────────────
            "edit" => {
                use serenity::builder::EditMessage;
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_id = pget!(get_id(&input, "message_id"));
                let text = pget!(get_str(&input, "message")).to_string();
                let edit = EditMessage::new().content(text.as_str());
                match http
                    .edit_message(
                        ChannelId::new(channel_id),
                        MessageId::new(message_id),
                        &edit,
                        vec![],
                    )
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!("Message {message_id} edited."))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to edit message: {e}"))),
                }
            }

            // ── delete ───────────────────────────────────────────────────────
            "delete" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_id = pget!(get_id(&input, "message_id"));
                match http
                    .delete_message(ChannelId::new(channel_id), MessageId::new(message_id), None)
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Message {message_id} deleted."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to delete message: {e}"))),
                }
            }

            // ── pin ──────────────────────────────────────────────────────────
            "pin" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_id = pget!(get_id(&input, "message_id"));
                match http
                    .pin_message(ChannelId::new(channel_id), MessageId::new(message_id), None)
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!("Message {message_id} pinned."))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to pin message: {e}"))),
                }
            }

            // ── unpin ────────────────────────────────────────────────────────
            "unpin" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_id = pget!(get_id(&input, "message_id"));
                match http
                    .unpin_message(ChannelId::new(channel_id), MessageId::new(message_id), None)
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Message {message_id} unpinned."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to unpin message: {e}"))),
                }
            }

            // ── create_thread ────────────────────────────────────────────────
            "create_thread" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let message_id = pget!(get_id(&input, "message_id"));
                let thread_name = pget!(get_str(&input, "thread_name")).to_string();
                let body = serde_json::json!({ "name": thread_name });
                match http
                    .create_thread_from_message(
                        ChannelId::new(channel_id),
                        MessageId::new(message_id),
                        &body,
                        None,
                    )
                    .await
                {
                    Ok(ch) => Ok(ToolResult::success(format!(
                        "Thread '{}' created (id={}).",
                        ch.name, ch.id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to create thread: {e}"))),
                }
            }

            // ── send_embed ───────────────────────────────────────────────────
            "send_embed" => {
                use serenity::builder::{CreateEmbed, CreateMessage};
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let title = input
                    .get("embed_title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let description = input
                    .get("embed_description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let color = input
                    .get("embed_color")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0x5865F2) as u32; // Discord blurple default
                let embed = CreateEmbed::new()
                    .title(title.as_str())
                    .description(description.as_str())
                    .color(color);
                let builder = CreateMessage::new().embed(embed);
                match ChannelId::new(channel_id)
                    .send_message(&http, builder)
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Embed sent to channel {channel_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send embed: {e}"))),
                }
            }

            // ── get_messages ─────────────────────────────────────────────────
            "get_messages" => {
                let channel_id = pget!(channel_or_err(channel_id_opt));
                let limit = input
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.min(100) as u8)
                    .unwrap_or(10);
                match http
                    .get_messages(ChannelId::new(channel_id), None, Some(limit))
                    .await
                {
                    Ok(messages) => {
                        let summary = messages
                            .iter()
                            .map(|m| {
                                format!(
                                    "[{}] {}: {}",
                                    m.id,
                                    m.author.name,
                                    &m.content[..m.content.len().min(80)]
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(ToolResult::success(format!(
                            "Last {} messages in channel {channel_id}:\n{summary}",
                            messages.len()
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to fetch messages: {e}"))),
                }
            }

            // ── list_channels ────────────────────────────────────────────────
            "list_channels" => {
                let gid = pget!(guild_or_err(guild_id_opt));
                match http.get_channels(GuildId::new(gid)).await {
                    Ok(channels) => {
                        let list = channels
                            .iter()
                            .map(|c| format!("{}: {} ({})", c.id, c.name, c.kind.name()))
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(ToolResult::success(format!(
                            "Channels in guild {gid}:\n{list}"
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to list channels: {e}"))),
                }
            }

            // ── add_role ─────────────────────────────────────────────────────
            "add_role" => {
                let gid = pget!(guild_or_err(guild_id_opt));
                let user_id = pget!(get_id(&input, "user_id"));
                let role_id = pget!(get_id(&input, "role_id"));
                match http
                    .add_member_role(
                        GuildId::new(gid),
                        UserId::new(user_id),
                        RoleId::new(role_id),
                        None,
                    )
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Role {role_id} added to user {user_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to add role: {e}"))),
                }
            }

            // ── remove_role ──────────────────────────────────────────────────
            "remove_role" => {
                let gid = pget!(guild_or_err(guild_id_opt));
                let user_id = pget!(get_id(&input, "user_id"));
                let role_id = pget!(get_id(&input, "role_id"));
                match http
                    .remove_member_role(
                        GuildId::new(gid),
                        UserId::new(user_id),
                        RoleId::new(role_id),
                        None,
                    )
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Role {role_id} removed from user {user_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to remove role: {e}"))),
                }
            }

            // ── kick ─────────────────────────────────────────────────────────
            "kick" => {
                let gid = pget!(guild_or_err(guild_id_opt));
                let user_id = pget!(get_id(&input, "user_id"));
                match http
                    .kick_member(GuildId::new(gid), UserId::new(user_id), None)
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!("User {user_id} kicked."))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to kick user: {e}"))),
                }
            }

            // ── ban ──────────────────────────────────────────────────────────
            "ban" => {
                let gid = pget!(guild_or_err(guild_id_opt));
                let user_id = pget!(get_id(&input, "user_id"));
                match http
                    .ban_user(GuildId::new(gid), UserId::new(user_id), 0, None)
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!("User {user_id} banned."))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to ban user: {e}"))),
                }
            }

            unknown => Ok(ToolResult::error(format!(
                "Unknown action '{unknown}'. Valid: send, reply, react, unreact, edit, delete, \
                 pin, unpin, create_thread, send_embed, get_messages, list_channels, \
                 add_role, remove_role, kick, ban"
            ))),
        }
    }
}
