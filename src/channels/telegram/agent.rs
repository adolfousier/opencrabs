//! Telegram Agent
//!
//! Agent struct and startup logic.

use super::TelegramState;
use super::handler::handle_message;
use crate::brain::agent::AgentService;
use crate::config::{RespondTo, VoiceConfig};
use crate::services::{ServiceContext, SessionService};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Telegram bot that forwards messages to the agent
pub struct TelegramAgent {
    agent_service: Arc<AgentService>,
    session_service: SessionService,
    allowed_users: HashSet<i64>,
    voice_config: VoiceConfig,
    openai_api_key: Option<String>,
    /// Shared session ID from the TUI — owner user shares the terminal session
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    telegram_state: Arc<TelegramState>,
    respond_to: RespondTo,
    allowed_channels: HashSet<String>,
}

impl TelegramAgent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_service: Arc<AgentService>,
        service_context: ServiceContext,
        allowed_users: Vec<i64>,
        voice_config: VoiceConfig,
        openai_api_key: Option<String>,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        telegram_state: Arc<TelegramState>,
        respond_to: RespondTo,
        allowed_channels: Vec<String>,
    ) -> Self {
        Self {
            agent_service,
            session_service: SessionService::new(service_context),
            allowed_users: allowed_users.into_iter().collect(),
            voice_config,
            openai_api_key,
            shared_session_id,
            telegram_state,
            respond_to,
            allowed_channels: allowed_channels.into_iter().collect(),
        }
    }

    /// Start the bot as a background task. Returns a JoinHandle.
    pub fn start(self, token: String) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Validate token format BEFORE creating Bot: "numbers:alphanumeric"
            // e.g., "123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
            if token.is_empty() {
                tracing::debug!("Telegram bot token is empty, skipping bot start");
                return;
            }

            if !token.contains(':') {
                tracing::debug!("Telegram bot token missing ':' separator, skipping bot start");
                return;
            }

            let parts: Vec<&str> = token.splitn(2, ':').collect();
            if parts.len() != 2 {
                tracing::debug!("Telegram bot token has invalid format, skipping bot start");
                return;
            }

            // First part must be numeric (bot ID)
            if parts[0].parse::<u64>().is_err() {
                tracing::debug!("Telegram bot token has invalid bot ID, skipping bot start");
                return;
            }

            // Second part must be at least 30 chars (API key)
            if parts[1].len() < 30 {
                tracing::debug!("Telegram bot token has too short API key, skipping bot start");
                return;
            }

            tracing::info!(
                "Starting Telegram bot with {} allowed user(s), STT={}, TTS={}",
                self.allowed_users.len(),
                self.voice_config.stt_enabled,
                self.voice_config.tts_enabled,
            );

            let bot = Bot::new(token.clone());

            // Verify token works with Telegram API before setting up dispatcher
            match bot.get_me().await {
                Ok(me) => {
                    if let Some(ref username) = me.username {
                        tracing::info!("Telegram: bot username is @{}", username);
                        self.telegram_state.set_bot_username(username.clone()).await;
                    }
                    // Store bot in state for proactive messaging only after successful auth
                    self.telegram_state.set_bot(bot.clone()).await;
                }
                Err(e) => {
                    tracing::warn!("Telegram: token validation failed: {}. Bot not started.", e);
                    return;
                }
            }

            // Per-user session tracking for non-owner users (owner shares TUI session)
            let extra_sessions: Arc<Mutex<HashMap<i64, Uuid>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let agent = self.agent_service.clone();
            let session_svc = self.session_service.clone();
            let allowed = Arc::new(self.allowed_users);
            let voice_config = Arc::new(self.voice_config);
            let openai_key = Arc::new(self.openai_api_key);
            let bot_token = Arc::new(token);
            let shared_session = self.shared_session_id.clone();
            let telegram_state = self.telegram_state.clone();
            let respond_to = Arc::new(self.respond_to);
            let allowed_channels: Arc<HashSet<String>> = Arc::new(self.allowed_channels);

            let handler = Update::filter_message().endpoint(move |bot: Bot, msg: Message| {
                let agent = agent.clone();
                let session_svc = session_svc.clone();
                let allowed = allowed.clone();
                let extra_sessions = extra_sessions.clone();
                let voice_config = voice_config.clone();
                let openai_key = openai_key.clone();
                let bot_token = bot_token.clone();
                let shared_session = shared_session.clone();
                let telegram_state = telegram_state.clone();
                let respond_to = respond_to.clone();
                let allowed_channels = allowed_channels.clone();
                async move {
                    handle_message(
                        bot,
                        msg,
                        agent,
                        session_svc,
                        allowed,
                        extra_sessions,
                        voice_config,
                        openai_key,
                        bot_token,
                        shared_session,
                        telegram_state,
                        &respond_to,
                        &allowed_channels,
                    )
                    .await
                }
            });

            Dispatcher::builder(bot, handler).build().dispatch().await;
        })
    }
}
