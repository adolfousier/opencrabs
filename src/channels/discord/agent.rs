//! Discord Agent
//!
//! Agent struct and startup logic. Mirrors the Telegram/WhatsApp agent pattern.

use super::DiscordState;
use super::handler;
use crate::brain::agent::AgentService;
use crate::config::Config;
use crate::db::ChannelMessageRepository;
use crate::services::{ServiceContext, SessionService};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use serenity::async_trait;
use serenity::model::application::Interaction;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

/// Discord bot that forwards messages to the AgentService
pub struct DiscordAgent {
    agent_service: Arc<AgentService>,
    session_service: SessionService,
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    discord_state: Arc<DiscordState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    channel_msg_repo: ChannelMessageRepository,
}

impl DiscordAgent {
    pub fn new(
        agent_service: Arc<AgentService>,
        service_context: ServiceContext,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        discord_state: Arc<DiscordState>,
        config_rx: tokio::sync::watch::Receiver<Config>,
        channel_msg_repo: ChannelMessageRepository,
    ) -> Self {
        Self {
            agent_service,
            session_service: SessionService::new(service_context),
            shared_session_id,
            discord_state,
            config_rx,
            channel_msg_repo,
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

            let cfg = self.config_rx.borrow().clone();
            tracing::info!(
                "Starting Discord bot with {} allowed user(s), STT={}, TTS={}",
                cfg.channels.discord.allowed_users.len(),
                cfg.voice_config().stt_enabled,
                cfg.voice_config().tts_enabled,
            );

            let extra_sessions: Arc<Mutex<HashMap<u64, (Uuid, std::time::Instant)>>> =
                Arc::new(Mutex::new(HashMap::new()));

            let event_handler = Handler {
                agent: self.agent_service,
                session_svc: self.session_service,
                extra_sessions,
                shared_session: self.shared_session_id,
                discord_state: self.discord_state,
                config_rx: self.config_rx,
                channel_msg_repo: self.channel_msg_repo,
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
    extra_sessions: Arc<Mutex<HashMap<u64, (Uuid, std::time::Instant)>>>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    discord_state: Arc<DiscordState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    channel_msg_repo: ChannelMessageRepository,
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
            self.extra_sessions.clone(),
            self.shared_session.clone(),
            self.discord_state.clone(),
            self.config_rx.clone(),
            self.channel_msg_repo.clone(),
        )
        .await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Some(comp) = interaction.message_component() {
            let custom_id = comp.data.custom_id.as_str();
            tracing::info!("Discord callback received: custom_id={}", custom_id);

            // Provider picker callback → show models for that provider
            if let Some(provider_name) = custom_id.strip_prefix("provider:") {
                let resp = crate::channels::commands::models_for_provider(provider_name).await;

                // Agent-handled providers (OpenRouter 300+ models, custom)
                if resp.agent_handled {
                    let session_id = *self.shared_session.lock().await;
                    let display = crate::channels::commands::provider_display_name(provider_name);
                    if let Ok(config) = crate::config::Config::load()
                        && let Ok(new_provider) =
                            crate::brain::provider::factory::create_provider_by_name(
                                &config,
                                provider_name,
                            )
                    {
                        self.agent.swap_provider(new_provider);
                    }
                    if !resp.current_model.is_empty() {
                        let _ = crate::channels::commands::switch_model(
                            &self.agent,
                            &resp.current_model,
                            session_id,
                        )
                        .await;
                    }
                    let _ = comp
                        .create_response(
                            &ctx.http,
                            serenity::builder::CreateInteractionResponse::Acknowledge,
                        )
                        .await;
                    if let Some(sid) = session_id {
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
                                display,
                                resp.current_model,
                                if provider_name == "openrouter" {
                                    "openrouter"
                                } else {
                                    provider_name
                                }
                            )
                        };
                        let agent_clone = self.agent.clone();
                        let http = ctx.http.clone();
                        let channel_id = comp.channel_id;
                        tokio::spawn(async move {
                            match agent_clone.send_message(sid, prompt, None).await {
                                Ok(r) => {
                                    let _ = channel_id.say(&http, &r.content).await;
                                }
                                Err(e) => tracing::error!("Agent follow-up failed: {}", e),
                            }
                        });
                    }
                    return;
                }

                if resp.models.is_empty() {
                    let _ = comp
                        .create_response(
                            &ctx.http,
                            serenity::builder::CreateInteractionResponse::Message(
                                serenity::builder::CreateInteractionResponseMessage::new()
                                    .content("No models available for this provider.")
                                    .ephemeral(true),
                            ),
                        )
                        .await;
                    return;
                }
                use serenity::builder::{
                    CreateActionRow, CreateButton, CreateInteractionResponse,
                    CreateInteractionResponseMessage,
                };
                use serenity::model::application::ButtonStyle;
                let rows: Vec<CreateActionRow> = resp
                    .models
                    .chunks(5)
                    .take(5)
                    .map(|chunk| {
                        CreateActionRow::Buttons(
                            chunk
                                .iter()
                                .map(|m| {
                                    let label = if *m == resp.current_model {
                                        format!("✓ {}", m)
                                    } else {
                                        m.clone()
                                    };
                                    let label = if label.len() > 80 {
                                        format!("{}…", &label[..79])
                                    } else {
                                        label
                                    };
                                    CreateButton::new(format!("model:{}:{}", resp.provider_name, m))
                                        .label(label)
                                        .style(ButtonStyle::Secondary)
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let _ = comp
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content(&resp.text)
                                .components(rows)
                                .ephemeral(true),
                        ),
                    )
                    .await;
                return;
            }

            // Model switch callback (format: model:<provider>:<model>)
            if let Some(rest) = custom_id.strip_prefix("model:") {
                let (provider_name, model_name) = if let Some((p, m)) = rest.split_once(':') {
                    (Some(p), m)
                } else {
                    (None, rest)
                };
                if let Some(pname) = provider_name
                    && let Ok(config) = crate::config::Config::load()
                    && let Ok(new_provider) =
                        crate::brain::provider::factory::create_provider_by_name(&config, pname)
                {
                    self.agent.swap_provider(new_provider);
                }
                let session_id = *self.shared_session.lock().await;
                let reply = match crate::channels::commands::switch_model(
                    &self.agent,
                    model_name,
                    session_id,
                )
                .await
                {
                    Ok(ctx) => {
                        let _ = ctx;
                        format!("✅ Model switched to `{}`", model_name)
                    }
                    Err(e) => format!("⚠️ {}", e),
                };
                let _ = comp
                    .create_response(
                        &ctx.http,
                        serenity::builder::CreateInteractionResponse::Message(
                            serenity::builder::CreateInteractionResponseMessage::new()
                                .content(reply)
                                .ephemeral(true),
                        ),
                    )
                    .await;
                return;
            }

            // Session switch callback
            if let Some(session_id_str) = custom_id.strip_prefix("session:") {
                if let Ok(new_id) = session_id_str.parse::<Uuid>() {
                    let cfg = self.config_rx.borrow().clone();
                    let caller_id = comp.user.id.get();
                    let owner_id = cfg
                        .channels
                        .discord
                        .allowed_users
                        .first()
                        .and_then(|s| s.parse::<u64>().ok());
                    let is_owner = cfg.channels.discord.allowed_users.is_empty()
                        || owner_id == Some(caller_id);

                    if is_owner {
                        *self.shared_session.lock().await = Some(new_id);
                    } else {
                        self.extra_sessions
                            .lock()
                            .await
                            .insert(caller_id, (new_id, std::time::Instant::now()));
                    }
                    self.discord_state
                        .register_session_channel(new_id, comp.channel_id.get())
                        .await;
                    let display = match self.session_svc.get_session(new_id).await {
                        Ok(Some(s)) => s.title.unwrap_or_else(|| {
                            session_id_str[..8.min(session_id_str.len())].to_string()
                        }),
                        _ => session_id_str[..8.min(session_id_str.len())].to_string(),
                    };
                    let _ = comp
                        .create_response(
                            &ctx.http,
                            serenity::builder::CreateInteractionResponse::Message(
                                serenity::builder::CreateInteractionResponseMessage::new()
                                    .content(format!("✅ Switched to session `{}`", display))
                                    .ephemeral(true),
                            ),
                        )
                        .await;
                } else {
                    let _ = comp
                        .create_response(
                            &ctx.http,
                            serenity::builder::CreateInteractionResponse::Message(
                                serenity::builder::CreateInteractionResponseMessage::new()
                                    .content("Invalid session ID")
                                    .ephemeral(true),
                            ),
                        )
                        .await;
                }
                return;
            }

            let (approved, always, yolo, approval_id) =
                if let Some(id) = custom_id.strip_prefix("approve:") {
                    (true, false, false, id.to_string())
                } else if let Some(id) = custom_id.strip_prefix("always:") {
                    (true, true, false, id.to_string())
                } else if let Some(id) = custom_id.strip_prefix("yolo:") {
                    (true, true, true, id.to_string())
                } else if let Some(id) = custom_id.strip_prefix("deny:") {
                    (false, false, false, id.to_string())
                } else {
                    tracing::warn!("Discord: unknown interaction custom_id: {}", custom_id);
                    let _ = comp
                        .create_response(
                            &ctx.http,
                            serenity::builder::CreateInteractionResponse::Acknowledge,
                        )
                        .await;
                    return;
                };

            if yolo {
                crate::utils::persist_auto_always_policy();
            }

            let resolved = self
                .discord_state
                .resolve_pending_approval(&approval_id, approved, always)
                .await;
            tracing::info!(
                "Discord approval resolved: id={}, approved={}, always={}, found_pending={}",
                approval_id,
                approved,
                always,
                resolved
            );
            if !resolved {
                tracing::warn!(
                    "Discord: no pending approval for id={} — may have timed out or already resolved",
                    approval_id
                );
            }

            // Ack the interaction so Discord doesn't show "interaction failed"
            let _ = comp
                .create_response(
                    &ctx.http,
                    serenity::builder::CreateInteractionResponse::Acknowledge,
                )
                .await;
        }
    }
}
