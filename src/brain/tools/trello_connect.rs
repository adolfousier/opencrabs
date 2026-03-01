//! Trello Connect Tool
//!
//! Agent-callable tool that connects a Trello board at runtime.
//! Verifies credentials, resolves the board, saves config, and starts the polling agent.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::channels::ChannelFactory;
use crate::channels::trello::TrelloState;
use crate::channels::trello::client::TrelloClient;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// Tool that connects a Trello board to OpenCrabs.
pub struct TrelloConnectTool {
    channel_factory: Arc<ChannelFactory>,
    trello_state: Arc<TrelloState>,
}

impl TrelloConnectTool {
    pub fn new(channel_factory: Arc<ChannelFactory>, trello_state: Arc<TrelloState>) -> Self {
        Self {
            channel_factory,
            trello_state,
        }
    }
}

#[async_trait]
impl Tool for TrelloConnectTool {
    fn name(&self) -> &str {
        "trello_connect"
    }

    fn description(&self) -> &str {
        "Connect one or more Trello boards to OpenCrabs. Accepts a Trello API Key and API Token \
         from https://trello.com/power-ups/admin, plus board names or IDs to monitor. \
         Board names are resolved automatically — you can mix names and 24-char hex IDs. \
         Once connected, the agent polls every 30 s for new card comments and replies. \
         Call this when the user asks to connect or set up Trello."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "api_key": {
                    "type": "string",
                    "description": "Trello API Key from https://trello.com/power-ups/admin"
                },
                "api_token": {
                    "type": "string",
                    "description": "Trello API Token (generated alongside the API Key)"
                },
                "boards": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Board names or IDs to monitor for card comments. \
                                    Names are resolved to IDs automatically. \
                                    At least one board is required."
                },
                "allowed_users": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Trello member IDs allowed to interact with the bot. \
                                    Empty = respond to all members."
                }
            },
            "required": ["api_key", "api_token", "boards"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network, ToolCapability::SystemModification]
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        // Check if already connected
        if self.trello_state.is_connected().await {
            return Ok(ToolResult::success(
                "Trello is already connected and polling.".to_string(),
            ));
        }

        let api_key = match input.get("api_key").and_then(|v| v.as_str()) {
            Some(k) if !k.is_empty() => k.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing or empty 'api_key'. Get it from https://trello.com/power-ups/admin"
                        .to_string(),
                ));
            }
        };

        let api_token = match input.get("api_token").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing or empty 'api_token'. Get it from https://trello.com/power-ups/admin"
                        .to_string(),
                ));
            }
        };

        let board_queries: Vec<String> = input
            .get("boards")
            .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();

        if board_queries.is_empty() {
            return Ok(ToolResult::error(
                "Missing or empty 'boards'. Provide at least one board name or ID.".to_string(),
            ));
        }

        let allowed_users: Vec<String> = input
            .get("allowed_users")
            .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
            .unwrap_or_default();

        // Verify credentials
        let client = TrelloClient::new(&api_key, &api_token);

        let me = match client.get_member_me().await {
            Ok(m) => m,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Trello credential verification failed: {}. \
                     Check your API Key and Token at https://trello.com/power-ups/admin",
                    e
                )));
            }
        };

        // Resolve each board query to an ID
        let mut board_ids: Vec<String> = Vec::new();
        let mut board_names: Vec<String> = Vec::new();
        for query in &board_queries {
            match client.resolve_board(query).await {
                Ok(id) => {
                    board_names.push(query.clone());
                    board_ids.push(id);
                }
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Could not find Trello board '{}': {}",
                        query, e
                    )));
                }
            }
        }

        // Count open cards across all boards
        let mut total_open_cards = 0usize;
        for bid in &board_ids {
            total_open_cards += client
                .get_board_cards(bid)
                .await
                .map(|c| c.len())
                .unwrap_or(0);
        }

        // Persist to keys.toml / config.toml
        let _ = crate::config::write_secret_key("channels.trello", "app_token", &api_key);
        let _ = crate::config::write_secret_key("channels.trello", "token", &api_token);
        let _ = crate::config::Config::write_key("channels.trello", "enabled", "true");
        let _ =
            crate::config::Config::write_array("channels.trello", "allowed_channels", &board_ids);
        if !allowed_users.is_empty() {
            let _ = crate::config::Config::write_array(
                "channels.trello",
                "allowed_users",
                &allowed_users,
            );
        }

        // Spawn the TrelloAgent
        let factory = self.channel_factory.clone();
        let agent_svc = factory.create_agent_service();
        let service_ctx = factory.service_context();
        let shared_session = factory.shared_session_id();
        let trello_state = self.trello_state.clone();

        let trello_agent = crate::channels::trello::TrelloAgent::new(
            agent_svc,
            service_ctx,
            allowed_users,
            shared_session,
            trello_state.clone(),
            board_ids,
        );

        let _handle = trello_agent.start(api_key, api_token);

        // Wait up to 5 seconds for the agent to confirm connection
        let timeout = Duration::from_secs(5);
        let start = std::time::Instant::now();
        loop {
            if trello_state.is_connected().await {
                break;
            }
            if start.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        let connected = trello_state.is_connected().await;
        if connected {
            Ok(ToolResult::success(format!(
                "Trello connected! Monitoring {} board(s): {} ({} total open cards).\n\
                 Authenticated as @{}. Polling every 30 seconds for new card comments.",
                board_names.len(),
                board_names.join(", "),
                total_open_cards,
                me.username
            )))
        } else {
            Ok(ToolResult::error(
                "Trello agent started but did not confirm connection within 5 seconds. \
                 Check the logs for more detail."
                    .to_string(),
            ))
        }
    }
}
