//! Discord Connect Tool
//!
//! Agent-callable tool that connects a Discord bot at runtime.
//! Accepts a bot token, saves it to config/keyring, spawns the bot,
//! and waits for a successful connection.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::channels::discord::DiscordState;
use crate::channels::ChannelFactory;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// Tool that connects a Discord bot by accepting a bot token.
pub struct DiscordConnectTool {
    channel_factory: Arc<ChannelFactory>,
    discord_state: Arc<DiscordState>,
}

impl DiscordConnectTool {
    pub fn new(channel_factory: Arc<ChannelFactory>, discord_state: Arc<DiscordState>) -> Self {
        Self {
            channel_factory,
            discord_state,
        }
    }
}

#[async_trait]
impl Tool for DiscordConnectTool {
    fn name(&self) -> &str {
        "discord_connect"
    }

    fn description(&self) -> &str {
        "Connect a Discord bot to OpenCrabs. Accepts a bot token and starts listening for \
         messages. The user must first create a bot at https://discord.com/developers/applications, \
         enable MESSAGE CONTENT intent, and invite the bot to their server. \
         Call this when the user asks to connect or set up Discord."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "description": "Discord bot token from the Developer Portal"
                },
                "channel_id": {
                    "type": "integer",
                    "description": "Discord channel ID where the bot should send its welcome message. \
                                    Right-click a channel → Copy Channel ID (Developer Mode must be on)."
                },
                "allowed_users": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Discord user IDs allowed to talk to the bot. \
                                    Enable Developer Mode in Discord settings, then right-click \
                                    your username → Copy User ID. If empty, anyone can message the bot."
                }
            },
            "required": ["token", "allowed_users"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network, ToolCapability::SystemModification]
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        // Check if already connected
        if self.discord_state.is_connected().await {
            return Ok(ToolResult::success(
                "Discord is already connected.".to_string(),
            ));
        }

        let token = match input.get("token").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing or empty 'token' parameter. \
                     The user needs to provide their Discord bot token from \
                     https://discord.com/developers/applications"
                        .to_string(),
                ));
            }
        };

        let allowed_users: Vec<i64> = input
            .get("allowed_users")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Save token to keyring for persistence
        if let Ok(entry) = keyring::Entry::new("opencrabs", "discord_bot_token") {
            let _ = entry.set_password(&token);
        }

        // Persist enabled state to config
        let _ = crate::config::Config::write_key("channels.discord", "enabled", "true");

        // Create and spawn the Discord agent
        let factory = self.channel_factory.clone();
        let agent = factory.create_agent_service();
        let service_context = factory.service_context();
        let voice_config = factory.voice_config().clone();
        let openai_tts_key = factory.openai_tts_key();
        let shared_session = factory.shared_session_id();
        let discord_state = self.discord_state.clone();

        let dc_agent = crate::channels::discord::DiscordAgent::new(
            agent,
            service_context,
            allowed_users,
            voice_config,
            openai_tts_key,
            shared_session,
            discord_state.clone(),
            crate::config::RespondTo::default(),
            vec![],
        );

        let _handle = dc_agent.start(token);

        // Wait for the bot to connect (ready event sets discord_state)
        let timeout = Duration::from_secs(30);
        let start = std::time::Instant::now();
        loop {
            if discord_state.is_connected().await {
                // Set owner channel if provided so send tool works immediately
                if let Some(ch) = input.get("channel_id").and_then(|v| v.as_u64()) {
                    discord_state.set_owner_channel(ch).await;
                }

                let mut msg = "Discord bot connected successfully! Now listening for messages. \
                     Connection persists across restarts."
                    .to_string();

                if input.get("channel_id").and_then(|v| v.as_u64()).is_some() {
                    msg.push_str(
                        "\n\nIMPORTANT: Send a welcome message to the user RIGHT NOW \
                         using the `discord_send` tool. Be wildly fun — talk like their \
                         best buddy from childhood. You can curse, be unhinged, hilarious, \
                         and totally informal. Use your personality, SOUL.md, and any context \
                         you have about the user. Make it memorable and uniquely yours. \
                         No generic corporate greetings — surprise them!",
                    );
                }

                return Ok(ToolResult::success(msg));
            }
            if start.elapsed() > timeout {
                return Ok(ToolResult::error(
                    "Timed out waiting for Discord bot to connect (30s). \
                     Check that the bot token is valid and the bot has the required intents \
                     (MESSAGE CONTENT) enabled in the Developer Portal."
                        .to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}
