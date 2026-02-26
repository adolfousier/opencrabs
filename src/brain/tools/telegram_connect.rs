//! Telegram Connect Tool
//!
//! Agent-callable tool that connects a Telegram bot at runtime.
//! Accepts a bot token, saves it to keys.toml, and spawns the bot.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::channels::ChannelFactory;
use crate::channels::telegram::TelegramState;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// Tool that connects a Telegram bot by accepting a bot token from BotFather.
pub struct TelegramConnectTool {
    channel_factory: Arc<ChannelFactory>,
    telegram_state: Arc<TelegramState>,
}

impl TelegramConnectTool {
    pub fn new(channel_factory: Arc<ChannelFactory>, telegram_state: Arc<TelegramState>) -> Self {
        Self {
            channel_factory,
            telegram_state,
        }
    }
}

#[async_trait]
impl Tool for TelegramConnectTool {
    fn name(&self) -> &str {
        "telegram_connect"
    }

    fn description(&self) -> &str {
        "Connect a Telegram bot to OpenCrabs. Accepts a bot token from @BotFather and starts \
         listening for messages. The user must first create a bot via @BotFather on Telegram. \
         Call this when the user asks to connect or set up Telegram."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "description": "Telegram bot token from @BotFather (format: 123456:ABC-DEF...)"
                },
                "allowed_users": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Telegram user IDs allowed to talk to the bot. \
                                    The user can send /start to their bot to see their numeric ID, \
                                    or find it via @userinfobot. If empty, anyone can message the bot."
                }
            },
            "required": ["token", "allowed_users"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network, ToolCapability::SystemModification]
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        if self.telegram_state.is_connected().await {
            return Ok(ToolResult::success(
                "Telegram is already connected.".to_string(),
            ));
        }

        let token = match input.get("token").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing or empty 'token' parameter. \
                     The user needs to create a bot via @BotFather on Telegram and provide the token."
                        .to_string(),
                ));
            }
        };

        let allowed_users: Vec<i64> = input
            .get("allowed_users")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Capture owner ID before vec is moved into TelegramAgent
        let owner_chat_id = allowed_users.first().copied();

        // Save token to keys.toml for persistence
        let _ = crate::config::write_secret_key("channels.telegram", "token", &token);

        // Persist enabled state to config
        let _ = crate::config::Config::write_key("channels.telegram", "enabled", "true");

        // Create and spawn the Telegram agent
        let factory = self.channel_factory.clone();
        let agent = factory.create_agent_service();
        let service_context = factory.service_context();
        let voice_config = factory.voice_config().clone();
        let shared_session = factory.shared_session_id();
        let telegram_state = self.telegram_state.clone();

        // Get OpenAI TTS key from config (keys.toml)
        let openai_key = factory.openai_tts_key();

        let tg_agent = crate::channels::telegram::TelegramAgent::new(
            agent,
            service_context,
            allowed_users,
            voice_config,
            openai_key,
            shared_session,
            telegram_state.clone(),
            crate::config::RespondTo::default(),
            vec![],
        );

        let _handle = tg_agent.start(token);

        // Wait for the bot to connect (agent stores Bot in state)
        let timeout = Duration::from_secs(15);
        let start = std::time::Instant::now();
        loop {
            if telegram_state.is_connected().await {
                // Set owner chat ID from first allowed user (chat_id == user_id for DMs)
                if let Some(owner_id) = owner_chat_id {
                    telegram_state.set_owner_chat_id(owner_id).await;
                }

                let mut msg = "Telegram bot connected successfully! Now listening for messages. \
                     Connection persists across restarts."
                    .to_string();

                if owner_chat_id.is_some() {
                    msg.push_str(
                        "\n\nIMPORTANT: Send a welcome message to the user RIGHT NOW \
                         using the `telegram_send` tool. Be wildly fun — talk like their \
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
                    "Timed out waiting for Telegram bot to connect (15s). \
                     Check that the bot token from @BotFather is valid."
                        .to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}
