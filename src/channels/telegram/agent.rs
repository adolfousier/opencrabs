//! Telegram Agent
//!
//! Agent struct and startup logic.

use super::TelegramState;
use super::handler::handle_message;
use crate::brain::agent::AgentService;
use crate::config::Config;
use crate::db::ChannelMessageRepository;
use crate::services::{ServiceContext, SessionService};
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Telegram bot that forwards messages to the agent
pub struct TelegramAgent {
    agent_service: Arc<AgentService>,
    session_service: SessionService,
    /// Shared session ID from the TUI — owner user shares the terminal session
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    telegram_state: Arc<TelegramState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    channel_msg_repo: ChannelMessageRepository,
}

impl TelegramAgent {
    pub fn new(
        agent_service: Arc<AgentService>,
        service_context: ServiceContext,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        telegram_state: Arc<TelegramState>,
        config_rx: tokio::sync::watch::Receiver<Config>,
        channel_msg_repo: ChannelMessageRepository,
    ) -> Self {
        Self {
            agent_service,
            session_service: SessionService::new(service_context),
            shared_session_id,
            telegram_state,
            config_rx,
            channel_msg_repo,
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

            // Read initial config for logging
            let cfg = self.config_rx.borrow().clone();
            tracing::info!(
                "Starting Telegram bot with {} allowed user(s), STT={}, TTS={}",
                cfg.channels.telegram.allowed_users.len(),
                cfg.voice.stt_enabled,
                cfg.voice.tts_enabled,
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
            let extra_sessions: Arc<Mutex<HashMap<i64, (Uuid, std::time::Instant)>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let agent = self.agent_service.clone();
            let session_svc = self.session_service.clone();
            let bot_token = Arc::new(token);
            let shared_session = self.shared_session_id.clone();
            let telegram_state = self.telegram_state.clone();
            let config_rx = self.config_rx.clone();
            let channel_msg_repo = self.channel_msg_repo.clone();

            // ── Message handler ───────────────────────────────────────────────
            let msg_handler = Update::filter_message().endpoint({
                let agent = agent.clone();
                let session_svc = session_svc.clone();
                let extra_sessions = extra_sessions.clone();
                let bot_token = bot_token.clone();
                let shared_session = shared_session.clone();
                let telegram_state = telegram_state.clone();
                let config_rx = config_rx.clone();
                let channel_msg_repo = channel_msg_repo.clone();
                move |bot: Bot, msg: Message| {
                    let agent = agent.clone();
                    let session_svc = session_svc.clone();
                    let extra_sessions = extra_sessions.clone();
                    let bot_token = bot_token.clone();
                    let shared_session = shared_session.clone();
                    let telegram_state = telegram_state.clone();
                    let config_rx = config_rx.clone();
                    let channel_msg_repo = channel_msg_repo.clone();
                    async move {
                        // Spawn in background so the dispatcher is free to
                        // process callback queries (approval button clicks)
                        // while the agent is running.
                        tokio::spawn(async move {
                            let _ = handle_message(
                                bot,
                                msg,
                                agent,
                                session_svc,
                                extra_sessions,
                                bot_token,
                                shared_session,
                                telegram_state,
                                config_rx,
                                channel_msg_repo,
                            )
                            .await;
                        });
                        ResponseResult::Ok(())
                    }
                }
            });

            // ── Callback query handler (for Approve / Deny inline buttons) ────
            let cb_handler = Update::filter_callback_query().endpoint({
                let telegram_state = telegram_state.clone();
                let agent = agent.clone();
                move |bot: Bot, query: CallbackQuery| {
                    let state = telegram_state.clone();
                    let agent = agent.clone();
                    async move {
                        if let Some(data) = query.data.as_deref() {
                            tracing::info!("Telegram callback query received: data={}", data);

                            // Model switch callback
                            if let Some(model_name) = data.strip_prefix("model:") {
                                crate::channels::commands::switch_model(&agent, model_name);
                                let _ = bot
                                    .answer_callback_query(&query.id)
                                    .text(format!("Switched to {}", model_name))
                                    .await;
                                // Update the message to reflect the new selection
                                if let Some(msg) = &query.message {
                                    let text =
                                        format!("✅ Model switched to <code>{}</code>", model_name);
                                    use teloxide::payloads::EditMessageTextSetters;
                                    use teloxide::prelude::Requester;
                                    let _ = bot
                                        .edit_message_text(msg.chat().id, msg.id(), &text)
                                        .parse_mode(teloxide::types::ParseMode::Html)
                                        .reply_markup(
                                            teloxide::types::InlineKeyboardMarkup::default(),
                                        )
                                        .await;
                                }
                                return ResponseResult::Ok(());
                            }

                            let (approved, always, yolo, id) =
                                if let Some(id) = data.strip_prefix("approve:") {
                                    (true, false, false, id.to_string())
                                } else if let Some(id) = data.strip_prefix("always:") {
                                    (true, true, false, id.to_string())
                                } else if let Some(id) = data.strip_prefix("yolo:") {
                                    (true, true, true, id.to_string())
                                } else if let Some(id) = data.strip_prefix("deny:") {
                                    (false, false, false, id.to_string())
                                } else {
                                    tracing::warn!("Telegram: unknown callback data: {}", data);
                                    let _ = bot.answer_callback_query(&query.id).await;
                                    return ResponseResult::Ok(());
                                };

                            // Persist YOLO (permanent) directly from callback
                            if yolo {
                                crate::utils::persist_auto_always_policy();
                            }

                            let resolved = state.resolve_pending_approval(&id, approved, always).await;
                            tracing::info!(
                                "Telegram approval resolved: id={}, approved={}, always={}, found_pending={}",
                                id, approved, always, resolved
                            );
                            if !resolved {
                                tracing::warn!(
                                    "Telegram: no pending approval found for id={} — may have timed out or already resolved",
                                    id
                                );
                            }
                            let _ = bot.answer_callback_query(&query.id).await;

                            // Edit the approval message: keep original context, append outcome, remove buttons
                            if let Some(msg) = &query.message {
                                let label = if yolo {
                                    "\n\n🔥 YOLO — always approved"
                                } else if always {
                                    "\n\n🔁 Always approved (session)"
                                } else if approved {
                                    "\n\n✅ Approved"
                                } else {
                                    "\n\n❌ Denied"
                                };
                                let original_text = match msg {
                                    teloxide::types::MaybeInaccessibleMessage::Regular(m) => {
                                        m.text().unwrap_or("").to_string()
                                    }
                                    _ => String::new(),
                                };
                                let updated = format!("{}{}", original_text, label);
                                use teloxide::payloads::EditMessageTextSetters;
                                use teloxide::prelude::Requester;
                                if let Err(e) = bot
                                    .edit_message_text(msg.chat().id, msg.id(), &updated)
                                    .reply_markup(teloxide::types::InlineKeyboardMarkup::default())
                                    .await
                                {
                                    tracing::error!("Telegram: failed to edit approval message: {}", e);
                                }
                            } else {
                                tracing::warn!("Telegram: callback query has no message — cannot edit");
                            }
                        } else {
                            tracing::warn!("Telegram: callback query with no data");
                            let _ = bot.answer_callback_query(&query.id).await;
                        }
                        ResponseResult::Ok(())
                    }
                }
            });

            let tree = dptree::entry().branch(msg_handler).branch(cb_handler);

            Dispatcher::builder(bot, tree).build().dispatch().await;
        })
    }
}
