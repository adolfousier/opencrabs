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
                cfg.voice_config().stt_enabled,
                cfg.voice_config().tts_enabled,
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
                let bot_token = bot_token.clone();
                let shared_session = shared_session.clone();
                let telegram_state = telegram_state.clone();
                let config_rx = config_rx.clone();
                let channel_msg_repo = channel_msg_repo.clone();
                move |bot: Bot, msg: Message| {
                    let agent = agent.clone();
                    let session_svc = session_svc.clone();
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
                            let result = tokio::task::spawn(async move {
                                handle_message(
                                    bot,
                                    msg,
                                    agent,
                                    session_svc,
                                    bot_token,
                                    shared_session,
                                    telegram_state,
                                    config_rx,
                                    channel_msg_repo,
                                )
                                .await
                            })
                            .await;
                            match result {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => {
                                    tracing::error!("Telegram handle_message error: {e}");
                                }
                                Err(panic_err) => {
                                    tracing::error!(
                                        "Telegram handle_message panicked: {:?}",
                                        panic_err
                                    );
                                }
                            }
                        });
                        ResponseResult::Ok(())
                    }
                }
            });

            // ── Callback query handler (for Approve / Deny inline buttons) ────
            let cb_handler = Update::filter_callback_query().endpoint({
                let telegram_state = telegram_state.clone();
                let agent = agent.clone();
                let session_svc = session_svc.clone();
                let shared_session = shared_session.clone();
                let extra_sessions = extra_sessions.clone();
                let config_rx = config_rx.clone();
                move |bot: Bot, query: CallbackQuery| {
                    let state = telegram_state.clone();
                    let agent = agent.clone();
                    let session_svc = session_svc.clone();
                    let shared_session = shared_session.clone();
                    let extra_sessions = extra_sessions.clone();
                    let config_rx = config_rx.clone();
                    async move {
                        if let Some(data) = query.data.as_deref() {
                            tracing::info!("Telegram callback query received: data={}", data);

                            // Provider picker callback → show models for that provider
                            if let Some(provider_name) = data.strip_prefix("provider:") {
                                let resp = crate::channels::commands::models_for_provider(provider_name).await;

                                // Agent-handled providers (OpenRouter 300+ models, custom)
                                // Switch to default if set, then let the agent follow up.
                                if resp.agent_handled {
                                    let session_id = *shared_session.lock().await;
                                    let display = crate::channels::commands::provider_display_name(provider_name);
                                    // Switch to this provider with its default model
                                    if let Ok(config) = crate::config::Config::load()
                                        && let Ok(new_provider) = crate::brain::provider::factory::create_provider_by_name(&config, provider_name)
                                    {
                                        agent.swap_provider(new_provider);
                                    }
                                    if !resp.current_model.is_empty() {
                                        let _ = crate::channels::commands::switch_model(&agent, &resp.current_model, session_id).await;
                                    }
                                    let _ = bot.answer_callback_query(&query.id).await;
                                    // Send synthetic message to agent so it handles follow-up
                                    let prompt = if resp.current_model.is_empty() {
                                        format!(
                                            "[System: User selected {} provider but no default model is set. \
                                             Ask them which model they want. Use config_manager tool to read \
                                             providers section, then set the default_model. Keep current provider \
                                             until a model is chosen.]",
                                            display
                                        )
                                    } else {
                                        format!(
                                            "[System: User switched to {} provider with model {}. \
                                             Confirm the switch. Ask if they want a different model — \
                                             if so, use config_manager to update providers.{}.default_model \
                                             and confirm.]",
                                            display, resp.current_model,
                                            if provider_name == "openrouter" { "openrouter" } else { provider_name }
                                        )
                                    };
                                    if let Some(sid) = session_id {
                                        let agent_clone = agent.clone();
                                        let bot_clone = bot.clone();
                                        let chat_id = query.message.as_ref().map(|m| m.chat().id).unwrap_or(teloxide::types::ChatId(0));
                                        tokio::spawn(async move {
                                            match agent_clone.send_message(sid, prompt, None).await {
                                                Ok(resp) => {
                                                    let clean = crate::utils::sanitize::strip_llm_artifacts(&resp.content);
                                                    let html = crate::channels::telegram::handler::md_to_html(&clean);
                                                    let _ = bot_clone.send_message(chat_id, html)
                                                        .parse_mode(teloxide::types::ParseMode::Html)
                                                        .await;
                                                }
                                                Err(e) => {
                                                    tracing::error!("Agent follow-up failed: {}", e);
                                                }
                                            }
                                        });
                                    }
                                    return ResponseResult::Ok(());
                                }

                                if resp.models.is_empty() {
                                    let _ = bot
                                        .answer_callback_query(&query.id)
                                        .text("No models available for this provider")
                                        .await;
                                    return ResponseResult::Ok(());
                                }
                                let _ = bot.answer_callback_query(&query.id).await;
                                if let Some(msg) = &query.message {
                                    use teloxide::payloads::EditMessageTextSetters;
                                    use teloxide::prelude::Requester;
                                    use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
                                    let rows: Vec<Vec<InlineKeyboardButton>> = resp
                                        .models
                                        .iter()
                                        .map(|m| {
                                            let display = if *m == resp.current_model {
                                                format!("✓ {}", m)
                                            } else {
                                                m.clone()
                                            };
                                            vec![InlineKeyboardButton::callback(
                                                display,
                                                format!("model:{}:{}", resp.provider_name, m),
                                            )]
                                        })
                                        .collect();
                                    let keyboard = InlineKeyboardMarkup::new(rows);
                                    let text = crate::channels::telegram::handler::md_to_html(&resp.text);
                                    let _ = bot
                                        .edit_message_text(msg.chat().id, msg.id(), &text)
                                        .parse_mode(teloxide::types::ParseMode::Html)
                                        .reply_markup(keyboard)
                                        .await;
                                }
                                return ResponseResult::Ok(());
                            }

                            // Model switch callback (format: model:<provider>:<model>)
                            if let Some(rest) = data.strip_prefix("model:") {
                                let (provider_name, model_name) = if let Some((p, m)) = rest.split_once(':') {
                                    (Some(p), m)
                                } else {
                                    (None, rest)
                                };
                                // Switch provider if specified and different
                                let mut provider_err: Option<String> = None;
                                if let Some(pname) = provider_name {
                                    match crate::config::Config::load() {
                                        Ok(config) => match crate::brain::provider::factory::create_provider_by_name(&config, pname) {
                                            Ok(new_provider) => agent.swap_provider(new_provider),
                                            Err(e) => provider_err = Some(format!("Failed to create provider '{}': {}", pname, e)),
                                        },
                                        Err(e) => provider_err = Some(format!("Failed to load config: {}", e)),
                                    }
                                }
                                let (switch_ok, display_text) = if let Some(err) = provider_err {
                                    (false, format!("⚠️ {}", err))
                                } else {
                                    let session_id = *shared_session.lock().await;
                                    match crate::channels::commands::switch_model(&agent, model_name, session_id).await {
                                        Ok(_) => (true, format!("✅ Model switched to <code>{}</code>", model_name)),
                                        Err(e) => (false, format!("⚠️ {}", e)),
                                    }
                                };
                                let _ = bot.answer_callback_query(&query.id).await;
                                if let Some(msg) = &query.message {
                                    use teloxide::payloads::EditMessageTextSetters;
                                    use teloxide::prelude::Requester;
                                    let _ = bot
                                        .edit_message_text(msg.chat().id, msg.id(), &display_text)
                                        .parse_mode(teloxide::types::ParseMode::Html)
                                        .reply_markup(
                                            teloxide::types::InlineKeyboardMarkup::default(),
                                        )
                                        .await;
                                }
                                if !switch_ok {
                                    tracing::warn!("Telegram model switch failed: {}", display_text);
                                }
                                return ResponseResult::Ok(());
                            }

                            // Session switch callback
                            if let Some(session_id_str) = data.strip_prefix("session:") {
                                if let Ok(new_id) = session_id_str.parse::<Uuid>() {
                                    // Determine if caller is owner
                                    let cfg = config_rx.borrow().clone();
                                    let caller_id = query.from.id.0 as i64;
                                    let owner_id = cfg
                                        .channels
                                        .telegram
                                        .allowed_users
                                        .first()
                                        .and_then(|s| s.parse::<i64>().ok());
                                    let is_owner = cfg.channels.telegram.allowed_users.is_empty()
                                        || owner_id == Some(caller_id);

                                    if is_owner {
                                        *shared_session.lock().await = Some(new_id);
                                    } else {
                                        extra_sessions.lock().await.insert(
                                            caller_id,
                                            (new_id, std::time::Instant::now()),
                                        );
                                    }
                                    state
                                        .register_session_chat(new_id, query.message.as_ref().map(|m| m.chat().id.0).unwrap_or(caller_id))
                                        .await;
                                    let _ = bot
                                        .answer_callback_query(&query.id)
                                        .text("Session switched")
                                        .await;
                                    if let Some(msg) = &query.message {
                                        use teloxide::payloads::EditMessageTextSetters;
                                        use teloxide::prelude::Requester;
                                        let _ = bot
                                            .edit_message_text(
                                                msg.chat().id,
                                                msg.id(),
                                                {
                                                    let display = match session_svc.get_session(new_id).await {
                                                        Ok(Some(s)) => s.title.unwrap_or_else(|| session_id_str[..8.min(session_id_str.len())].to_string()),
                                                        _ => session_id_str[..8.min(session_id_str.len())].to_string(),
                                                    };
                                                    format!("✅ Switched to session <code>{}</code>", display)
                                                },
                                            )
                                            .parse_mode(teloxide::types::ParseMode::Html)
                                            .reply_markup(
                                                teloxide::types::InlineKeyboardMarkup::default(),
                                            )
                                            .await;
                                    }
                                } else {
                                    let _ = bot
                                        .answer_callback_query(&query.id)
                                        .text("Invalid session ID")
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

            // Retry loop: if the dispatcher exits (network hiccup, Telegram conflict
            // from another process using the same token, etc.), wait and reconnect.
            // Without this, daemon mode silently loses the Telegram connection forever.
            loop {
                tracing::info!("Telegram: starting dispatcher polling loop");
                Dispatcher::builder(bot.clone(), tree.clone())
                    .build()
                    .dispatch()
                    .await;
                tracing::warn!("Telegram: dispatcher exited unexpectedly — reconnecting in 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        })
    }
}
