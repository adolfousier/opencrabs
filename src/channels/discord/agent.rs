//! Discord Agent
//!
//! Agent struct and startup logic. Mirrors the Telegram/WhatsApp agent pattern.

use super::handler;
use super::DiscordState;
use crate::brain::agent::AgentService;
use crate::config::{RespondTo, VoiceConfig};
use crate::services::{ServiceContext, SessionService};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

/// Discord bot that forwards messages to the AgentService
pub struct DiscordAgent {
    agent_service: Arc<AgentService>,
    session_service: SessionService,
    allowed_users: Vec<i64>,
    voice_config: VoiceConfig,
    openai_api_key: Option<String>,
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    discord_state: Arc<DiscordState>,
    respond_to: RespondTo,
    allowed_channels: Vec<String>,
}

impl DiscordAgent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_service: Arc<AgentService>,
        service_context: ServiceContext,
        allowed_users: Vec<i64>,
        voice_config: VoiceConfig,
        openai_api_key: Option<String>,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        discord_state: Arc<DiscordState>,
        respond_to: RespondTo,
        allowed_channels: Vec<String>,
    ) -> Self {
        Self {
            agent_service,
            session_service: SessionService::new(service_context),
            allowed_users,
            voice_config,
            openai_api_key,
            shared_session_id,
            discord_state,
            respond_to,
            allowed_channels,
        }
    }

    /// Start the bot as a background task. Returns a JoinHandle.
    pub fn start(self, token: String) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Validate token format - Discord tokens are typically ~70 chars
            if token.is_empty() || token.len() < 50 {
                tracing::debug!("Discord bot token not configured or invalid, skipping bot start");
                return;
            }

            tracing::info!(
                "Starting Discord bot with {} allowed user(s), STT={}, TTS={}",
                self.allowed_users.len(),
                self.voice_config.stt_enabled,
                self.voice_config.tts_enabled,
            );

            let allowed: Arc<HashSet<i64>> = Arc::new(self.allowed_users.into_iter().collect());
            let extra_sessions: Arc<Mutex<HashMap<u64, Uuid>>> =
                Arc::new(Mutex::new(HashMap::new()));

            let allowed_channels: HashSet<String> = self.allowed_channels.into_iter().collect();

            let voice_config = Arc::new(self.voice_config);
            let openai_key = Arc::new(self.openai_api_key);

            let event_handler = Handler {
                agent: self.agent_service,
                session_svc: self.session_service,
                allowed,
                extra_sessions,
                shared_session: self.shared_session_id,
                discord_state: self.discord_state,
                respond_to: self.respond_to,
                allowed_channels: Arc::new(allowed_channels),
                voice_config,
                openai_key,
            };

            let intents = GatewayIntents::GUILD_MESSAGES
                | GatewayIntents::DIRECT_MESSAGES
                | GatewayIntents::MESSAGE_CONTENT;

            let mut client = match Client::builder(&token, intents)
                .event_handler(event_handler)
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Discord: failed to create client: {}", e);
                    return;
                }
            };

            if let Err(e) = client.start().await {
                tracing::error!("Discord: client error: {}", e);
            }
        })
    }
}

/// Serenity event handler — routes messages to the agent
struct Handler {
    agent: Arc<AgentService>,
    session_svc: SessionService,
    allowed: Arc<HashSet<i64>>,
    extra_sessions: Arc<Mutex<HashMap<u64, Uuid>>>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    discord_state: Arc<DiscordState>,
    respond_to: RespondTo,
    allowed_channels: Arc<HashSet<String>>,
    voice_config: Arc<VoiceConfig>,
    openai_key: Arc<Option<String>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!(
            "Discord: connected as {} (id={})",
            ready.user.name,
            ready.user.id
        );
        self.discord_state
            .set_connected(ctx.http.clone(), None)
            .await;
        self.discord_state
            .set_bot_user_id(ready.user.id.get())
            .await;
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Skip bot messages
        if msg.author.bot {
            return;
        }

        handler::handle_message(
            &ctx,
            &msg,
            self.agent.clone(),
            self.session_svc.clone(),
            self.allowed.clone(),
            self.extra_sessions.clone(),
            self.shared_session.clone(),
            self.discord_state.clone(),
            &self.respond_to,
            &self.allowed_channels,
            self.voice_config.clone(),
            self.openai_key.clone(),
        )
        .await;
    }
}
