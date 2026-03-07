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
                            SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
                        let session = client.open_session(&token);
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
                    if let Some(pname) = provider_name
                        && let Ok(config) = crate::config::Config::load()
                        && let Ok(new_provider) =
                            crate::brain::provider::factory::create_provider_by_name(&config, pname)
                    {
                        state.agent.swap_provider(new_provider);
                    }
                    crate::channels::commands::switch_model(&state.agent, model_name);
                    tracing::info!("Slack: model switched to {}", model_name);
                    if let Some(ref channel) = block_actions.channel {
                        let token =
                            SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
                        let session = client.open_session(&token);
                        let request = SlackApiChatPostMessageRequest::new(
                            channel.id.clone(),
                            SlackMessageContent::new()
                                .with_text(format!("✅ Model switched to `{}`", model_name)),
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
                                state.bot_token.clone(),
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
}

/// Split a message into chunks that fit Slack's limit (conservative 3000 chars).
pub fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = (start + max_len).min(text.len());
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
pub async fn on_push_event(
    event: SlackPushEventCallback,
    client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::debug!("Slack: received push event");
    match event.event {
        SlackEventCallbackBody::Message(msg) => {
            tracing::debug!(
                "Slack: message event from user={:?}, channel={:?}, bot_id={:?}",
                msg.sender.user,
                msg.origin.channel,
                msg.sender.bot_id
            );
            handle_message(&msg, client).await;
        }
        _ => {
            tracing::debug!("Slack: unhandled event type");
        }
    }
    Ok(())
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
async fn handle_message(msg: &SlackMessageEvent, client: Arc<SlackHyperClient>) {
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
        async move {
            if text.is_empty() {
                return;
            }
            let cm = DbChannelMessage::new(
                "slack".into(),
                ch_id,
                None, // Slack channel names require an API call; omit for now
                uid,
                String::new(), // Slack user names require an API call
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
    let mut voice_config = cfg.voice.clone();
    voice_config.stt_provider = cfg.providers.stt.as_ref().and_then(|s| s.groq.clone());
    let tts_providers = cfg.providers.tts.as_ref();
    voice_config.tts_provider = tts_providers.and_then(|t| t.openai.clone());
    if let Some(ref v) = tts_providers.and_then(|t| t.voice.as_ref()) {
        voice_config.tts_voice = v.to_string();
    }
    if let Some(ref m) = tts_providers.and_then(|t| t.model.as_ref()) {
        voice_config.tts_model = m.to_string();
    }

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
                let mentioned = state
                    .bot_user_id
                    .as_ref()
                    .is_some_and(|bid| text.contains(&format!("<@{}>", bid)));
                if !mentioned {
                    tracing::debug!("Slack: respond_to=mention, bot not mentioned — ignoring");
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
            text
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

    // Resolve session: owner shares TUI session, others get per-user sessions
    let session_id = if is_owner {
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
                        match state
                            .session_svc
                            .create_session(Some("Chat".to_string()))
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
        let mut map = state.extra_sessions.lock().await;
        if let Some((old_id, last_activity)) = map.get(&user_id).copied() {
            if idle_timeout_hours
                .is_some_and(|h| last_activity.elapsed().as_secs() > (h * 3600.0) as u64)
            {
                let _ = state.session_svc.archive_session(old_id).await;
                map.remove(&user_id);
                let title = format!("Slack: {}", user_id);
                match state.session_svc.create_session(Some(title)).await {
                    Ok(session) => {
                        map.insert(user_id.clone(), (session.id, std::time::Instant::now()));
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("Slack: failed to create session: {}", e);
                        return;
                    }
                }
            } else {
                map.insert(user_id.clone(), (old_id, std::time::Instant::now()));
                old_id
            }
        } else {
            let title = format!("Slack: {}", user_id);
            match state.session_svc.create_session(Some(title)).await {
                Ok(session) => {
                    map.insert(user_id.clone(), (session.id, std::time::Instant::now()));
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
        use crate::utils::{FileContent, classify_file};
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
                .header("Authorization", format!("Bearer {}", state.bot_token))
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
                if voice_config.stt_enabled
                    && let Some(ref stt_provider) = voice_config.stt_provider
                    && let Some(ref stt_key) = stt_provider.api_key
                {
                    match crate::channels::voice::transcribe_audio(dl_bytes, stt_key).await {
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

            match classify_file(&dl_bytes, mime, fname) {
                FileContent::Image => {
                    let ext = fname.rsplit('.').next().unwrap_or("png");
                    let tmp = std::env::temp_dir().join(format!(
                        "slack_img_{}.{}",
                        uuid::Uuid::new_v4(),
                        ext
                    ));
                    if tokio::fs::write(&tmp, &dl_bytes).await.is_ok() {
                        if content.is_empty() {
                            content = "Describe this image.".to_string();
                        }
                        content.push_str(&format!(" <<IMG:{}>>", tmp.display()));
                        let cleanup = tmp.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                            let _ = tokio::fs::remove_file(cleanup).await;
                        });
                    }
                }
                FileContent::Text(extracted) => {
                    if content.is_empty() {
                        content = extracted;
                    } else {
                        content.push_str(&format!("\n\n{extracted}"));
                    }
                }
                FileContent::Unsupported(note) => {
                    content.push_str(&format!("\n\n{note}"));
                }
            }
        }
    }

    if content.is_empty() {
        tracing::debug!("Slack: no processable content after file handling, ignoring");
        return;
    }

    // ── Channel commands (/help, /usage, /models) ──────────────────────────
    {
        use crate::channels::commands::{self, ChannelCommand};
        match commands::handle_command(&content, session_id, &state.agent, &state.session_svc).await
        {
            ChannelCommand::Help(body) | ChannelCommand::Usage(body) => {
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
                let session = client.open_session(&token);
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id),
                    SlackMessageContent::new().with_text(body),
                );
                let _ = session.chat_post_message(&request).await;
                return;
            }
            ChannelCommand::Models(resp) => {
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
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
                match state
                    .session_svc
                    .create_session(Some("Chat".to_string()))
                    .await
                {
                    Ok(new_session) => {
                        if is_owner {
                            *state.shared_session.lock().await = Some(new_session.id);
                        } else {
                            state.extra_sessions.lock().await.insert(
                                user_id.to_string(),
                                (new_session.id, std::time::Instant::now()),
                            );
                        }
                        state
                            .slack_state
                            .register_session_channel(new_session.id, channel_id.clone())
                            .await;
                        let token =
                            SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
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
                            SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
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
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
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
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
                let session = client.open_session(&token);
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id),
                    SlackMessageContent::new().with_text(reply.to_string()),
                );
                let _ = session.chat_post_message(&request).await;
                return;
            }
            ChannelCommand::UserPrompt(prompt) => {
                content = prompt;
                // fall through to agent with the prompt as the message
            }
            ChannelCommand::UserSystem(text) => {
                let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
                let session = client.open_session(&token);
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id),
                    SlackMessageContent::new().with_text(text),
                );
                let _ = session.chat_post_message(&request).await;
                return;
            }
            ChannelCommand::NotACommand => {}
        }
    }

    // Detect thread replies so the agent knows the message is in a thread context.
    // Slack doesn't embed the parent message in the event — fetching it would require
    // an API call per message, so we note the thread context without the parent text.
    let reply_context = msg
        .origin
        .thread_ts
        .as_ref()
        .map(|ts| format!("[Replying in thread (thread_ts: {ts})]"));

    // For non-owner users, prepend sender identity so the agent knows who
    // it's talking to and doesn't assume it's the owner.
    let agent_input = if !is_owner {
        if is_dm {
            format!("[Slack DM from {user_id}]\n{content}")
        } else {
            format!("[Slack message from {user_id} in channel {channel_id}]\n{content}")
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

    let result = state
        .agent
        .send_message_with_tools_and_callback(
            session_id,
            agent_input,
            None,
            Some(cancel_token),
            Some(approval_cb),
            None,
        )
        .await;

    state.slack_state.remove_cancel_token(session_id).await;

    match result {
        Ok(response) => {
            // Extract <<IMG:path>> markers — upload each as a Slack file.
            let (text_only, img_paths) = crate::utils::extract_img_markers(&response.content);
            let text_only = redact_secrets(&text_only);

            let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
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
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id.clone()),
                    SlackMessageContent::new().with_text(chunk.to_string()),
                );
                if let Err(e) = session.chat_post_message(&request).await {
                    tracing::error!("Slack: failed to send reply: {}", e);
                }
            }
        }
        Err(e) => {
            tracing::error!("Slack: agent error: {}", e);
            let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
            let session = client.open_session(&token);
            let error_msg = format!("Error: {}", e);
            let request = SlackApiChatPostMessageRequest::new(
                SlackChannelId::new(channel_id),
                SlackMessageContent::new().with_text(error_msg),
            );
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
