//! Discord Message Handler
//!
//! Processes incoming Discord messages: text + image attachments, allowlist enforcement,
//! session routing (owner shares TUI session, others get per-user sessions).

use super::DiscordState;
use crate::brain::agent::AgentService;
use crate::config::{Config, RespondTo};
use crate::db::ChannelMessageRepository;
use crate::db::models::ChannelMessage as DbChannelMessage;
use crate::services::SessionService;
use crate::utils::sanitize::redact_secrets;
use crate::utils::truncate_str;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use serenity::builder::{CreateAttachment, CreateMessage};
use serenity::model::channel::Message;
use serenity::prelude::*;

/// Split a message into chunks that fit Discord's 2000 char limit.
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_message(
    ctx: &Context,
    msg: &Message,
    agent: Arc<AgentService>,
    session_svc: SessionService,
    extra_sessions: Arc<Mutex<HashMap<u64, (Uuid, std::time::Instant)>>>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    discord_state: Arc<DiscordState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    channel_msg_repo: ChannelMessageRepository,
) {
    // Read latest config from watch channel — single source of truth
    let cfg = config_rx.borrow().clone();
    let dc_cfg = &cfg.channels.discord;
    let allowed: HashSet<i64> = dc_cfg
        .allowed_users
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    let respond_to = &dc_cfg.respond_to;
    let allowed_channels: HashSet<String> = dc_cfg.allowed_channels.iter().cloned().collect();
    let idle_timeout_hours = dc_cfg.session_idle_hours;
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
    let openai_tts_key = cfg
        .providers
        .tts
        .as_ref()
        .and_then(|t| t.openai.as_ref())
        .and_then(|p| p.api_key.clone());

    let user_id = msg.author.id.get() as i64;

    // Helper: passively capture a channel message for history
    let store_channel_msg = |text: String| {
        let repo = channel_msg_repo.clone();
        let channel_chat_id = msg.channel_id.get().to_string();
        let guild_name = msg
            .guild_id
            .map(|g| g.get().to_string())
            .unwrap_or_else(|| "DM".to_string());
        let sender_id = msg.author.id.get().to_string();
        let sender_name = msg.author.name.clone();
        let msg_id = msg.id.get().to_string();
        async move {
            if text.is_empty() {
                return;
            }
            let cm = DbChannelMessage::new(
                "discord".into(),
                channel_chat_id,
                Some(guild_name),
                sender_id,
                sender_name,
                text,
                "text".into(),
                Some(msg_id),
            );
            if let Err(e) = repo.insert(&cm).await {
                tracing::warn!("Failed to store Discord channel message: {e}");
            }
        }
    };

    // Allowlist check — if allowed list is empty, accept all
    if !allowed.is_empty() && !allowed.contains(&user_id) {
        tracing::debug!(
            "Discord: ignoring message from non-allowed user {}",
            user_id
        );
        return;
    }

    // respond_to / allowed_channels filtering — DMs always pass
    let is_dm = msg.guild_id.is_none();
    if !is_dm {
        let channel_str = msg.channel_id.get().to_string();

        // Check allowed_channels (empty = all channels allowed)
        if !allowed_channels.is_empty() && !allowed_channels.contains(&channel_str) {
            tracing::debug!(
                "Discord: ignoring message in non-allowed channel {}",
                channel_str
            );
            store_channel_msg(msg.content.clone()).await;
            return;
        }

        match respond_to {
            RespondTo::DmOnly => {
                tracing::debug!("Discord: respond_to=dm_only, ignoring channel message");
                store_channel_msg(msg.content.clone()).await;
                return;
            }
            RespondTo::Mention => {
                let bot_id = discord_state.bot_user_id().await;
                let mentioned =
                    bot_id.is_some_and(|bid| msg.mentions.iter().any(|u| u.id.get() == bid));
                if !mentioned {
                    tracing::debug!("Discord: respond_to=mention, bot not mentioned — ignoring");
                    store_channel_msg(msg.content.clone()).await;
                    return;
                }
            }
            RespondTo::All => {} // pass through
        }
    }

    // Also store directed channel messages for complete history
    if !is_dm {
        store_channel_msg(msg.content.clone()).await;
    }

    // Check for audio attachments → STT
    let audio_attachment = msg.attachments.iter().find(|a| {
        a.content_type
            .as_ref()
            .is_some_and(|ct| ct.starts_with("audio/"))
    });

    let mut is_voice = false;
    let mut content = msg.content.clone();

    if let Some(audio) = audio_attachment
        && voice_config.stt_enabled
        && let Some(ref stt_provider) = voice_config.stt_provider
        && let Some(ref stt_key) = stt_provider.api_key
        && let Ok(resp) = reqwest::get(&audio.url).await
        && let Ok(bytes) = resp.bytes().await
    {
        match crate::channels::voice::transcribe_audio(bytes.to_vec(), stt_key).await {
            Ok(transcript) => {
                tracing::info!(
                    "Discord: transcribed voice: {}",
                    truncate_str(&transcript, 80)
                );
                content = transcript;
                is_voice = true;
            }
            Err(e) => tracing::error!("Discord: STT error: {e}"),
        }
    }

    // Strip bot @mention from content when responding to a mention
    if !is_dm
        && respond_to == &RespondTo::Mention
        && let Some(bot_id) = discord_state.bot_user_id().await
    {
        let mention_tag = format!("<@{}>", bot_id);
        content = content.replace(&mention_tag, "").trim().to_string();
    }
    if content.is_empty() && msg.attachments.is_empty() {
        return;
    }

    // Handle attachments — images as <<IMG:url>>, text files extracted inline
    if !is_voice {
        use crate::utils::{FileContent, classify_file};
        for attachment in &msg.attachments {
            let mime = attachment.content_type.as_deref().unwrap_or("");
            let fname = &attachment.filename;

            if mime.starts_with("image/") {
                if content.is_empty() {
                    content = "Describe this image.".to_string();
                }
                content.push_str(&format!(" <<IMG:{}>>", attachment.url));
            } else if !mime.starts_with("audio/") {
                // Try to download and classify non-audio, non-image attachments
                if let Ok(resp) = reqwest::get(attachment.url.as_str()).await
                    && let Ok(bytes) = resp.bytes().await
                {
                    match classify_file(&bytes, mime, fname) {
                        FileContent::Text(extracted) => {
                            content.push_str(&format!("\n\n{extracted}"));
                        }
                        FileContent::Image => {
                            // Rare: image MIME not caught above — use URL
                            if content.is_empty() {
                                content = "Describe this image.".to_string();
                            }
                            content.push_str(&format!(" <<IMG:{}>>", attachment.url));
                        }
                        FileContent::Unsupported(note) => {
                            content.push_str(&format!("\n\n{note}"));
                        }
                    }
                }
            }
        }
    }

    if content.is_empty() {
        return;
    }

    let text_preview = truncate_str(&content, 50);
    tracing::info!(
        "Discord: message from {} ({}): {}",
        msg.author.name,
        user_id,
        text_preview
    );

    // Track owner's channel for proactive messaging
    let is_owner = allowed.is_empty()
        || allowed
            .iter()
            .next()
            .map(|&a| a == user_id)
            .unwrap_or(false);

    if is_owner {
        discord_state.set_owner_channel(msg.channel_id.get()).await;
    }

    // Track guild ID for guild-scoped actions (kick, ban, roles, list_channels)
    if let Some(guild_id) = msg.guild_id {
        discord_state.set_guild_id(guild_id.get()).await;
    }

    // Resolve session: owner shares TUI session, others get per-user sessions
    let session_id = if is_owner {
        let shared = shared_session.lock().await;
        match *shared {
            Some(id) => id,
            None => {
                drop(shared);
                // Resume most recent session from DB (survives daemon restarts)
                let restored = match session_svc.get_most_recent_session().await {
                    Ok(Some(s)) => {
                        tracing::info!("Discord: restored most recent session {}", s.id);
                        Some(s.id)
                    }
                    _ => None,
                };
                let id = match restored {
                    Some(id) => id,
                    None => {
                        tracing::info!("Discord: no existing session, creating one for owner");
                        match session_svc.create_session(Some("Chat".to_string())).await {
                            Ok(session) => session.id,
                            Err(e) => {
                                tracing::error!("Discord: failed to create session: {}", e);
                                return;
                            }
                        }
                    }
                };
                *shared_session.lock().await = Some(id);
                id
            }
        }
    } else {
        let mut map = extra_sessions.lock().await;
        let disc_user_id = msg.author.id.get();
        if let Some((old_id, last_activity)) = map.get(&disc_user_id).copied() {
            if idle_timeout_hours
                .is_some_and(|h| last_activity.elapsed().as_secs() > (h * 3600.0) as u64)
            {
                let _ = session_svc.archive_session(old_id).await;
                map.remove(&disc_user_id);
                let title = format!("Discord: {}", msg.author.name);
                match session_svc.create_session(Some(title)).await {
                    Ok(session) => {
                        map.insert(disc_user_id, (session.id, std::time::Instant::now()));
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("Discord: failed to create session: {}", e);
                        return;
                    }
                }
            } else {
                map.insert(disc_user_id, (old_id, std::time::Instant::now()));
                old_id
            }
        } else {
            let title = format!("Discord: {}", msg.author.name);
            match session_svc.create_session(Some(title)).await {
                Ok(session) => {
                    map.insert(disc_user_id, (session.id, std::time::Instant::now()));
                    session.id
                }
                Err(e) => {
                    tracing::error!("Discord: failed to create session: {}", e);
                    return;
                }
            }
        }
    };

    // ── Channel commands (/help, /usage, /models) ──────────────────────────
    {
        use crate::channels::commands::{self, ChannelCommand};
        match commands::handle_command(&content, session_id, &agent, &session_svc).await {
            ChannelCommand::Help(body) | ChannelCommand::Usage(body) => {
                let _ = msg.channel_id.say(&ctx.http, &body).await;
                return;
            }
            ChannelCommand::Models(resp) => {
                use serenity::builder::{CreateActionRow, CreateButton, CreateMessage};
                use serenity::model::application::ButtonStyle;
                // Max 5 buttons per row, max 5 rows
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
                                    // Discord button labels max 80 chars
                                    let label = if label.len() > 80 {
                                        format!("{}…", &label[..79])
                                    } else {
                                        label
                                    };
                                    CreateButton::new(format!("model:{}", m))
                                        .label(label)
                                        .style(ButtonStyle::Secondary)
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let builder = CreateMessage::new().content(&resp.text).components(rows);
                let _ = msg.channel_id.send_message(&ctx.http, builder).await;
                return;
            }
            ChannelCommand::Stop => {
                let cancelled = discord_state.cancel_session(session_id).await;
                let reply = if cancelled {
                    "Operation cancelled."
                } else {
                    "No operation in progress."
                };
                let _ = msg.channel_id.say(&ctx.http, reply).await;
                return;
            }
            ChannelCommand::NotACommand => {}
        }
    }

    // Extract replied-to message context so the agent knows what the user is referencing.
    let reply_context = msg.referenced_message.as_ref().and_then(|reply| {
        let reply_text = reply.content.trim();
        if reply_text.is_empty() {
            return None;
        }
        let reply_sender = if reply.author.bot {
            "assistant".to_string()
        } else {
            reply.author.name.clone()
        };
        Some(format!("[Replying to {reply_sender}: \"{reply_text}\"]"))
    });

    // For non-owner users, prepend sender identity so the agent knows who
    // it's talking to and doesn't assume it's the owner.
    let agent_input = if !is_owner {
        let name = &msg.author.name;
        let uid = msg.author.id.get();
        if msg.guild_id.is_some() {
            let channel = msg.channel_id.get();
            format!("[Discord message from {name} (ID {uid}) in channel {channel}]\n{content}")
        } else {
            format!("[Discord DM from {name} (ID {uid})]\n{content}")
        }
    } else {
        content
    };

    // Prepend reply context if the user is replying to a specific message.
    let agent_input = if let Some(ref ctx) = reply_context {
        format!("{ctx}\n{agent_input}")
    } else {
        agent_input
    };

    // Tell the LLM its text response is automatically delivered to the chat,
    // so it should NOT use discord_send for simple text replies.
    let agent_input = format!(
        "[Channel: Discord — your text response is automatically sent to this channel. \
         Do NOT call discord_send to deliver your answer. Only use discord_send for: \
         sending to a different channel, embeds, reactions, threads, files, or moderation.]\n{agent_input}"
    );

    // Register channel for approval routing, then send with approval callback
    discord_state
        .register_session_channel(session_id, msg.channel_id.get())
        .await;
    let approval_cb = make_approval_callback(discord_state.clone());

    let cancel_token = tokio_util::sync::CancellationToken::new();
    discord_state
        .store_cancel_token(session_id, cancel_token.clone())
        .await;

    let result = agent
        .send_message_with_tools_and_callback(
            session_id,
            agent_input,
            None,
            Some(cancel_token),
            Some(approval_cb),
            None,
        )
        .await;

    discord_state.remove_cancel_token(session_id).await;

    match result {
        Ok(response) => {
            // Extract <<IMG:path>> markers — send each as a Discord file attachment.
            let (text_only, img_paths) = crate::utils::extract_img_markers(&response.content);
            let text_only = redact_secrets(&text_only);

            for img_path in img_paths {
                match tokio::fs::read(&img_path).await {
                    Ok(bytes) => {
                        let fname = std::path::Path::new(&img_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("image.png")
                            .to_string();
                        let file = CreateAttachment::bytes(bytes.as_slice(), fname);
                        if let Err(e) = msg
                            .channel_id
                            .send_message(&ctx.http, CreateMessage::new().add_file(file))
                            .await
                        {
                            tracing::error!("Discord: failed to send generated image: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Discord: failed to read image {}: {}", img_path, e);
                    }
                }
            }

            for chunk in split_message(&text_only, 2000) {
                if let Err(e) = msg.channel_id.say(&ctx.http, chunk).await {
                    tracing::error!("Discord: failed to send reply: {}", e);
                }
            }

            // TTS: send voice reply if input was audio and TTS is enabled
            if is_voice
                && voice_config.tts_enabled
                && let Some(ref oai_key) = openai_tts_key
            {
                match crate::channels::voice::synthesize_speech(
                    &response.content,
                    oai_key,
                    &voice_config.tts_voice,
                    &voice_config.tts_model,
                )
                .await
                {
                    Ok(audio_bytes) => {
                        let file = CreateAttachment::bytes(audio_bytes.as_slice(), "response.ogg");
                        if let Err(e) = msg
                            .channel_id
                            .send_message(&ctx.http, CreateMessage::new().add_file(file))
                            .await
                        {
                            tracing::error!("Discord: failed to send TTS voice: {e}");
                        }
                    }
                    Err(e) => tracing::error!("Discord: TTS error: {e}"),
                }
            }
        }
        Err(e) => {
            tracing::error!("Discord: agent error: {}", e);
            let error_msg = format!("Error: {}", e);
            let _ = msg.channel_id.say(&ctx.http, error_msg).await;
        }
    }
}

/// Build an `ApprovalCallback` that sends a Discord message with 3 buttons
/// (Yes / Always / No) and waits up to 5 min for a click.
pub(crate) fn make_approval_callback(
    state: Arc<super::DiscordState>,
) -> crate::brain::agent::ApprovalCallback {
    use crate::brain::agent::ToolApprovalInfo;
    use crate::utils::{check_approval_policy, persist_auto_session_policy};
    use serenity::builder::{CreateActionRow, CreateButton, CreateMessage, EditMessage};
    use serenity::model::application::ButtonStyle;
    use serenity::model::id::ChannelId;
    use tokio::sync::oneshot;

    Arc::new(move |info: ToolApprovalInfo| {
        let state = state.clone();
        Box::pin(async move {
            if let Some(result) = check_approval_policy() {
                return Ok(result);
            }

            let http = match state.http().await {
                Some(h) => h,
                None => {
                    tracing::warn!("Discord approval: bot not connected");
                    return Ok((false, false));
                }
            };

            let channel_id = match state.session_channel(info.session_id).await {
                Some(id) => id,
                None => match state.owner_channel_id().await {
                    Some(id) => id,
                    None => {
                        tracing::warn!(
                            "Discord approval: no channel_id for session {}",
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
                "🔐 **Tool Approval Required**\n\nTool: `{}`\nInput:\n```json\n{}\n```",
                info.tool_name,
                truncate_str(&input_pretty, 1800),
            );

            let row = CreateActionRow::Buttons(vec![
                CreateButton::new(format!("approve:{}", approval_id))
                    .label("✅ Yes")
                    .style(ButtonStyle::Success),
                CreateButton::new(format!("always:{}", approval_id))
                    .label("🔁 Always (session)")
                    .style(ButtonStyle::Primary),
                CreateButton::new(format!("yolo:{}", approval_id))
                    .label("🔥 YOLO")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(format!("deny:{}", approval_id))
                    .label("❌ No")
                    .style(ButtonStyle::Danger),
            ]);

            // Register BEFORE sending to prevent race condition
            let (tx, rx) = oneshot::channel();
            state
                .register_pending_approval(approval_id.clone(), tx)
                .await;
            tracing::info!(
                "Discord approval: registered pending id={}, sending to channel={}",
                approval_id,
                channel_id
            );

            let mut sent_msg = match ChannelId::new(channel_id)
                .send_message(
                    &http,
                    CreateMessage::new().content(&text).components(vec![row]),
                )
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("Discord approval: failed to send message: {}", e);
                    return Ok((false, false));
                }
            };

            tracing::info!(
                "Discord approval: message sent, waiting for response (id={})",
                approval_id
            );

            match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                Ok(Ok((approved, always))) => {
                    tracing::info!(
                        "Discord approval: user responded id={}, approved={}, always={}",
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
                    let _ = sent_msg
                        .edit(&http, EditMessage::new().content(label).components(vec![]))
                        .await;
                    Ok((approved, always))
                }
                Ok(Err(_)) => {
                    tracing::warn!(
                        "Discord approval: oneshot channel closed (id={})",
                        approval_id
                    );
                    Ok((false, false))
                }
                Err(_) => {
                    tracing::warn!(
                        "Discord approval: 5-minute timeout — auto-denying (id={})",
                        approval_id
                    );
                    let _ = sent_msg
                        .edit(
                            &http,
                            EditMessage::new()
                                .content("⏱️ Approval timed out — denied")
                                .components(vec![]),
                        )
                        .await;
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
        let chunks = split_message("hello", 2000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_long_message() {
        let text = "a\n".repeat(1500);
        let chunks = split_message(&text, 2000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 2000);
        }
        let joined: String = chunks.into_iter().collect();
        assert_eq!(joined, text);
    }
}
