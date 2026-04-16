//! Slack Message Handler
//!
//! Processes incoming Slack messages: text, allowlist enforcement,
//! session routing (owner shares TUI session, others get per-user sessions).
//!
//! Uses a module-level static for handler state because slack-morphism's
//! Socket Mode callbacks require plain function pointers (not closures).

use super::SlackState;
use crate::brain::agent::AgentService;
use crate::config::{Config, RespondTo};
use crate::db::ChannelMessageRepository;
use crate::db::models::ChannelMessage as DbChannelMessage;
use crate::services::SessionService;
use crate::utils::sanitize::redact_secrets;
use crate::utils::truncate_str;
use slack_morphism::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Socket Mode interaction callback — handles button clicks for tool approvals.
pub async fn on_interaction(
    event: SlackInteractionEvent,
    client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let SlackInteractionEvent::BlockActions(block_actions) = event {
        let state = match HANDLER_STATE.get() {
            Some(s) => s.clone(),
            None => {
                tracing::warn!("Slack: interaction received but HANDLER_STATE not initialized");
                return Ok(());
            }
        };

        if let Some(actions) = block_actions.actions {
            for action in actions {
                let action_id = action.action_id.0.as_str();
                tracing::info!("Slack callback received: action_id={}", action_id);

                // Provider picker callback → show models for that provider
                if let Some(provider_name) = action_id.strip_prefix("provider:") {
                    let resp = crate::channels::commands::models_for_provider(provider_name).await;
                    tracing::info!("Slack: showing models for provider {}", provider_name);
                    if let Some(ref channel) = block_actions.channel {
                        let token =
                            SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
                        let session = client.open_session(&token);

                        // Agent-handled providers (OpenRouter 300+ models, custom)
                        if resp.agent_handled {
                            let session_id = *state.shared_session.lock().await;
                            let display =
                                crate::channels::commands::provider_display_name(provider_name);
                            if let Ok(config) = crate::config::Config::load()
                                && let Ok(new_provider) =
                                    crate::brain::provider::factory::create_provider_by_name(
                                        &config,
                                        provider_name,
                                    )
                                    .await
                            {
                                state.agent.swap_provider(new_provider);
                            }
                            if !resp.current_model.is_empty() {
                                let _ = crate::channels::commands::switch_model(
                                    &state.agent,
                                    &resp.current_model,
                                    session_id,
                                )
                                .await;
                            }
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
                                let agent_clone = state.agent.clone();
                                let bot_token = state.current_bot_token();
                                let channel_id_clone = channel.id.clone();
                                let client_clone = client.clone();
                                tokio::spawn(async move {
                                    match agent_clone.send_message(sid, prompt, None).await {
                                        Ok(r) => {
                                            let token = SlackApiToken::new(
                                                SlackApiTokenValue::from(bot_token),
                                            );
                                            let session = client_clone.open_session(&token);
                                            let request = SlackApiChatPostMessageRequest::new(
                                                channel_id_clone,
                                                SlackMessageContent::new().with_text(r.content),
                                            );
                                            let _ = session.chat_post_message(&request).await;
                                        }
                                        Err(e) => tracing::error!("Agent follow-up failed: {}", e),
                                    }
                                });
                            }
                            continue;
                        }

                        let header = SlackBlock::Section(SlackSectionBlock::new().with_text(
                            SlackBlockText::MarkDown(SlackBlockMarkDownText::new(
                                resp.text.clone(),
                            )),
                        ));
                        let buttons: Vec<SlackActionBlockElement> = resp
                            .models
                            .iter()
                            .take(25)
                            .map(|m| {
                                let label = if *m == resp.current_model {
                                    format!("✓ {}", m)
                                } else {
                                    m.clone()
                                };
                                SlackActionBlockElement::Button(SlackBlockButtonElement::new(
                                    SlackActionId::new(format!(
                                        "model:{}:{}",
                                        resp.provider_name, m
                                    )),
                                    SlackBlockPlainTextOnly::from(SlackBlockPlainText::new(label)),
                                ))
                            })
                            .collect();
                        let mut blocks = vec![header];
                        for chunk in buttons.chunks(5) {
                            blocks
                                .push(SlackBlock::Actions(SlackActionsBlock::new(chunk.to_vec())));
                        }
                        let request = SlackApiChatPostMessageRequest::new(
                            channel.id.clone(),
                            SlackMessageContent::new().with_blocks(blocks),
                        );
                        let _ = session.chat_post_message(&request).await;
                    }
                    continue;
                }

                // Model switch callback (format: model:<provider>:<model>)
                if let Some(rest) = action_id.strip_prefix("model:") {
                    let (provider_name, model_name) = if let Some((p, m)) = rest.split_once(':') {
                        (Some(p), m)
                    } else {
                        (None, rest)
                    };
                    let mut provider_err: Option<String> = None;
                    if let Some(pname) = provider_name {
                        match crate::config::Config::load() {
                            Ok(config) => {
                                match crate::brain::provider::factory::create_provider_by_name(
                                    &config, pname,
                                )
                                .await
                                {
                                    Ok(new_provider) => state.agent.swap_provider(new_provider),
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
                        tracing::warn!("Slack: provider switch failed: {}", err);
                        format!("⚠️ {}", err)
                    } else {
                        let session_id = *state.shared_session.lock().await;
                        match crate::channels::commands::switch_model(
                            &state.agent,
                            model_name,
                            session_id,
                        )
                        .await
                        {
                            Ok(_) => {
                                tracing::info!("Slack: model switched to {}", model_name);
                                format!("✅ Model switched to `{}`", model_name)
                            }
                            Err(e) => {
                                tracing::warn!("Slack: model switch failed: {}", e);
                                format!("⚠️ {}", e)
                            }
                        }
                    };
                    if let Some(ref channel) = block_actions.channel {
                        let token =
                            SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
                        let session = client.open_session(&token);
                        let request = SlackApiChatPostMessageRequest::new(
                            channel.id.clone(),
                            SlackMessageContent::new().with_text(reply),
                        );
                        let _ = session.chat_post_message(&request).await;
                    }
                    continue;
                }

                // Session switch callback
                if let Some(session_id_str) = action_id.strip_prefix("session:") {
                    if let Ok(new_id) = session_id_str.parse::<Uuid>() {
                        let cfg = state.config_rx.borrow().clone();
                        let caller_id = block_actions
                            .user
                            .as_ref()
                            .map(|u| u.id.0.as_str())
                            .unwrap_or("");
                        let is_owner = cfg.channels.slack.allowed_users.is_empty()
                            || cfg
                                .channels
                                .slack
                                .allowed_users
                                .first()
                                .map(|a| a == caller_id)
                                .unwrap_or(false);

                        if is_owner {
                            *state.shared_session.lock().await = Some(new_id);
                        } else {
                            state
                                .extra_sessions
                                .lock()
                                .await
                                .insert(caller_id.to_string(), (new_id, std::time::Instant::now()));
                        }
                        if let Some(ref channel) = block_actions.channel {
                            state
                                .slack_state
                                .register_session_channel(new_id, channel.id.0.to_string())
                                .await;
                            let token = SlackApiToken::new(SlackApiTokenValue::from(
                                state.current_bot_token(),
                            ));
                            let session = client.open_session(&token);
                            let display = match state.session_svc.get_session(new_id).await {
                                Ok(Some(s)) => s.title.unwrap_or_else(|| {
                                    session_id_str[..8.min(session_id_str.len())].to_string()
                                }),
                                _ => session_id_str[..8.min(session_id_str.len())].to_string(),
                            };
                            let request = SlackApiChatPostMessageRequest::new(
                                channel.id.clone(),
                                SlackMessageContent::new()
                                    .with_text(format!("✅ Switched to session `{}`", display)),
                            );
                            let _ = session.chat_post_message(&request).await;
                        }
                    }
                    continue;
                }

                let (approved, always, yolo, id) =
                    if let Some(id) = action_id.strip_prefix("approve:") {
                        (true, false, false, id.to_string())
                    } else if let Some(id) = action_id.strip_prefix("always:") {
                        (true, true, false, id.to_string())
                    } else if let Some(id) = action_id.strip_prefix("yolo:") {
                        (true, true, true, id.to_string())
                    } else if let Some(id) = action_id.strip_prefix("deny:") {
                        (false, false, false, id.to_string())
                    } else {
                        tracing::warn!("Slack: unknown action_id: {}", action_id);
                        continue;
                    };
                if yolo {
                    crate::utils::persist_auto_always_policy();
                }
                let resolved = state
                    .slack_state
                    .resolve_pending_approval(&id, approved, always)
                    .await;
                tracing::info!(
                    "Slack approval resolved: id={}, approved={}, always={}, found_pending={}",
                    id,
                    approved,
                    always,
                    resolved
                );
                if !resolved {
                    tracing::warn!(
                        "Slack: no pending approval for id={} — may have timed out or already resolved",
                        id
                    );
                }
            }
        }
    }
    Ok(())
}

/// Global handler state — set once by the agent before starting the listener.
pub static HANDLER_STATE: OnceLock<Arc<HandlerState>> = OnceLock::new();

/// Shared state for the Slack message handler callbacks.
pub struct HandlerState {
    pub agent: Arc<AgentService>,
    pub session_svc: SessionService,
    pub extra_sessions: Arc<Mutex<HashMap<String, (Uuid, std::time::Instant)>>>,
    pub shared_session: Arc<Mutex<Option<Uuid>>>,
    pub slack_state: Arc<SlackState>,
    pub bot_token: String,
    pub bot_user_id: Option<String>,
    pub config_rx: tokio::sync::watch::Receiver<Config>,
    pub channel_msg_repo: ChannelMessageRepository,
    /// Dedup: recently seen message timestamps (Slack retries if ack is slow).
    /// Entries are pruned when they exceed 200.
    pub seen_ts: Mutex<HashSet<String>>,
}

impl HandlerState {
    /// Get the current bot token — prefers hot-reloaded config, falls back to startup token.
    pub fn current_bot_token(&self) -> String {
        self.config_rx
            .borrow()
            .channels
            .slack
            .token
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| self.bot_token.clone())
    }
}

/// Split a message into chunks that fit Slack's limit (conservative 3000 chars).
pub fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max_len).min(text.len());
        // Ensure end falls on a char boundary (back up if inside a multi-byte char)
        while end < text.len() && !text.is_char_boundary(end) {
            end -= 1;
        }
        let break_at = if end < text.len() {
            text[start..end]
                .rfind('\n')
                .filter(|&pos| pos > end - start - 200)
                .map(|pos| start + pos + 1)
                .unwrap_or(end)
        } else {
            end
        };
        chunks.push(&text[start..break_at]);
        start = break_at;
    }
    chunks
}

