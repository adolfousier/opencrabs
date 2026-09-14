//! WhatsApp History Search Tool
//!
//! Read-only retrieval over the bounded history store (#1525): rows captured
//! from offline-sync / PDO-recovered frames plus live-captured messages, all
//! in the shared `channel_messages` table. The decision doc grants the agent
//! no write surface — import requests fire from the connect path when the
//! owner opts a chat in via config, never from here.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolHints, ToolResult};
use crate::channels::whatsapp::history::clamp_search;
use crate::db::repository::channel_message::ChannelMessageRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Read-only search over one WhatsApp chat's stored history.
pub struct WhatsAppHistoryTool {
    repo: ChannelMessageRepository,
}

impl WhatsAppHistoryTool {
    pub fn new(repo: ChannelMessageRepository) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl Tool for WhatsAppHistoryTool {
    fn name(&self) -> &str {
        "whatsapp_history"
    }

    fn description(&self) -> &str {
        "Read-only search over a WhatsApp chat's stored history: imported \
         (offline-sync / recovered) and live-captured messages, newest first. \
         Needs the chat's normalized WhatsApp id (e.g. '5511999999999@s.whatsapp.net' \
         or a group id ending '@g.us'). Optional substring query, day window \
         (max 90), and limit (max 100)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "chat": {
                    "type": "string",
                    "description": "Normalized WhatsApp chat id as stored (the JID string)."
                },
                "query": {
                    "type": "string",
                    "description": "Optional case-insensitive substring filter; omit to list the newest stored messages in the window."
                },
                "days": {
                    "type": "integer",
                    "description": "Look-back window in days, clamped to 1..=90 (default 90)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum rows returned, clamped to 1..=100 (default 50)."
                }
            },
            "required": ["chat"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![]
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            read_only: true,
            destructive: false,
            ..Default::default()
        }
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let chat = match input.get("chat").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing required parameter 'chat'.".to_string(),
                ));
            }
        };
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let days = input.get("days").and_then(|v| v.as_i64()).unwrap_or(90);
        let limit = input.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
        let (days, limit) = clamp_search(days, limit);
        let now = Utc::now();
        let since = (now - chrono::Duration::days(days)).timestamp();

        let rows = match self
            .repo
            .search_history("whatsapp", &chat, &query, since, limit)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult::error(format!("History search failed: {e}")));
            }
        };

        if rows.is_empty() {
            let qual = if query.is_empty() {
                String::new()
            } else {
                format!(" matching '{query}'")
            };
            return Ok(ToolResult::success(format!(
                "No stored WhatsApp history for {chat} in the last {days} day(s){qual}."
            )));
        }

        let lines: Vec<String> = rows
            .iter()
            .map(|(content, sender, ts)| {
                let when = DateTime::from_timestamp(*ts, 0)
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| format!("{ts}"));
                format!("[{when}] {sender}: {content}")
            })
            .collect();
        Ok(ToolResult::success(format!(
            "{} stored message(s) for {chat}, newest first (window {days}d):\n{}",
            rows.len(),
            lines.join("\n")
        )))
    }
}
