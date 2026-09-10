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

            let agent = self.agent_service;
            let session_svc = self.session_service;
            let shared_session = self.shared_session_id;
            let discord_state = self.discord_state;
            let config_rx = self.config_rx;
            let channel_msg_repo = self.channel_msg_repo;

            let intents = GatewayIntents::GUILD_MESSAGES
                | GatewayIntents::DIRECT_MESSAGES
                | GatewayIntents::MESSAGE_CONTENT;

            let make_handler = || Handler {
                agent: agent.clone(),
                session_svc: session_svc.clone(),
                extra_sessions: extra_sessions.clone(),
                shared_session: shared_session.clone(),
                discord_state: discord_state.clone(),
                config_rx: config_rx.clone(),
                channel_msg_repo: channel_msg_repo.clone(),
            };

            let mut client = match Client::builder(&token, intents)
                .event_handler(make_handler())
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Discord: failed to create client: {}", e);
                    return;
                }
            };

            // Retry loop: if the gateway connection drops (network hiccup, Discord
            // server restart, etc.), wait and reconnect instead of dying silently.
            loop {
                tracing::info!("Discord: starting gateway connection");
                if let Err(e) = client.start().await {
                    tracing::error!("Discord: client error: {} — reconnecting in 5s", e);
                } else {
                    tracing::warn!("Discord: client exited unexpectedly — reconnecting in 5s");
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                client = match Client::builder(&token, intents)
                    .event_handler(make_handler())
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Discord: failed to rebuild client: {}", e);
                        return;
                    }
                };
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
    async fn reaction_add(&self, ctx: Context, reaction: serenity::model::channel::Reaction) {
        let agent = self.agent.clone();
        let session_svc = self.session_svc.clone();
        let discord_state = self.discord_state.clone();
        let config_rx = self.config_rx.clone();
        tokio::spawn(async move {
            super::reactions::handle_reaction_add(
                &ctx,
                &reaction,
                agent,
                session_svc,
                discord_state,
                config_rx,
            )
            .await;
        });
    }

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
            self.shared_session.clone(),
            self.discord_state.clone(),
            self.config_rx.clone(),
            self.channel_msg_repo.clone(),
        )
        .await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // Modal submissions (#383): route the filled fields back as a turn.
        if let Interaction::Modal(modal) = &interaction {
            let custom_id = modal.data.custom_id.clone();
            if let Some(form_id) = custom_id.strip_prefix("formsub:") {
                let Some(spec) = self.discord_state.take_form(form_id).await else {
                    let _ack = modal
                        .create_response(
                            &ctx.http,
                            serenity::builder::CreateInteractionResponse::Acknowledge,
                        )
                        .await;
                    return;
                };
                // Collect input values in field order.
                use serenity::model::application::ActionRowComponent;
                let mut values: Vec<String> = Vec::new();
                for row in &modal.data.components {
                    for comp in &row.components {
                        if let ActionRowComponent::InputText(input) = comp {
                            values.push(input.value.clone().unwrap_or_default());
                        }
                    }
                }
                let filled: Vec<String> = spec
                    .fields
                    .iter()
                    .zip(values.iter())
                    .map(|((label, _), v)| format!("{label}: {v}"))
                    .collect();
                let user = modal.user.id.get();
                let user_name = modal
                    .user
                    .global_name
                    .clone()
                    .unwrap_or_else(|| modal.user.name.clone());
                let is_dm = modal.guild_id.is_none();
                let channel_id = modal.channel_id.get();
                let _ack = modal
                    .create_response(
                        &ctx.http,
                        serenity::builder::CreateInteractionResponse::Acknowledge,
                    )
                    .await;
                let agent = self.agent.clone();
                let session_svc = self.session_svc.clone();
                let idle = self.config_rx.borrow().channels.discord.session_idle_hours;
                let title = spec.title.clone();
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    super::interactions::route_interaction_turn(
                        &ctx2,
                        agent,
                        session_svc,
                        is_dm,
                        user,
                        channel_id,
                        idle,
                        format!(
                            "[{user_name} submitted the \"{title}\" form]\n{}",
                            filled.join("\n")
                        ),
                        format!("[System: {user_name} submitted the \"{title}\" form]"),
                    )
                    .await;
                });
                return;
            }
        }

        if let Some(comp) = interaction.message_component() {
            let custom_id = comp.data.custom_id.as_str();
            tracing::info!("Discord callback received: custom_id={}", custom_id);

            // Optional follow-up suggestion tapped (#598): inject the chosen
            // suggestion as the user's next message (a fresh turn). Options were
            // stashed under `followup:<id>:<idx>` via the TTL-bounded select map.
            if let Some(rest) = custom_id.strip_prefix(super::suggest_options::FOLLOWUP_PREFIX) {
                let ttl = self.config_rx.borrow().channels.discord.component_ttl_hours;
                let picked: Option<String> = if let Some((id, idx_str)) = rest.rsplit_once(':') {
                    match (
                        self.discord_state.take_select(id, ttl).await,
                        idx_str.parse::<usize>(),
                    ) {
                        (Some(opts), Ok(idx)) => opts.get(idx).cloned(),
                        _ => None,
                    }
                } else {
                    None
                };
                use serenity::builder::{
                    CreateInteractionResponse, CreateInteractionResponseMessage,
                };
                let Some(choice) = picked else {
                    let _e = comp
                        .create_response(
                            &ctx.http,
                            CreateInteractionResponse::UpdateMessage(
                                CreateInteractionResponseMessage::new()
                                    .content("⌛ These suggestions expired.")
                                    .components(Vec::new()),
                            ),
                        )
                        .await;
                    return;
                };
                let user = comp.user.id.get();
                let is_dm = comp.guild_id.is_none();
                let channel_id = comp.channel_id.get();
                let display_tag = comp
                    .user
                    .global_name
                    .clone()
                    .unwrap_or_else(|| comp.user.name.clone());
                // Ack by disabling the buttons and echoing the pick.
                let _e = comp
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::UpdateMessage(
                            CreateInteractionResponseMessage::new()
                                .content(format!("\u{25b6}\u{fe0f} {choice}"))
                                .components(Vec::new()),
                        ),
                    )
                    .await;
                let agent = self.agent.clone();
                let session_svc = self.session_svc.clone();
                let idle = self.config_rx.borrow().channels.discord.session_idle_hours;
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    super::interactions::route_interaction_turn(
                        &ctx2,
                        agent,
                        session_svc,
                        is_dm,
                        user,
                        channel_id,
                        idle,
                        choice,
                        display_tag,
                    )
                    .await;
                });
                return;
            }

            // Select menu pick (#382), with lazy TTL (#386).
            if let Some(sel_id) = custom_id.strip_prefix("sel:") {
                let ttl = self.config_rx.borrow().channels.discord.component_ttl_hours;
                let options = self.discord_state.take_select(sel_id, ttl).await;
                use serenity::model::application::ComponentInteractionDataKind;
                let picked: Option<String> = match (&comp.data.kind, options) {
                    (ComponentInteractionDataKind::StringSelect { values }, Some(opts)) => values
                        .first()
                        .and_then(|v| v.parse::<usize>().ok())
                        .and_then(|i| opts.get(i).cloned()),
                    (_, None) => None,
                    _ => None,
                };
                let Some(choice) = picked else {
                    // Expired or unknown: say so and strip the dead menu.
                    use serenity::builder::{
                        CreateInteractionResponse, CreateInteractionResponseMessage,
                    };
                    let _e = comp
                        .create_response(
                            &ctx.http,
                            CreateInteractionResponse::UpdateMessage(
                                CreateInteractionResponseMessage::new()
                                    .content("⌛ This menu expired.")
                                    .components(Vec::new()),
                            ),
                        )
                        .await;
                    return;
                };
                let user = comp.user.id.get();
                let user_name = comp
                    .user
                    .global_name
                    .clone()
                    .unwrap_or_else(|| comp.user.name.clone());
                let is_dm = comp.guild_id.is_none();
                let channel_id = comp.channel_id.get();
                // Ack by disabling the menu and showing the pick.
                use serenity::builder::{
                    CreateInteractionResponse, CreateInteractionResponseMessage,
                };
                let _e = comp
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::UpdateMessage(
                            CreateInteractionResponseMessage::new()
                                .content(format!("✅ {user_name} picked: {choice}"))
                                .components(Vec::new()),
                        ),
                    )
                    .await;
                let agent = self.agent.clone();
                let session_svc = self.session_svc.clone();
                let idle = self.config_rx.borrow().channels.discord.session_idle_hours;
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    super::interactions::route_interaction_turn(
                        &ctx2,
                        agent,
                        session_svc,
                        is_dm,
                        user,
                        channel_id,
                        idle,
                        format!(
                            "[{user_name} picked \"{choice}\" from your select menu — \
                             continue accordingly]"
                        ),
                        format!("[System: {user_name} picked \"{choice}\"]"),
                    )
                    .await;
                });
                return;
            }

            // Form button (#383): open the modal, with lazy TTL (#386).
            if let Some(form_id) = custom_id.strip_prefix("form:") {
                let ttl = self.config_rx.borrow().channels.discord.component_ttl_hours;
                let Some(spec) = self.discord_state.get_form(form_id, ttl).await else {
                    use serenity::builder::{
                        CreateInteractionResponse, CreateInteractionResponseMessage,
                    };
                    let _e = comp
                        .create_response(
                            &ctx.http,
                            CreateInteractionResponse::UpdateMessage(
                                CreateInteractionResponseMessage::new()
                                    .content("⌛ This form expired.")
                                    .components(Vec::new()),
                            ),
                        )
                        .await;
                    return;
                };
                use serenity::builder::{
                    CreateActionRow, CreateInputText, CreateInteractionResponse, CreateModal,
                };
                use serenity::model::application::InputTextStyle;
                let rows: Vec<CreateActionRow> = spec
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, (label, multiline))| {
                        let style = if *multiline {
                            InputTextStyle::Paragraph
                        } else {
                            InputTextStyle::Short
                        };
                        let short_label: String = label.chars().take(45).collect();
                        CreateActionRow::InputText(CreateInputText::new(
                            style,
                            short_label,
                            format!("field:{i}"),
                        ))
                    })
                    .collect();
                let modal = CreateModal::new(format!("formsub:{form_id}"), spec.title.clone())
                    .components(rows);
                if let Err(e) = comp
                    .create_response(&ctx.http, CreateInteractionResponse::Modal(modal))
                    .await
                {
                    tracing::warn!("Discord: failed to open modal: {e}");
                }
                return;
            }

            // Provider picker callback → show models for that provider
            if let Some(mid_str) = custom_id.strip_prefix("toolgroup:") {
                // Expand/Collapse toggle (#380): flip stored state and
                // update THIS message via the interaction response (which
                // also acks the click).
                if let Ok(mid) = mid_str.parse::<u64>() {
                    if let Some(group) = self.discord_state.toggle_tool_group(mid).await {
                        use serenity::builder::{
                            CreateInteractionResponse, CreateInteractionResponseMessage,
                        };
                        let resp = CreateInteractionResponse::UpdateMessage(
                            CreateInteractionResponseMessage::new()
                                .content(super::tool_group::render_content(&group))
                                .components(super::tool_group::render_components(&group, mid)),
                        );
                        if let Err(e) = comp.create_response(&ctx.http, resp).await {
                            tracing::warn!("Discord: tool group toggle response failed: {e}");
                        }
                    } else {
                        tracing::debug!("Discord: tool group {mid} aged out — toggle ignored");
                        let _ack = comp
                            .create_response(
                                &ctx.http,
                                serenity::builder::CreateInteractionResponse::Acknowledge,
                            )
                            .await;
                    }
                }
                return;
            }

            if let Some(provider_name) = custom_id.strip_prefix("provider:") {
                let resp = crate::channels::commands::models_for_provider(provider_name).await;

                // Agent-handled providers (OpenRouter 300+ models, custom)
                if resp.agent_handled {
                    let session_id = *self.shared_session.lock().await;
                    let display = crate::channels::commands::provider_display_name(provider_name);
                    let config = crate::config::Config::current();
                    if let Ok(new_provider) =
                        crate::brain::provider::factory::create_provider_by_name(
                            &config,
                            provider_name,
                        )
                        .await
                    {
                        match session_id {
                            Some(sid) => self.agent.swap_provider_for_session(
                                sid,
                                new_provider.clone(),
                                new_provider.default_model().to_string(),
                            ),
                            None => self.agent.swap_provider(new_provider),
                        }
                    }
                    if !resp.current_model.is_empty() {
                        let _ = crate::channels::commands::switch_model(
                            &self.agent,
                            &resp.current_model,
                            session_id,
                            Some(provider_name),
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
                                    if let Err(e) = channel_id.say(&http, &r.content).await {
                                        tracing::warn!(error = %e, "failed to send Discord agent message");
                                    }
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
                                        let mut end = 79;
                                        while !label.is_char_boundary(end) {
                                            end -= 1;
                                        }
                                        format!("{}…", &label[..end])
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
                // Resolve session first so the provider swap pins to the
                // right per-session slot instead of leaking via the global.
                let session_id = *self.shared_session.lock().await;
                let mut provider_err: Option<String> = None;
                if let Some(pname) = provider_name {
                    match crate::config::Config::load() {
                        Ok(config) => {
                            match crate::brain::provider::factory::create_provider_by_name(
                                &config, pname,
                            )
                            .await
                            {
                                Ok(new_provider) => match session_id {
                                    Some(sid) => self.agent.swap_provider_for_session(
                                        sid,
                                        new_provider.clone(),
                                        new_provider.default_model().to_string(),
                                    ),
                                    None => self.agent.swap_provider(new_provider),
                                },
                                Err(e) => {
                                    provider_err = Some(format!(
                                        "Failed to create provider '{}': {}",
                                        pname, e
                                    ))
                                }
                            }
                        }
                        Err(e) => provider_err = Some(format!("Failed to load config: {}", e)),
                    }
                }
                let reply = if let Some(err) = provider_err {
                    format!("⚠️ {}", err)
                } else {
                    match crate::channels::commands::switch_model(
                        &self.agent,
                        model_name,
                        session_id,
                        provider_name,
                    )
                    .await
                    {
                        Ok(_) => format!("✅ Model switched to `{}`", model_name),
                        Err(e) => format!("⚠️ {}", e),
                    }
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

            // OC-01: the approval keyboard sits in a channel where any member
            // can press it, so re-check that the tapper is the owner before
            // acting. Without this a non-owner tap runs the pending tool, and a
            // YOLO tap persists auto-always for the whole instance. The session
            // switch branch above already re-checks; the tool-approval buttons
            // did not. Uses the canonical owner resolver, so an empty allowlist
            // is unconfigured (deny), not "everyone is owner".
            {
                let cfg = self.config_rx.borrow().clone();
                let caller = comp.user.id.get().to_string();
                let is_owner = crate::config::owner::is_owner(
                    &cfg.channels.discord.allowed_users,
                    &cfg.channels.discord.bot_owner,
                    &caller,
                );
                if !is_owner {
                    tracing::warn!(
                        "Discord: non-owner {} tapped '{}' — refused (OC-01)",
                        caller,
                        custom_id
                    );
                    let _ = comp
                        .create_response(
                            &ctx.http,
                            serenity::builder::CreateInteractionResponse::Message(
                                serenity::builder::CreateInteractionResponseMessage::new()
                                    .content("⛔ Only the owner can approve tool calls.")
                                    .ephemeral(true),
                            ),
                        )
                        .await;
                    return;
                }
            }

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