/// Socket Mode push event callback (function pointer — required by slack-morphism).
///
/// Returns immediately so Slack gets the ack within 3 s (prevents retries).
/// Actual processing is spawned as a background task.
/// Deduplicates by message timestamp to drop Slack retries.
pub async fn on_push_event(
    event: SlackPushEventCallback,
    client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::debug!("Slack: received push event");
    match event.event {
        SlackEventCallbackBody::Message(msg) => {
            let ts = msg.origin.ts.to_string();
            if !dedup_ts(&ts).await {
                return Ok(());
            }
            tracing::debug!(
                "Slack: message event from user={:?}, channel={:?}, bot_id={:?}",
                msg.sender.user,
                msg.origin.channel,
                msg.sender.bot_id
            );
            tokio::spawn(async move {
                handle_message(&msg, client, false).await;
            });
        }
        SlackEventCallbackBody::AppMention(mention) => {
            let ts = mention.origin.ts.to_string();
            if !dedup_ts(&ts).await {
                return Ok(());
            }
            tracing::info!(
                "Slack: app_mention from user={:?}, channel={:?}, text={:?}",
                mention.user,
                mention.channel,
                mention
                    .content
                    .text
                    .as_ref()
                    .map(|t| crate::utils::truncate_str(t, 80)),
            );
            // Convert app_mention into a SlackMessageEvent so handle_message can process it
            let msg = SlackMessageEvent {
                origin: SlackMessageOrigin {
                    ts: mention.origin.ts,
                    channel: Some(mention.channel),
                    channel_type: None,
                    thread_ts: mention.origin.thread_ts,
                    client_msg_id: None,
                },
                content: Some(mention.content),
                sender: SlackMessageSender {
                    user: Some(mention.user),
                    bot_id: None,
                    username: None,
                    display_as_bot: None,
                    user_profile: None,
                    bot_profile: None,
                },
                subtype: None,
                hidden: None,
                message: None,
                previous_message: None,
                deleted_ts: None,
            };
            tokio::spawn(async move {
                handle_message(&msg, client, true).await;
            });
        }
        other => {
            tracing::debug!(
                "Slack: unhandled event type: {:?}",
                std::any::type_name_of_val(&other)
            );
        }
    }
    Ok(())
}

