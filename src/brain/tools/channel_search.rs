//! Channel Search Tool
//!
//! Searches passively captured channel messages (Telegram groups, etc.).
//! Provides list_chats, recent, and search operations.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::db::ChannelMessageRepository;
use async_trait::async_trait;
use chrono::DateTime;
use serde_json::Value;

/// Tool for listing and searching channel message history.
pub struct ChannelSearchTool {
    repo: ChannelMessageRepository,
}

impl ChannelSearchTool {
    pub fn new(repo: ChannelMessageRepository) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl Tool for ChannelSearchTool {
    fn name(&self) -> &str {
        "channel_search"
    }

    fn description(&self) -> &str {
        "Search or list channel message history captured from Telegram groups, Telegram userbot chats, Discord, Slack, etc. \
         Use 'list_chats' to see known groups/channels with message counts. \
         Use 'recent' to get the last N messages from a specific chat. \
         Use 'search' to find messages by content across chats."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["list_chats", "recent", "search", "attachments"],
                    "description": "'list_chats' to see known chats, 'recent' for last N messages, 'search' to find by content, 'attachments' to list stored files"
                },
                "channel": {
                    "type": "string",
                    "enum": ["telegram", "telegram-userbot", "discord", "slack", "whatsapp"],
                    "description": "Filter by channel platform (omit for all)"
                },
                "chat_id": {
                    "type": "string",
                    "description": "Chat/channel/group ID (required for 'recent' and 'attachments', optional for 'search')"
                },
                "query": {
                    "type": "string",
                    "description": "Search text (required for 'search')"
                },
                "n": {
                    "type": "integer",
                    "description": "Max results to return (default: 20)",
                    "default": 20
                },
                "thread_id": {
                    "type": "string",
                    "description": "Filter by thread/topic ID (optional, for Telegram forum topics)"
                },
                "message_type": {
                    "type": "string",
                    "enum": ["text", "document", "photo", "video", "voice", "video_note", "animation"],
                    "description": "Filter by message type (optional, for 'recent' and 'search')"
                }
            },
            "required": ["operation"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let operation = input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("list_chats");

        let channel = input.get("channel").and_then(|v| v.as_str());
        let n = input.get("n").and_then(|v| v.as_i64()).unwrap_or(20);

        match operation {
            "list_chats" => {
                let chats = self
                    .repo
                    .list_chats(channel)
                    .await
                    .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

                if chats.is_empty() {
                    return Ok(ToolResult::success(
                        "No channel messages captured yet.".to_string(),
                    ));
                }

                let lines: Vec<String> = chats
                    .iter()
                    .map(|c| {
                        let name = c.channel_chat_name.as_deref().unwrap_or("unnamed");
                        let ts = DateTime::from_timestamp(c.last_message_at, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_default();
                        // One id, in one place (#985). This used to print the
                        // chat id twice, the first time quoted directly after
                        // the name, where it read as a handle rather than the
                        // same number again. The oc:// form (#148) is the
                        // target-URL half of the dual-form listing: raw id
                        // for humans, URL for targeting tools.
                        format!(
                            "- [{}] {} (id={}, target `oc://{}/{}`), {} msgs, last: {}",
                            c.channel,
                            name,
                            c.channel_chat_id,
                            c.channel,
                            c.channel_chat_id,
                            c.message_count,
                            ts
                        )
                    })
                    .collect();

                Ok(ToolResult::success(format!(
                    "Known chats ({}):\n{}",
                    chats.len(),
                    lines.join("\n")
                )))
            }

            "recent" => {
                let chat_id = match input.get("chat_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "'chat_id' is required for 'recent' operation.".to_string(),
                        ));
                    }
                };

                let thread_id = input.get("thread_id").and_then(|v| v.as_str());

                let messages = self
                    .repo
                    .recent(channel, chat_id, n, thread_id, None)
                    .await
                    .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

                if messages.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No messages found in chat {chat_id}."
                    )));
                }

                let lines: Vec<String> = messages
                    .iter()
                    .rev() // oldest first for readability
                    .map(|m| {
                        let ts = m.created_at.format("%m-%d %H:%M");
                        let pmid = m
                            .platform_message_id
                            .as_deref()
                            .map(|id| format!(" [msgid:{}]", id))
                            .unwrap_or_default();
                        format!("[{}]{} {}: {}", ts, pmid, m.sender_name, m.content)
                    })
                    .collect();

                Ok(ToolResult::success(format!(
                    "Recent messages in {} ({}):\n{}",
                    chat_id,
                    messages.len(),
                    lines.join("\n")
                )))
            }

            "search" => {
                let query = match input.get("query").and_then(|v| v.as_str()) {
                    Some(q) if !q.is_empty() => q,
                    _ => {
                        return Ok(ToolResult::error(
                            "'query' is required for 'search' operation.".to_string(),
                        ));
                    }
                };

                let chat_id = input.get("chat_id").and_then(|v| v.as_str());
                let thread_id = input.get("thread_id").and_then(|v| v.as_str());

                let messages = self
                    .repo
                    .search(channel, chat_id, query, n, thread_id)
                    .await
                    .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

                if messages.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No messages matching \"{query}\"."
                    )));
                }

                let lines: Vec<String> = messages
                    .iter()
                    .map(|m| {
                        let ts = m.created_at.format("%m-%d %H:%M");
                        let chat = m.channel_chat_name.as_deref().unwrap_or(&m.channel_chat_id);
                        let thread = m
                            .thread_id
                            .as_deref()
                            .map(|t| format!(" [{}]", t))
                            .unwrap_or_default();
                        let topic = m
                            .topic_name
                            .as_deref()
                            .map(|t| format!(" ({})", t))
                            .unwrap_or_default();
                        let pmid = m
                            .platform_message_id
                            .as_deref()
                            .map(|id| format!(" [msgid:{}]", id))
                            .unwrap_or_default();
                        format!(
                            "[{}] [{}:{}]{}{}{}: {}: {}",
                            ts, m.channel, chat, thread, topic, pmid, m.sender_name, m.content
                        )
                    })
                    .collect();

                Ok(ToolResult::success(format!(
                    "Search results for \"{}\" ({}):\n{}",
                    query,
                    messages.len(),
                    lines.join("\n")
                )))
            }

            "attachments" => {
                let chat_id = match input.get("chat_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "'chat_id' is required for 'attachments' operation.".to_string(),
                        ));
                    }
                };

                let thread_id = input.get("thread_id").and_then(|v| v.as_str());

                let messages = self
                    .repo
                    .recent(channel, chat_id, n, thread_id, None)
                    .await
                    .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

                // Filter to only attachment types
                let attachments: Vec<_> = messages
                    .iter()
                    .filter(|m| {
                        matches!(
                            m.message_type.as_str(),
                            "document" | "photo" | "video" | "voice" | "video_note" | "animation"
                        )
                    })
                    .collect();

                if attachments.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No attachments found in chat {chat_id}."
                    )));
                }

                let lines: Vec<String> = attachments
                    .iter()
                    .map(|m| {
                        let ts = m.created_at.format("%m-%d %H:%M");
                        let topic = m
                            .topic_name
                            .as_deref()
                            .map(|t| format!(" ({})", t))
                            .unwrap_or_default();
                        format!(
                            "[{}] [{}]{} {}: {}",
                            ts, m.message_type, topic, m.sender_name, m.content
                        )
                    })
                    .collect();

                Ok(ToolResult::success(format!(
                    "Attachments in {} ({}):\n{}",
                    chat_id,
                    attachments.len(),
                    lines.join("\n")
                )))
            }

            unknown => Ok(ToolResult::error(format!(
                "Unknown operation '{unknown}'. Valid: list_chats, recent, search, attachments"
            ))),
        }
    }
}
