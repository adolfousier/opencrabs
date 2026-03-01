//! Telegram Send Tool
//!
//! Agent-callable tool for full Telegram control: send, reply, edit, delete,
//! pin/unpin, forward, media, polls, inline buttons, chat info, moderation,
//! and reactions. Always prefer this tool over http_request — credentials
//! are handled securely.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::channels::telegram::TelegramState;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::*;
use teloxide::types::{
    ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId, ReactionType,
    ReplyParameters, UserId,
};

/// Tool for comprehensive Telegram bot control (16 actions).
pub struct TelegramSendTool {
    telegram_state: Arc<TelegramState>,
}

impl TelegramSendTool {
    pub fn new(telegram_state: Arc<TelegramState>) -> Self {
        Self { telegram_state }
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

/// Parse a required integer param as i64.
fn get_id(input: &Value, key: &str) -> std::result::Result<i64, ToolResult> {
    match input.get(key).and_then(|v| v.as_i64()) {
        Some(id) => Ok(id),
        None => Err(ToolResult::error(format!(
            "Missing required parameter '{key}' (must be an integer)."
        ))),
    }
}

/// Resolve chat_id: explicit param or owner fallback.
async fn chat_or_err(input: &Value, state: &TelegramState) -> std::result::Result<i64, ToolResult> {
    if let Some(id) = input.get("chat_id").and_then(|v| v.as_i64()) {
        return Ok(id);
    }
    match state.owner_chat_id().await {
        Some(id) => Ok(id),
        None => Err(ToolResult::error(
            "No owner chat ID known yet and no 'chat_id' parameter provided. \
             The owner needs to send at least one message to the bot first, \
             or specify a chat_id."
                .to_string(),
        )),
    }
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
impl Tool for TelegramSendTool {
    fn name(&self) -> &str {
        "telegram_send"
    }

    fn description(&self) -> &str {
        "Full Telegram control: send messages, reply, edit, delete, pin/unpin, forward, \
         send photos/documents/locations/polls, inline buttons, get chat info, ban/unban \
         users, and set emoji reactions. Always use telegram_send instead of http_request \
         — credentials handled securely. Requires Telegram to be connected first."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "send", "reply", "edit", "delete", "pin", "unpin",
                        "forward", "send_photo", "send_document", "send_location",
                        "send_poll", "send_buttons", "get_chat",
                        "ban_user", "unban_user", "set_reaction"
                    ],
                    "description": "The Telegram action to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Message text (send, reply, edit, send_buttons)"
                },
                "chat_id": {
                    "type": "integer",
                    "description": "Telegram chat ID. Omit to use owner's chat."
                },
                "message_id": {
                    "type": "integer",
                    "description": "Target message ID for reply/edit/delete/pin/unpin/forward/set_reaction"
                },
                "from_chat_id": {
                    "type": "integer",
                    "description": "Source chat ID for forward action"
                },
                "photo_url": {
                    "type": "string",
                    "description": "HTTPS URL of the photo for send_photo"
                },
                "document_url": {
                    "type": "string",
                    "description": "HTTPS URL of the document for send_document"
                },
                "latitude": {
                    "type": "number",
                    "description": "Latitude for send_location"
                },
                "longitude": {
                    "type": "number",
                    "description": "Longitude for send_location"
                },
                "poll_question": {
                    "type": "string",
                    "description": "Poll question text for send_poll"
                },
                "poll_options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Array of poll option strings (2–10) for send_poll"
                },
                "buttons": {
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": {"type": "string"},
                                "callback_data": {"type": "string"}
                            }
                        }
                    },
                    "description": "2D array of button rows for send_buttons. Each button has 'text' and 'callback_data'."
                },
                "user_id": {
                    "type": "integer",
                    "description": "Telegram user ID for ban_user/unban_user"
                },
                "emoji": {
                    "type": "string",
                    "description": "Emoji for set_reaction (e.g. \"👍\")"
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

        let bot = match self.telegram_state.bot().await {
            Some(b) => b,
            None => {
                return Ok(ToolResult::error(
                    "Telegram is not connected. Ask the user to connect Telegram first \
                     (use the telegram_connect tool)."
                        .to_string(),
                ));
            }
        };

        match action.as_str() {
            // ── send ─────────────────────────────────────────────────────────
            "send" => {
                let text = pget!(get_str(&input, "message")).to_string();
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let chunks = crate::channels::telegram::handler::split_message(&text, 4096);
                for chunk in chunks {
                    if let Err(e) = bot.send_message(ChatId(chat_id), chunk).await {
                        return Ok(ToolResult::error(format!("Failed to send: {e}")));
                    }
                }
                Ok(ToolResult::success(format!(
                    "Message sent to chat {chat_id}."
                )))
            }

            // ── reply ────────────────────────────────────────────────────────
            "reply" => {
                let text = pget!(get_str(&input, "message")).to_string();
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let message_id = pget!(get_id(&input, "message_id"));
                match bot
                    .send_message(ChatId(chat_id), text)
                    .reply_parameters(ReplyParameters::new(MessageId(message_id as i32)))
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Reply sent to message {message_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to reply: {e}"))),
                }
            }

            // ── edit ─────────────────────────────────────────────────────────
            "edit" => {
                let text = pget!(get_str(&input, "message")).to_string();
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let message_id = pget!(get_id(&input, "message_id"));
                match bot
                    .edit_message_text(ChatId(chat_id), MessageId(message_id as i32), text)
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!("Message {message_id} edited."))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to edit: {e}"))),
                }
            }

            // ── delete ───────────────────────────────────────────────────────
            "delete" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let message_id = pget!(get_id(&input, "message_id"));
                match bot
                    .delete_message(ChatId(chat_id), MessageId(message_id as i32))
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message {message_id} deleted."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to delete: {e}"))),
                }
            }

            // ── pin ──────────────────────────────────────────────────────────
            "pin" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let message_id = pget!(get_id(&input, "message_id"));
                match bot
                    .pin_chat_message(ChatId(chat_id), MessageId(message_id as i32))
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!("Message {message_id} pinned."))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to pin: {e}"))),
                }
            }

            // ── unpin ────────────────────────────────────────────────────────
            "unpin" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                match bot.unpin_chat_message(ChatId(chat_id)).await {
                    Ok(_) => Ok(ToolResult::success(
                        "Latest pinned message unpinned.".to_string(),
                    )),
                    Err(e) => Ok(ToolResult::error(format!("Failed to unpin: {e}"))),
                }
            }

            // ── forward ──────────────────────────────────────────────────────
            "forward" => {
                let to_chat = pget!(chat_or_err(&input, &self.telegram_state).await);
                let from_chat = pget!(get_id(&input, "from_chat_id"));
                let message_id = pget!(get_id(&input, "message_id"));
                match bot
                    .forward_message(
                        ChatId(to_chat),
                        ChatId(from_chat),
                        MessageId(message_id as i32),
                    )
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message {message_id} forwarded from chat {from_chat} to {to_chat}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to forward: {e}"))),
                }
            }

            // ── send_photo ───────────────────────────────────────────────────
            "send_photo" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let url = pget!(get_str(&input, "photo_url")).to_string();
                let file = InputFile::url(url.parse().map_err(|e| {
                    crate::brain::tools::error::ToolError::Execution(format!(
                        "Invalid photo_url: {e}"
                    ))
                })?);
                match bot.send_photo(ChatId(chat_id), file).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Photo sent to chat {chat_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send photo: {e}"))),
                }
            }

            // ── send_document ────────────────────────────────────────────────
            "send_document" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let url = pget!(get_str(&input, "document_url")).to_string();
                let file = InputFile::url(url.parse().map_err(|e| {
                    crate::brain::tools::error::ToolError::Execution(format!(
                        "Invalid document_url: {e}"
                    ))
                })?);
                match bot.send_document(ChatId(chat_id), file).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Document sent to chat {chat_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send document: {e}"))),
                }
            }

            // ── send_location ────────────────────────────────────────────────
            "send_location" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let lat = match input.get("latitude").and_then(|v| v.as_f64()) {
                    Some(v) => v,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing required 'latitude' parameter.".to_string(),
                        ));
                    }
                };
                let lng = match input.get("longitude").and_then(|v| v.as_f64()) {
                    Some(v) => v,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing required 'longitude' parameter.".to_string(),
                        ));
                    }
                };
                match bot.send_location(ChatId(chat_id), lat, lng).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Location ({lat}, {lng}) sent to chat {chat_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send location: {e}"))),
                }
            }

            // ── send_poll ────────────────────────────────────────────────────
            "send_poll" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let question = pget!(get_str(&input, "poll_question")).to_string();
                let opts: Vec<String> = match input.get("poll_options").and_then(|v| v.as_array()) {
                    Some(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                    None => {
                        return Ok(ToolResult::error(
                            "Missing required 'poll_options' parameter.".to_string(),
                        ));
                    }
                };
                if opts.len() < 2 {
                    return Ok(ToolResult::error(
                        "'poll_options' must have at least 2 options.".to_string(),
                    ));
                }
                match bot.send_poll(ChatId(chat_id), question, opts).await {
                    Ok(_) => Ok(ToolResult::success(format!("Poll sent to chat {chat_id}."))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send poll: {e}"))),
                }
            }

            // ── send_buttons ─────────────────────────────────────────────────
            "send_buttons" => {
                let text = pget!(get_str(&input, "message")).to_string();
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let rows: Vec<Vec<InlineKeyboardButton>> =
                    match input.get("buttons").and_then(|v| v.as_array()) {
                        Some(outer) => outer
                            .iter()
                            .filter_map(|row| row.as_array())
                            .map(|row| {
                                row.iter()
                                    .filter_map(|btn| {
                                        let text =
                                            btn.get("text").and_then(|v| v.as_str())?.to_string();
                                        let data = btn
                                            .get("callback_data")
                                            .and_then(|v| v.as_str())?
                                            .to_string();
                                        Some(InlineKeyboardButton::callback(text, data))
                                    })
                                    .collect()
                            })
                            .collect(),
                        None => {
                            return Ok(ToolResult::error(
                                "Missing required 'buttons' parameter.".to_string(),
                            ));
                        }
                    };
                let keyboard = InlineKeyboardMarkup::new(rows);
                match bot
                    .send_message(ChatId(chat_id), text)
                    .reply_markup(keyboard)
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message with buttons sent to chat {chat_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to send message with buttons: {e}"
                    ))),
                }
            }

            // ── get_chat ─────────────────────────────────────────────────────
            "get_chat" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                match bot.get_chat(ChatId(chat_id)).await {
                    Ok(chat) => {
                        let info = format!(
                            "Chat {}: type={:?}, title={:?}",
                            chat.id,
                            chat.kind,
                            chat.title()
                        );
                        Ok(ToolResult::success(info))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to get chat: {e}"))),
                }
            }

            // ── ban_user ─────────────────────────────────────────────────────
            "ban_user" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let user_id = pget!(get_id(&input, "user_id"));
                match bot
                    .ban_chat_member(ChatId(chat_id), UserId(user_id as u64))
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "User {user_id} banned from chat {chat_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to ban user: {e}"))),
                }
            }

            // ── unban_user ───────────────────────────────────────────────────
            "unban_user" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let user_id = pget!(get_id(&input, "user_id"));
                match bot
                    .unban_chat_member(ChatId(chat_id), UserId(user_id as u64))
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "User {user_id} unbanned from chat {chat_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to unban user: {e}"))),
                }
            }

            // ── set_reaction ─────────────────────────────────────────────────
            "set_reaction" => {
                let chat_id = pget!(chat_or_err(&input, &self.telegram_state).await);
                let message_id = pget!(get_id(&input, "message_id"));
                let emoji = pget!(get_str(&input, "emoji")).to_string();
                let reactions = vec![ReactionType::Emoji {
                    emoji: emoji.clone(),
                }];
                match bot
                    .set_message_reaction(ChatId(chat_id), MessageId(message_id as i32))
                    .reaction(reactions)
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Reaction {emoji} set on message {message_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to set reaction: {e}"))),
                }
            }

            unknown => Ok(ToolResult::error(format!(
                "Unknown action '{unknown}'. Valid actions: send, reply, edit, delete, pin, \
                 unpin, forward, send_photo, send_document, send_location, send_poll, \
                 send_buttons, get_chat, ban_user, unban_user, set_reaction"
            ))),
        }
    }
}