/// Returns `true` if this is the first time we see this timestamp (proceed).
/// Returns `false` if it's a duplicate (skip).
async fn dedup_ts(ts: &str) -> bool {
    let state = match HANDLER_STATE.get() {
        Some(s) => s,
        None => return true,
    };
    let mut seen = state.seen_ts.lock().await;
    if !seen.insert(ts.to_string()) {
        tracing::debug!("Slack: dropping duplicate event ts={}", ts);
        return false;
    }
    // Prune if too large to avoid unbounded growth
    if seen.len() > 200 {
        seen.clear();
        seen.insert(ts.to_string());
    }
    true
}

/// Socket Mode error handler.
pub fn on_error(
    err: Box<dyn std::error::Error + Send + Sync>,
    _client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> HttpStatusCode {
    tracing::error!("Slack: socket mode error: {}", err);
    HttpStatusCode::OK
}

/// Handle an incoming Slack message event.
async fn handle_message(
    msg: &SlackMessageEvent,
    client: Arc<SlackHyperClient>,
    is_app_mention: bool,
) {
    let state = match HANDLER_STATE.get() {
        Some(s) => s.clone(),
        None => {
            tracing::error!("Slack: handler state not initialized");
            return;
        }
    };

    // Skip bot messages
    if msg.sender.bot_id.is_some() {
        tracing::debug!(
            "Slack: skipping bot message (bot_id={:?})",
            msg.sender.bot_id
        );
        return;
    }

    // Extract user ID
    let user_id = match &msg.sender.user {
        Some(uid) => uid.to_string(),
        None => {
            tracing::debug!("Slack: message has no sender user ID, ignoring");
            return;
        }
    };

    // Extract channel ID
    let channel_id = match &msg.origin.channel {
        Some(ch) => ch.to_string(),
        None => {
            tracing::debug!("Slack: message has no channel ID, ignoring");
            return;
        }
    };

    // Resolve user display name via Slack API (cached per conversation turn)
    let user_name = {
        let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
        let session = client.open_session(&token);
        match session
            .users_info(&SlackApiUsersInfoRequest::new(SlackUserId::new(
                user_id.clone(),
            )))
            .await
        {
            Ok(resp) => resp
                .user
                .profile
                .as_ref()
                .and_then(|p| {
                    p.display_name
                        .clone()
                        .filter(|n| !n.is_empty())
                        .or_else(|| p.real_name.clone())
                })
                .unwrap_or_else(|| user_id.clone()),
            Err(e) => {
                tracing::debug!("Slack: failed to resolve user name for {}: {}", user_id, e);
                user_id.clone()
            }
        }
    };

    // Resolve channel name via Slack API
    let channel_name = {
        let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
        let session = client.open_session(&token);
        match session
            .conversations_info(&SlackApiConversationsInfoRequest::new(SlackChannelId::new(
                channel_id.clone(),
            )))
            .await
        {
            Ok(resp) => resp
                .channel
                .name
                .map(|n| format!("#{n}"))
                .unwrap_or_else(|| channel_id.clone()),
            Err(e) => {
                tracing::debug!(
                    "Slack: failed to resolve channel name for {}: {}",
                    channel_id,
                    e
                );
                channel_id.clone()
            }
        }
    };

    // Extract text (may be empty if user sent files only)
    let text = msg
        .content
        .as_ref()
        .and_then(|c| c.text.clone())
        .unwrap_or_default();

    // Check for files
    let files: Vec<_> = msg
        .content
        .as_ref()
        .and_then(|c| c.files.as_ref())
        .map(|v| v.as_slice())
        .unwrap_or(&[])
        .to_vec();

    // Require at least text or files
    if text.is_empty() && files.is_empty() {
        tracing::debug!("Slack: message has no text and no files, ignoring");
        return;
    }

    // Helper: passively capture a channel message for history
    let store_channel_msg = |text: String| {
        let repo = state.channel_msg_repo.clone();
        let ch_id = channel_id.clone();
        let uid = user_id.clone();
        let uname = user_name.clone();
        let ch_name = channel_name.clone();
        async move {
            if text.is_empty() {
                return;
            }
            let cm = DbChannelMessage::new(
                "slack".into(),
                ch_id,
                Some(ch_name),
                uid,
                uname,
                text,
                "text".into(),
                None,
            );
            if let Err(e) = repo.insert(&cm).await {
                tracing::warn!("Failed to store Slack channel message: {e}");
            }
        }
    };

    // Read latest config from watch channel — single source of truth
    let cfg = state.config_rx.borrow().clone();
    let sl_cfg = &cfg.channels.slack;
    let allowed: HashSet<String> = sl_cfg.allowed_users.iter().cloned().collect();
    let respond_to = &sl_cfg.respond_to;
    let allowed_channels: HashSet<String> = sl_cfg.allowed_channels.iter().cloned().collect();
    let idle_timeout_hours = sl_cfg.session_idle_hours;
    let voice_config = cfg.voice_config();

    // Allowlist check — if allowed list is empty, accept all
    if !allowed.is_empty() && !allowed.contains(&user_id) {
        tracing::debug!("Slack: ignoring message from non-allowed user {}", user_id);
        return;
    }

    // respond_to / allowed_channels filtering — DMs (channel starts with 'D') always pass
    let is_dm = channel_id.starts_with('D');
    if !is_dm {
        // Check allowed_channels (empty = all channels allowed)
        if !allowed_channels.is_empty() && !allowed_channels.contains(&channel_id) {
            tracing::debug!(
                "Slack: ignoring message in non-allowed channel {}",
                channel_id
            );
            store_channel_msg(text.clone()).await;
            return;
        }

        match respond_to {
            RespondTo::DmOnly => {
                tracing::debug!("Slack: respond_to=dm_only, ignoring channel message");
                store_channel_msg(text.clone()).await;
                return;
            }
            RespondTo::Mention => {
                // app_mention events are already verified by Slack — trust them
                let mentioned = is_app_mention
                    || if let Some(ref bid) = state.bot_user_id {
                        text.contains(&format!("<@{}>", bid))
                    } else {
                        text.contains("<@U")
                    };
                if !mentioned {
                    tracing::debug!(
                        "Slack: respond_to=mention, bot not mentioned — ignoring (bot_user_id={:?}, text={:?})",
                        state.bot_user_id,
                        crate::utils::truncate_str(&text, 120),
                    );
                    store_channel_msg(text.clone()).await;
                    return;
                }
            }
            RespondTo::All => {} // pass through
        }
    }

    // Also store directed channel messages for complete history
    if !is_dm {
        store_channel_msg(text.clone()).await;
    }

    // Strip <@BOT_ID> from text when responding to a mention
    let text = if !is_dm && *respond_to == RespondTo::Mention {
        if let Some(ref bid) = state.bot_user_id {
            text.replace(&format!("<@{}>", bid), "").trim().to_string()
        } else {
            // bot_user_id unknown — strip any <@U...> mention tag
            let re = regex::Regex::new(r"<@U[A-Z0-9]+>").unwrap();
            re.replace_all(&text, "").trim().to_string()
        }
    } else {
        text
    };

    let text_preview = truncate_str(&text, 50);
    tracing::info!("Slack: message from {}: {}", user_id, text_preview);

    // Track owner's channel for proactive messaging
    let is_owner = allowed.is_empty()
        || allowed
            .iter()
            .next()
            .map(|a| *a == user_id)
            .unwrap_or(false);

    if is_owner {
        state
            .slack_state
            .set_owner_channel(channel_id.clone())
            .await;
    }

    // Resolve session: owner DM shares TUI session; channels/groups get per-channel sessions
    let session_id = if is_owner && is_dm {
        let shared = state.shared_session.lock().await;
        match *shared {
            Some(id) => id,
            None => {
                drop(shared);
                // Resume most recent session from DB (survives daemon restarts)
                let restored = match state.session_svc.get_most_recent_session().await {
                    Ok(Some(s)) => {
                        tracing::info!("Slack: restored most recent session {}", s.id);
                        Some(s.id)
                    }
                    _ => None,
                };
                let id = match restored {
                    Some(id) => id,
                    None => {
                        tracing::info!("Slack: no existing session, creating one for owner");
                        match crate::channels::session_init::create_channel_session(
                            &state.session_svc,
                            Some("Chat".to_string()),
                        )
                        .await
                        {
                            Ok(session) => session.id,
                            Err(e) => {
                                tracing::error!("Slack: failed to create session: {}", e);
                                return;
                            }
                        }
                    }
                };
                *state.shared_session.lock().await = Some(id);
                id
            }
        }
    } else {
        // Non-DM-owner sessions: keyed by channel_id for channels (shared per channel),
        // by user_id for DMs (separate per user).
        // Persisted in DB by title — survives restarts.
        let session_title = if is_dm {
            format!("Slack: {}", user_id)
        } else {
            format!("Slack: #{}", channel_id)
        };

        // Look up existing session from DB
        let existing = state
            .session_svc
            .find_session_by_title(&session_title)
            .await
            .ok()
            .flatten();

        if let Some(session) = existing {
            // Check idle timeout
            if idle_timeout_hours.is_some_and(|h| {
                let elapsed = (chrono::Utc::now() - session.updated_at).num_seconds();
                elapsed > (h * 3600.0) as i64
            }) {
                if let Err(e) = state.session_svc.archive_session(session.id).await {
                    tracing::error!("Slack: failed to archive session {}: {}", session.id, e);
                }
                match crate::channels::session_init::create_channel_session(
                    &state.session_svc,
                    Some(session_title),
                )
                .await
                {
                    Ok(new_session) => new_session.id,
                    Err(e) => {
                        tracing::error!("Slack: failed to create session: {}", e);
                        return;
                    }
                }
            } else {
                session.id
            }
        } else {
            match crate::channels::session_init::create_channel_session(
                &state.session_svc,
                Some(session_title),
            )
            .await
            {
                Ok(session) => {
                    tracing::info!(
                        "Slack: created new channel session {} for {}",
                        session.id,
                        if is_dm { &user_id } else { &channel_id }
                    );
                    session.id
                }
                Err(e) => {
                    tracing::error!("Slack: failed to create session: {}", e);
                    return;
                }
            }
        }
    };

    // Process attached files — images as <<IMG:tmp_path>>, text files extracted inline
    let mut content = text.clone();
    if !files.is_empty() {
        use crate::utils::{inject_file_content, process_file_with_vision};
        let cfg = match crate::config::Config::load() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Slack: failed to load config: {}", e);
                return;
            }
        };
        let http = reqwest::Client::new();
        for file in &files {
            let mime = file.mimetype.as_ref().map(|m| m.0.as_str()).unwrap_or("");
            let fname = file.name.as_deref().unwrap_or("file");

            // Download file using bot token (Slack private URLs require auth)
            let dl_url = match file
                .url_private_download
                .as_ref()
                .or(file.url_private.as_ref())
            {
                Some(u) => u.to_string(),
                None => continue,
            };
            let dl_bytes = match http
                .get(&dl_url)
                .header(
                    "Authorization",
                    format!("Bearer {}", state.current_bot_token()),
                )
                .send()
                .await
            {
                Ok(resp) => match resp.bytes().await {
                    Ok(b) => b.to_vec(),
                    Err(e) => {
                        tracing::error!("Slack: failed to read file bytes: {e}");
                        continue;
                    }
                },
                Err(e) => {
                    tracing::error!("Slack: failed to download file {}: {e}", fname);
                    continue;
                }
            };

            // Audio → STT
            if mime.starts_with("audio/") {
                if voice_config.stt_enabled {
                    match crate::channels::voice::transcribe(dl_bytes, &voice_config).await {
                        Ok(transcript) => {
                            tracing::info!(
                                "Slack: transcribed audio: {}",
                                truncate_str(&transcript, 80)
                            );
                            if content.is_empty() {
                                content = transcript;
                            } else {
                                content.push_str(&format!("\n\n[Transcription]: {transcript}"));
                            }
                        }
                        Err(e) => tracing::error!("Slack: STT error: {e}"),
                    }
                }
                continue;
            }

            let fc = process_file_with_vision(&dl_bytes, mime, fname, &cfg);
            let injected = inject_file_content(&fc).0;
            if !injected.is_empty() {
                if content.is_empty() {
                    content = injected;
                } else {
                    content.push_str(&format!("\n\n{injected}"));
                }
            }
        }
    }

    if content.is_empty() {
        tracing::debug!("Slack: no processable content after file handling, ignoring");
        return;
    }

    // Restore session's own provider (each session keeps its provider independently)
    let session_meta = state
        .session_svc
        .get_session(session_id)
        .await
        .ok()
        .flatten();
    crate::channels::commands::sync_provider_for_session(
        &state.agent,
        session_meta
            .as_ref()
            .and_then(|s| s.provider_name.as_deref()),
        session_meta.as_ref().and_then(|s| s.model.as_deref()),
    )
    .await;

    // ── Channel commands (/help, /usage, /models) ──────────────────────────
    {
        use crate::channels::commands::{self, ChannelCommand};
        let cmd =
            commands::handle_command(&content, session_id, &state.agent, &state.session_svc).await;

        // Handle simple text-response commands (Help, Usage, Evolve, Doctor, etc.)
        if let Some(reply) = commands::try_execute_text_command(&cmd).await {
            let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
            let session = client.open_session(&token);
            let request = SlackApiChatPostMessageRequest::new(
                SlackChannelId::new(channel_id),
                SlackMessageContent::new().with_text(reply),
            );
            let _ = session.chat_post_message(&request).await;
            return;
        }

        match cmd {
            ChannelCommand::Models(resp) => {
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
                let session = client.open_session(&token);
                let header = SlackBlock::Section(SlackSectionBlock::new().with_text(
                    SlackBlockText::MarkDown(SlackBlockMarkDownText::new(resp.text.clone())),
                ));
                let buttons: Vec<SlackActionBlockElement> = resp
                    .providers
                    .iter()
                    .take(25)
                    .map(|(name, label)| {
                        let display = if *name == resp.current_provider {
                            format!("✓ {}", label)
                        } else {
                            label.clone()
                        };
                        SlackActionBlockElement::Button(SlackBlockButtonElement::new(
                            SlackActionId::new(format!("provider:{}", name)),
                            SlackBlockPlainTextOnly::from(SlackBlockPlainText::new(display)),
                        ))
                    })
                    .collect();
                let mut blocks = vec![header];
                for chunk in buttons.chunks(5) {
                    blocks.push(SlackBlock::Actions(SlackActionsBlock::new(chunk.to_vec())));
                }
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id),
                    SlackMessageContent::new().with_blocks(blocks),
                );
                let _ = session.chat_post_message(&request).await;
                return;
            }
            ChannelCommand::NewSession => {
                // Archive the current channel session before creating a new one
                let session_title = if is_dm {
                    format!("Slack: {}", user_id)
                } else {
                    format!("Slack: #{}", channel_id)
                };
                if let Ok(Some(old)) = state
                    .session_svc
                    .find_session_by_title(&session_title)
                    .await
                    && let Err(e) = state.session_svc.archive_session(old.id).await
                {
                    tracing::error!("Slack: failed to archive old session {}: {}", old.id, e);
                }
                match crate::channels::session_init::create_channel_session(
                    &state.session_svc,
                    Some(session_title),
                )
                .await
                {
                    Ok(new_session) => {
                        if is_owner && is_dm {
                            *state.shared_session.lock().await = Some(new_session.id);
                        }
                        state
                            .slack_state
                            .register_session_channel(new_session.id, channel_id.clone())
                            .await;
                        let token =
                            SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
                        let session = client.open_session(&token);
                        let request = SlackApiChatPostMessageRequest::new(
                            SlackChannelId::new(channel_id),
                            SlackMessageContent::new()
                                .with_text("✅ New session started.".to_string()),
                        );
                        let _ = session.chat_post_message(&request).await;
                    }
                    Err(e) => {
                        tracing::error!("Slack: failed to create session: {}", e);
                        let token =
                            SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
                        let session = client.open_session(&token);
                        let request = SlackApiChatPostMessageRequest::new(
                            SlackChannelId::new(channel_id),
                            SlackMessageContent::new()
                                .with_text("Failed to create session.".to_string()),
                        );
                        let _ = session.chat_post_message(&request).await;
                    }
                }
                return;
            }
            ChannelCommand::Sessions(resp) => {
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
                let session = client.open_session(&token);
                let header = SlackBlock::Section(SlackSectionBlock::new().with_text(
                    SlackBlockText::MarkDown(SlackBlockMarkDownText::new(resp.text.clone())),
                ));
                let buttons: Vec<SlackActionBlockElement> = resp
                    .sessions
                    .iter()
                    .take(25)
                    .map(|(id, label)| {
                        let display = if *id == resp.current_session_id {
                            format!("✓ {}", label)
                        } else {
                            label.clone()
                        };
                        SlackActionBlockElement::Button(SlackBlockButtonElement::new(
                            SlackActionId::new(format!("session:{}", id)),
                            SlackBlockPlainTextOnly::from(SlackBlockPlainText::new(display)),
                        ))
                    })
                    .collect();
                let mut blocks = vec![header];
                for chunk in buttons.chunks(5) {
                    blocks.push(SlackBlock::Actions(SlackActionsBlock::new(chunk.to_vec())));
                }
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id),
                    SlackMessageContent::new().with_blocks(blocks),
                );
                let _ = session.chat_post_message(&request).await;
                return;
            }
            ChannelCommand::Stop => {
                let cancelled = state.slack_state.cancel_session(session_id).await;
                let reply = if cancelled {
                    "Operation cancelled."
                } else {
                    "No operation in progress."
                };
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
                let session = client.open_session(&token);
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id),
                    SlackMessageContent::new().with_text(reply.to_string()),
                );
                let _ = session.chat_post_message(&request).await;
                return;
            }
            ChannelCommand::Compact => {
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
                let session = client.open_session(&token);
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id.clone()),
                    SlackMessageContent::new().with_text("⏳ Compacting context...".to_string()),
                );
                let _ = session.chat_post_message(&request).await;
                content =
                    "[SYSTEM: Compact context now. Summarize this conversation for continuity.]"
                        .to_string();
            }
            ChannelCommand::UserPrompt(prompt) => {
                content = prompt;
                // fall through to agent with the prompt as the message
            }
            ChannelCommand::NotACommand => {}
            // Help, Usage, Evolve, Doctor, UserSystem handled by try_execute_text_command above
            _ => {}
        }
    }

    // Detect thread replies so the agent knows the message is in a thread context.
    // Also store the thread_ts so we reply in the same thread.
    let thread_ts = msg.origin.thread_ts.clone();
    let reply_context = thread_ts
        .as_ref()
        .map(|ts| format!("[Replying in thread (thread_ts: {ts})]"));

    // For non-owner users, prepend sender identity so the agent knows who
    // it's talking to and doesn't assume it's the owner.
    let agent_input = if !is_owner {
        if is_dm {
            format!("[Slack DM from {user_name} ({user_id})]\n{content}")
        } else {
            format!("[Slack message from {user_name} ({user_id}) in {channel_name}]\n{content}")
        }
    } else {
        content
    };

    // Prepend reply/thread context if the message is in a thread.
    let agent_input = if let Some(ref ctx) = reply_context {
        format!("{ctx}\n{agent_input}")
    } else {
        agent_input
    };

    // Inject recent channel history so the agent has full conversation context.
    let agent_input = if !is_dm {
        match state
            .channel_msg_repo
            .recent(Some("slack"), &channel_id, 30)
            .await
        {
            Ok(messages) if !messages.is_empty() => {
                let history: Vec<String> = messages
                    .iter()
                    .rev()
                    .map(|m| {
                        let ts = m.created_at.format("%H:%M");
                        format!("[{}] {}: {}", ts, m.sender_name, m.content)
                    })
                    .collect();
                format!(
                    "[Recent channel history ({} messages):\n{}\n--- end history ---]\n{}",
                    history.len(),
                    history.join("\n"),
                    agent_input
                )
            }
            _ => agent_input,
        }
    } else {
        agent_input
    };

    // Tell the LLM its text response is automatically delivered to the chat,
    // so it should NOT use slack_send for simple text replies.
    let agent_input = format!(
        "[Channel: Slack — your text response is automatically sent to this channel. \
         Do NOT call slack_send to deliver your answer. Only use slack_send for: \
         sending to a different channel, threads, blocks, reactions, files, or moderation.]\n{agent_input}"
    );

    // Register channel for approval routing, then send with approval callback
    state
        .slack_state
        .register_session_channel(session_id, channel_id.clone())
        .await;
    let approval_cb = make_approval_callback(state.slack_state.clone());

    let cancel_token = tokio_util::sync::CancellationToken::new();
    state
        .slack_state
        .store_cancel_token(session_id, cancel_token.clone())
        .await;

    // Post a "thinking" placeholder so the user knows we're processing
    let thinking_ts: Arc<Mutex<Option<SlackTs>>> = Arc::new(Mutex::new(None));
    {
        let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
        let session = client.open_session(&token);
        let mut req = SlackApiChatPostMessageRequest::new(
            SlackChannelId::new(channel_id.clone()),
            SlackMessageContent::new().with_text("_thinking..._".to_string()),
        );
        if let Some(ref ts) = thread_ts {
            req = req.with_thread_ts(ts.clone());
        }
        if let Ok(resp) = session.chat_post_message(&req).await {
            *thinking_ts.lock().await = Some(resp.ts);
        }
    }

    // Track sent intermediate messages for dedup against final response
    let sent_intermediates: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sent_intermediates_final = sent_intermediates.clone();

    // Build progress callback — sends tool call status as Slack messages
    let progress_cb: crate::brain::agent::ProgressCallback = {
        use crate::brain::agent::ProgressEvent;

        struct ToolEntry {
            msg_ts: Option<SlackTs>,
            name: String,
            context: String,
        }

        let tools: Arc<Mutex<Vec<ToolEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let bot_token_cb = state.current_bot_token();
        let channel_cb = SlackChannelId::new(channel_id.clone());
        let client_cb = client.clone();
        let thinking_ts_cb = thinking_ts.clone();
        let thread_ts_cb = thread_ts.clone();

        Arc::new(move |_session_id, event| {
            let tools = tools.clone();
            let sent = sent_intermediates.clone();
            let token = SlackApiToken::new(SlackApiTokenValue::from(bot_token_cb.clone()));
            let channel = channel_cb.clone();
            let client = client_cb.clone();
            let thread_ts_inner = thread_ts_cb.clone();

            match event {
                ProgressEvent::ToolStarted {
                    tool_name,
                    tool_input,
                } => {
                    let thinking_ts = thinking_ts_cb.clone();
                    let ctx = crate::utils::tool_context_hint(&tool_name, &tool_input);
                    tokio::spawn(async move {
                        let session = client.open_session(&token);
                        // Delete the "thinking..." placeholder on first tool call
                        if let Some(ts) = thinking_ts.lock().await.take() {
                            let del = SlackApiChatDeleteRequest::new(channel.clone(), ts);
                            let _ = session.chat_delete(&del).await;
                        }
                        let text = format!("⚙️ *{}*{}", tool_name, ctx);
                        let mut req = SlackApiChatPostMessageRequest::new(
                            channel,
                            SlackMessageContent::new().with_text(text),
                        );
                        if let Some(ref ts) = thread_ts_inner {
                            req = req.with_thread_ts(ts.clone());
                        }
                        if let Ok(resp) = session.chat_post_message(&req).await {
                            let mut t = tools.lock().await;
                            t.push(ToolEntry {
                                msg_ts: Some(resp.ts),
                                name: tool_name,
                                context: ctx,
                            });
                        }
                    });
                }
                ProgressEvent::ToolCompleted {
                    tool_name, success, ..
                } => {
                    tokio::spawn(async move {
                        let session = client.open_session(&token);
                        let mut t = tools.lock().await;
                        if let Some(entry) = t
                            .iter_mut()
                            .rev()
                            .find(|e| e.name == tool_name && e.msg_ts.is_some())
                        {
                            let icon = if success { "✅" } else { "❌" };
                            let text = format!("{} *{}*{}", icon, entry.name, entry.context);
                            if let Some(ts) = entry.msg_ts.take() {
                                let upd = SlackApiChatUpdateRequest::new(
                                    channel,
                                    SlackMessageContent::new().with_text(text),
                                    ts,
                                );
                                let _ = session.chat_update(&upd).await;
                            }
                        }
                    });
                }
                ProgressEvent::SelfHealingAlert { message } => {
                    let thread_ts_heal = thread_ts_inner.clone();
                    tokio::spawn(async move {
                        let session = client.open_session(&token);
                        let text = format!("🔧 {}", message);
                        let mut req = SlackApiChatPostMessageRequest::new(
                            channel,
                            SlackMessageContent::new().with_text(text),
                        );
                        if let Some(ref ts) = thread_ts_heal {
                            req = req.with_thread_ts(ts.clone());
                        }
                        let _ = session.chat_post_message(&req).await;
                    });
                }
                ProgressEvent::IntermediateText { text, .. } => {
                    let thread_ts_resp = thread_ts_inner.clone();
                    let sent_ref = sent.clone();
                    tokio::spawn(async move {
                        let session = client.open_session(&token);
                        let text_fmt = crate::utils::slack_fmt::markdown_to_mrkdwn(&text);
                        let mut req = SlackApiChatPostMessageRequest::new(
                            channel,
                            SlackMessageContent::new().with_text(text_fmt),
                        );
                        if let Some(ref ts) = thread_ts_resp {
                            req = req.with_thread_ts(ts.clone());
                        }
                        if let Err(e) = session.chat_post_message(&req).await {
                            tracing::debug!("Slack: failed to send intermediate text: {}", e);
                        } else {
                            sent_ref.lock().await.push(text);
                        }
                    });
                }
                _ => {}
            }
        })
    };

    let result = state
        .agent
        .send_message_with_tools_and_callback(
            session_id,
            agent_input,
            None,
            Some(cancel_token),
            Some(approval_cb),
            Some(progress_cb),
            "slack",
            Some(&channel_id),
        )
        .await;

    state.slack_state.remove_cancel_token(session_id).await;

    // Delete the "thinking..." placeholder if it's still around
    {
        let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
        let session = client.open_session(&token);
        if let Some(ts) = thinking_ts.lock().await.take() {
            let del = SlackApiChatDeleteRequest::new(SlackChannelId::new(channel_id.clone()), ts);
            let _ = session.chat_delete(&del).await;
        }
    }

    match result {
        Ok(response) => {
            // Extract <<IMG:path>> markers — upload each as a Slack file.
            let (text_only, img_paths) = crate::utils::extract_img_markers(&response.content);
            let text_only = crate::utils::sanitize::strip_llm_artifacts(&text_only);
            let text_only = redact_secrets(&text_only);

            // Dedup: strip text that was already sent as intermediate messages
            // to avoid duplicating content on Slack.
            let sent = sent_intermediates_final.lock().await.clone();
            tracing::info!(
                "Slack dedup: response.content len={}, sent_intermediates count={}",
                text_only.len(),
                sent.len()
            );
            let text_only = if !sent.is_empty() {
                let mut remaining = text_only.clone();
                for intermediate in &sent {
                    remaining = remaining.replace(intermediate.as_str(), "");
                }
                let result = remaining.trim().to_string();
                if result != text_only {
                    tracing::info!(
                        "Slack dedup: stripped {} chars, remaining len={}",
                        text_only.len() - result.len(),
                        result.len()
                    );
                } else {
                    tracing::warn!(
                        "Slack dedup: NO MATCH — none of {} intermediates found in response",
                        sent.len()
                    );
                }
                // If dedup stripped everything, the intermediates already
                // cover the full response — nothing new to send. Safe here
                // because Slack's intermediate handler only pushes to
                // sent_intermediates on confirmed successful post.
                if result.is_empty() {
                    tracing::info!(
                        "Slack dedup: result empty after stripping — intermediates already delivered full response (len={})",
                        text_only.len()
                    );
                    String::new()
                } else {
                    result
                }
            } else {
                tracing::info!("Slack dedup: no intermediates to strip");
                text_only
            };

            let text_only = crate::utils::slack_fmt::markdown_to_mrkdwn(&text_only);

            let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
            let session = client.open_session(&token);

            for img_path in img_paths {
                match tokio::fs::read(&img_path).await {
                    Ok(bytes) => {
                        let fname = std::path::Path::new(&img_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("image.png")
                            .to_string();
                        #[allow(deprecated)]
                        let req = SlackApiFilesUploadRequest {
                            channels: Some(vec![SlackChannelId::new(channel_id.clone())]),
                            binary_content: Some(bytes),
                            filename: Some(fname),
                            filetype: None,
                            content: None,
                            initial_comment: None,
                            thread_ts: None,
                            title: None,
                            file_content_type: Some("image/png".to_string()),
                        };
                        #[allow(deprecated)]
                        if let Err(e) = session.files_upload(&req).await {
                            tracing::error!("Slack: failed to upload generated image: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Slack: failed to read image {}: {}", img_path, e);
                    }
                }
            }

            for chunk in split_message(&text_only, 3000) {
                if chunk.is_empty() {
                    continue;
                }
                let mut request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id.clone()),
                    SlackMessageContent::new().with_text(chunk.to_string()),
                );
                if let Some(ref ts) = thread_ts {
                    request = request.with_thread_ts(ts.clone());
                }
                if let Err(e) = session.chat_post_message(&request).await {
                    tracing::error!("Slack: failed to send reply: {}", e);
                }
            }
        }
        Err(ref e) if matches!(e, crate::brain::agent::AgentError::Cancelled) => {
            tracing::info!("Slack: agent call cancelled for session {}", session_id);
        }
        Err(e) => {
            tracing::error!("Slack: agent error: {}", e);
            let token = SlackApiToken::new(SlackApiTokenValue::from(state.current_bot_token()));
            let session = client.open_session(&token);
            let error_msg = format!("Error: {}", e);
            let mut request = SlackApiChatPostMessageRequest::new(
                SlackChannelId::new(channel_id),
                SlackMessageContent::new().with_text(error_msg),
            );
            if let Some(ref ts) = thread_ts {
                request = request.with_thread_ts(ts.clone());
            }
            let _ = session.chat_post_message(&request).await;
        }
    }
}

/// Build an `ApprovalCallback` that sends a Slack Block Kit message with 3 buttons
/// (Yes / Always / No) and waits up to 5 min for a click.
pub(crate) fn make_approval_callback(
    state: Arc<super::SlackState>,
) -> crate::brain::agent::ApprovalCallback {
    use crate::brain::agent::ToolApprovalInfo;
    use crate::utils::{check_approval_policy, persist_auto_session_policy};
    use tokio::sync::oneshot;

    Arc::new(move |info: ToolApprovalInfo| {
        let state = state.clone();
        Box::pin(async move {
            if let Some(result) = check_approval_policy() {
                return Ok(result);
            }

            let client = match state.client().await {
                Some(c) => c,
                None => {
                    tracing::warn!("Slack approval: bot not connected");
                    return Ok((false, false));
                }
            };

            let bot_token = match state.bot_token().await {
                Some(t) => t,
                None => {
                    tracing::warn!("Slack approval: no bot token");
                    return Ok((false, false));
                }
            };

            let channel_id = match state.session_channel(info.session_id).await {
                Some(id) => id,
                None => match state.owner_channel_id().await {
                    Some(id) => id,
                    None => {
                        tracing::warn!(
                            "Slack approval: no channel_id for session {}",
                            info.session_id
                        );
                        return Ok((false, false));
                    }
                },
            };

            let approval_id = uuid::Uuid::new_v4().to_string();
            let safe_input = crate::utils::redact_tool_input(&info.tool_input);
            let input_pretty = serde_json::to_string_pretty(&safe_input)
                .unwrap_or_else(|_| safe_input.to_string());
            let text = format!(
                "🔐 *Tool Approval Required*\n\nTool: `{}`\nInput:\n```\n{}\n```",
                info.tool_name,
                truncate_str(&input_pretty, 1800),
            );

            let section = SlackBlock::Section(SlackSectionBlock::new().with_text(
                SlackBlockText::MarkDown(SlackBlockMarkDownText::new(text.clone())),
            ));
            let approve_btn = SlackBlockButtonElement::new(
                SlackActionId::new(format!("approve:{}", approval_id)),
                SlackBlockPlainTextOnly::from(SlackBlockPlainText::new("✅ Yes".to_string())),
            )
            .with_style("primary".to_string());
            let always_btn = SlackBlockButtonElement::new(
                SlackActionId::new(format!("always:{}", approval_id)),
                SlackBlockPlainTextOnly::from(SlackBlockPlainText::new(
                    "🔁 Always (session)".to_string(),
                )),
            );
            let yolo_btn = SlackBlockButtonElement::new(
                SlackActionId::new(format!("yolo:{}", approval_id)),
                SlackBlockPlainTextOnly::from(SlackBlockPlainText::new("🔥 YOLO".to_string())),
            );
            let deny_btn = SlackBlockButtonElement::new(
                SlackActionId::new(format!("deny:{}", approval_id)),
                SlackBlockPlainTextOnly::from(SlackBlockPlainText::new("❌ No".to_string())),
            )
            .with_style("danger".to_string());
            let actions = SlackBlock::Actions(SlackActionsBlock::new(vec![
                SlackActionBlockElement::Button(approve_btn),
                SlackActionBlockElement::Button(always_btn),
                SlackActionBlockElement::Button(yolo_btn),
                SlackActionBlockElement::Button(deny_btn),
            ]));

            let content = SlackMessageContent::new()
                .with_text(text)
                .with_blocks(vec![section, actions]);
            let request = SlackApiChatPostMessageRequest::new(
                SlackChannelId::new(channel_id.clone()),
                content,
            );
            let token = SlackApiToken::new(SlackApiTokenValue::from(bot_token.clone()));
            let session = client.open_session(&token);

            // Register BEFORE sending to prevent race condition
            let (tx, rx) = oneshot::channel();
            state
                .register_pending_approval(approval_id.clone(), tx)
                .await;
            tracing::info!(
                "Slack approval: registered pending id={}, sending to channel={}",
                approval_id,
                channel_id
            );

            let sent = match session.chat_post_message(&request).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("Slack approval: failed to send message: {}", e);
                    return Ok((false, false));
                }
            };

            let msg_ts = sent.ts.clone();
            tracing::info!(
                "Slack approval: message sent, waiting for response (id={})",
                approval_id
            );

            match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                Ok(Ok((approved, always))) => {
                    tracing::info!(
                        "Slack approval: user responded id={}, approved={}, always={}",
                        approval_id,
                        approved,
                        always
                    );
                    if always {
                        persist_auto_session_policy();
                    }
                    let label = if always {
                        "🔁 Always approved (session)"
                    } else if approved {
                        "✅ Approved"
                    } else {
                        "❌ Denied"
                    };
                    let update = SlackApiChatUpdateRequest::new(
                        SlackChannelId::new(channel_id),
                        SlackMessageContent::new().with_text(label.to_string()),
                        msg_ts,
                    );
                    let _ = session.chat_update(&update).await;
                    Ok((approved, always))
                }
                Ok(Err(_)) => {
                    tracing::warn!(
                        "Slack approval: oneshot channel closed (id={})",
                        approval_id
                    );
                    Ok((false, false))
                }
                Err(_) => {
                    tracing::warn!(
                        "Slack approval: 5-minute timeout — auto-denying (id={})",
                        approval_id
                    );
                    let update = SlackApiChatUpdateRequest::new(
                        SlackChannelId::new(channel_id),
                        SlackMessageContent::new()
                            .with_text("⏱️ Approval timed out — denied".to_string()),
                        msg_ts,
                    );
                    let _ = session.chat_update(&update).await;
                    Ok((false, false))
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_short_message() {
        let chunks = split_message("hello", 3000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_long_message() {
        let text = "a\n".repeat(2000);
        let chunks = split_message(&text, 3000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 3000);
        }
        let joined: String = chunks.into_iter().collect();
        assert_eq!(joined, text);
    }
}
