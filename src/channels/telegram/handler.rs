//! Telegram Message Handler
//!
//! Processes incoming messages: text, voice (STT/TTS), photos, image documents, allowlist enforcement.
//! Supports live streaming (edit-based) and Telegram-native approval inline keyboards.

use super::TelegramState;
use crate::brain::agent::{AgentService, ProgressCallback, ProgressEvent};
use crate::config::{Config, RespondTo};
use crate::db::ChannelMessageRepository;
use crate::db::models::ChannelMessage as DbChannelMessage;
use crate::services::SessionService;
use crate::utils::sanitize::redact_secrets;
use crate::utils::truncate_str;
use std::collections::HashSet;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{
    ChatAction, ChatKind, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId,
    ParseMode,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Guard that cancels a CancellationToken on drop (used for typing loop).
struct TypingGuard(CancellationToken);
impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Individual tool call — each gets its own Telegram message.
struct ToolMsg {
    msg_id: Option<MessageId>,
    name: String,
    context: String,
    /// None = running, Some(true) = success, Some(false) = failed
    completed: Option<bool>,
    dirty: bool,
}

/// Fun rotating status quips shown during long tool execution.
const TOOL_STATUS_QUIPS: &[&str] = &[
    "☕ Grab a coffee — my sub-agents are on fire right now",
    "🦀 My crabs are working their claws off — hang tight",
    "🔥 Still cooking... deep in the code",
    "⚡ Sub-agents going brrr — almost there",
    "🧠 Thinking hard so you don't have to",
    "🏗️ Building something beautiful — one sec",
    "🎯 Locked in — the crabs are laser-focused",
    "🚀 Full speed ahead — engines at max",
    "💪 Crunching through the code like a boss",
    "🌊 Riding the wave — results incoming",
    "🎪 The circus is in town — all crabs performing",
    "🔧 Wrenching away at it — precision work",
    "🏎️ Pedal to the metal — no brakes",
    "🧪 Experimenting... for science!",
    "🎵 Working to the rhythm — almost done",
];

/// Per-message streaming state shared between the progress callback and the edit loop.
/// Each tool call gets its own message above; response streams in a separate message below.
struct StreamingState {
    /// Response/thinking message (always at bottom)
    msg_id: Option<MessageId>,
    /// Reasoning/thinking text — streamed live, cleared before tool calls or response
    thinking: String,
    /// Each tool call = its own individual message
    tool_msgs: Vec<ToolMsg>,
    /// Intermediate agent texts (between tool rounds) — each sent as its own message
    pending_intermediate: Vec<String>,
    /// Response text from streaming chunks — own message at bottom
    response: String,
    dirty: bool,
    /// When true, the edit loop deletes the response message and creates a fresh one
    /// at the bottom of the chat (so it appears below tool/approval messages).
    recreate: bool,
    /// Rolling status message shown during long tool execution (single message, edited in-place)
    status_msg_id: Option<MessageId>,
    /// Number of tool rounds completed (for display)
    tool_round_count: usize,
    /// When tool execution started (for elapsed time)
    tools_started_at: Option<std::time::Instant>,
    /// Index into TOOL_STATUS_QUIPS for rotation
    quip_index: usize,
    /// When the current status quip was shown (for show/vanish timing)
    status_shown_at: Option<std::time::Instant>,
}

impl StreamingState {
    /// Render response message: thinking + response only (tools are separate messages).
    fn render(&self) -> String {
        let mut parts = Vec::new();
        if !self.thinking.is_empty() {
            let t = if self.thinking.len() > 800 {
                &self.thinking[self.thinking.len() - 800..]
            } else {
                &self.thinking
            };
            let t = crate::utils::sanitize::strip_llm_artifacts(t.trim());
            // Collapse repeated whitespace/newlines, then add line breaks after
            // sentence-ending punctuation so thinking text reads well in Telegram.
            let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
            let t = t
                .replace(". ", ".\n")
                .replace("? ", "?\n")
                .replace("! ", "!\n");
            parts.push(format!("💭 _{}_", redact_secrets(&t)));
        }
        if !self.response.is_empty() {
            let resp = crate::utils::sanitize::strip_llm_artifacts(&self.response);
            parts.push(redact_secrets(&resp));
        }
        if parts.is_empty() {
            String::new()
        } else {
            parts.join("\n\n")
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_message(
    bot: Bot,
    msg: Message,
    agent: Arc<AgentService>,
    session_svc: SessionService,
    bot_token: Arc<String>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    telegram_state: Arc<TelegramState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    channel_msg_repo: ChannelMessageRepository,
) -> ResponseResult<()> {
    let user = match msg.from {
        Some(ref u) => u,
        None => return Ok(()),
    };

    let user_id = user.id.0 as i64;

    // /start command -- always respond with user ID (for allowlist setup)
    if let Some(text) = msg.text()
        && text.starts_with("/start")
    {
        let reply = format!(
            "OpenCrabs Telegram Bot\n\nYour user ID: {}\n\nAdd this ID to your config.toml under [channels.telegram] allowed_users to get started.",
            user_id
        );
        bot.send_message(msg.chat.id, reply).await?;
        tracing::info!(
            "Telegram: /start from user {} ({})",
            user_id,
            user.first_name
        );
        return Ok(());
    }

    // Read latest config from watch channel — single source of truth
    let cfg = config_rx.borrow().clone();
    let tg_cfg = &cfg.channels.telegram;
    let allowed: HashSet<i64> = tg_cfg
        .allowed_users
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    let respond_to = &tg_cfg.respond_to;
    let allowed_channels: HashSet<String> = tg_cfg.allowed_channels.iter().cloned().collect();
    let idle_timeout_hours = tg_cfg.session_idle_hours;
    let voice_config = cfg.voice_config();

    // Allowlist check — read from config (hot-reloaded via watch channel)
    if !allowed.is_empty() && !allowed.contains(&user_id) {
        tracing::debug!(
            "Telegram: ignoring message from non-allowed user {}",
            user_id
        );
        bot.send_message(
            msg.chat.id,
            "You are not authorized. Send /start to get your user ID.",
        )
        .await?;
        return Ok(());
    }

    // respond_to / allowed_channels filtering — private chats always pass
    let is_dm = matches!(msg.chat.kind, ChatKind::Private { .. });
    let chat_title = msg
        .chat
        .title()
        .unwrap_or(if is_dm { "DM" } else { "unknown" });
    let chat_kind = match &msg.chat.kind {
        ChatKind::Private { .. } => "private",
        ChatKind::Public(public) => match &public.kind {
            teloxide::types::PublicChatKind::Group { .. } => "group",
            teloxide::types::PublicChatKind::Supergroup { .. } => "supergroup",
            teloxide::types::PublicChatKind::Channel { .. } => "channel",
        },
    };

    tracing::info!(
        "Telegram: incoming msg in {} \"{}\" (chat_id={}) from {} ({}) — kind={}, text={}",
        chat_kind,
        chat_title,
        msg.chat.id.0,
        user.first_name,
        user_id,
        if msg.text().is_some() {
            "text"
        } else if msg.voice().is_some() {
            "voice"
        } else if msg.photo().is_some() {
            "photo"
        } else if msg.document().is_some() {
            "document"
        } else {
            "other"
        },
        truncate_str(msg.text().or(msg.caption()).unwrap_or(""), 60),
    );

    // Helper: passively capture a group message for channel history
    let store_channel_msg = |text: String| {
        let repo = channel_msg_repo.clone();
        let channel_chat_id = msg.chat.id.0.to_string();
        let chat_name = chat_title.to_string();
        let sender_id = user.id.0.to_string();
        let sender_name = user.first_name.clone();
        let msg_id = msg.id.0.to_string();
        async move {
            if text.is_empty() {
                return;
            }
            let cm = DbChannelMessage::new(
                "telegram".into(),
                channel_chat_id,
                Some(chat_name),
                sender_id,
                sender_name,
                text,
                "text".into(),
                Some(msg_id),
            );
            if let Err(e) = repo.insert(&cm).await {
                tracing::warn!("Failed to store channel message: {e}");
            }
        }
    };

    if !is_dm {
        let chat_id_str = msg.chat.id.0.to_string();

        // Check allowed_channels (empty = all channels allowed)
        if !allowed_channels.is_empty() && !allowed_channels.contains(&chat_id_str) {
            tracing::debug!(
                "Telegram: dropping — chat {} not in allowed_channels",
                chat_id_str
            );
            store_channel_msg(msg.text().or(msg.caption()).unwrap_or("").to_string()).await;
            return Ok(());
        }

        match respond_to {
            RespondTo::DmOnly => {
                tracing::debug!(
                    "Telegram: dropping — respond_to=dm_only, {} \"{}\"",
                    chat_kind,
                    chat_title
                );
                store_channel_msg(msg.text().or(msg.caption()).unwrap_or("").to_string()).await;
                return Ok(());
            }
            RespondTo::Mention => {
                // Check if bot is @mentioned in text or message is a reply to the bot
                let bot_username = telegram_state.bot_username().await;
                let text_content = msg.text().or(msg.caption()).unwrap_or("");

                let mentioned_by_username = bot_username
                    .as_ref()
                    .is_some_and(|uname| text_content.contains(&format!("@{}", uname)));

                let replied_to_bot = msg
                    .reply_to_message()
                    .is_some_and(|reply| reply.from.as_ref().is_some_and(|u| u.is_bot));

                tracing::info!(
                    "Telegram: group mention check — mentioned={}, replied_to_bot={}, bot_username={:?}",
                    mentioned_by_username,
                    replied_to_bot,
                    bot_username,
                );

                if !mentioned_by_username && !replied_to_bot {
                    tracing::info!(
                        "Telegram: group msg not directed at bot — {} in \"{}\" said: {}",
                        user.first_name,
                        chat_title,
                        truncate_str(text_content, 80),
                    );
                    store_channel_msg(text_content.to_string()).await;
                    return Ok(());
                }
                tracing::info!(
                    "Telegram: bot mentioned/replied in \"{}\" by {} — processing",
                    chat_title,
                    user.first_name,
                );
            }
            RespondTo::All => {
                tracing::debug!(
                    "Telegram: respond_to=all, processing {} \"{}\"",
                    chat_kind,
                    chat_title
                );
            }
        }
    }

    // Also store directed group messages for complete history
    if !is_dm {
        store_channel_msg(msg.text().or(msg.caption()).unwrap_or("").to_string()).await;
    }

    // Extract text from either text message or voice note (via STT)
    let (text, is_voice) = if let Some(t) = msg.text() {
        if t.is_empty() {
            return Ok(());
        }
        (t.to_string(), false)
    } else if let Some(voice) = msg.voice() {
        // Voice note -- transcribe via STT provider
        if !voice_config.stt_enabled {
            bot.send_message(msg.chat.id, "Voice notes are not enabled.")
                .await?;
            return Ok(());
        }

        tracing::info!(
            "Telegram: voice note from user {} ({}) — {}s",
            user_id,
            user.first_name,
            voice.duration,
        );

        // Show typing immediately so user knows we're processing
        let _ = bot
            .send_chat_action(msg.chat.id, teloxide::types::ChatAction::Typing)
            .await;

        // Download the voice file from Telegram
        let file = bot.get_file(&voice.file.id).await?;
        let download_url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            bot_token.as_str(),
            file.path
        );

        let audio_bytes = match reqwest::get(&download_url).await {
            Ok(resp) => match resp.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => {
                    tracing::error!("Telegram: failed to read voice file bytes: {}", e);
                    bot.send_message(msg.chat.id, "Failed to download voice note.")
                        .await?;
                    return Ok(());
                }
            },
            Err(e) => {
                tracing::error!("Telegram: failed to download voice file: {}", e);
                bot.send_message(msg.chat.id, "Failed to download voice note.")
                    .await?;
                return Ok(());
            }
        };

        // Transcribe with STT dispatch (API or Local based on config)
        match crate::channels::voice::transcribe(audio_bytes, &voice_config).await {
            Ok(transcript) => {
                tracing::info!(
                    "Telegram: transcribed voice: {}",
                    truncate_str(&transcript, 80)
                );
                (transcript, true)
            }
            Err(e) => {
                tracing::error!("Telegram: STT error: {}", e);
                bot.send_message(msg.chat.id, format!("Transcription error: {}", e))
                    .await?;
                return Ok(());
            }
        }
    } else if let Some(photos) = msg.photo() {
        // Photo -- download and send to agent as image attachment
        let Some(photo) = photos.last() else {
            return Ok(());
        };
        tracing::info!(
            "Telegram: photo from user {} ({}) — {}x{}",
            user_id,
            user.first_name,
            photo.width,
            photo.height,
        );

        let file = bot.get_file(&photo.file.id).await?;
        let download_url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            bot_token.as_str(),
            file.path
        );

        let photo_bytes = match reqwest::get(&download_url).await {
            Ok(resp) => match resp.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => {
                    tracing::error!("Telegram: failed to read photo bytes: {}", e);
                    bot.send_message(msg.chat.id, "Failed to download photo.")
                        .await?;
                    return Ok(());
                }
            },
            Err(e) => {
                tracing::error!("Telegram: failed to download photo: {}", e);
                bot.send_message(msg.chat.id, "Failed to download photo.")
                    .await?;
                return Ok(());
            }
        };

        // Save to temp file so the agent's <<IMG:path>> pipeline can handle it
        let tmp_path = std::env::temp_dir().join(format!("tg_photo_{}.jpg", Uuid::new_v4()));
        if let Err(e) = tokio::fs::write(&tmp_path, &photo_bytes).await {
            tracing::error!("Telegram: failed to write temp photo: {}", e);
            bot.send_message(msg.chat.id, "Failed to process photo.")
                .await?;
            return Ok(());
        }

        // Use caption if provided, otherwise generic prompt
        let caption = msg.caption().unwrap_or("Analyze this image");
        let text_with_img = format!("<<IMG:{}>> {}", tmp_path.display(), caption);

        // Clean up temp file after a delay (don't block)
        let cleanup_path = tmp_path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let _ = tokio::fs::remove_file(cleanup_path).await;
        });

        (text_with_img, false)
    } else if let Some(doc) = msg.document() {
        let fname = doc.file_name.as_deref().unwrap_or("file");
        let mime = doc.mime_type.as_ref().map(|m| m.as_ref()).unwrap_or("");
        let ext = fname.rsplit('.').next().unwrap_or("bin");
        let caption = msg.caption().unwrap_or("");

        tracing::info!(
            "Telegram: document from user {} — name={} mime={}",
            user_id,
            fname,
            mime
        );

        let file = bot.get_file(&doc.file.id).await?;
        let download_url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            bot_token.as_str(),
            file.path
        );

        let bytes = match reqwest::get(&download_url).await {
            Ok(resp) => match resp.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => {
                    tracing::error!("Telegram: failed to read document bytes: {}", e);
                    bot.send_message(msg.chat.id, "Failed to download file.")
                        .await?;
                    return Ok(());
                }
            },
            Err(e) => {
                tracing::error!("Telegram: failed to download document: {}", e);
                bot.send_message(msg.chat.id, "Failed to download file.")
                    .await?;
                return Ok(());
            }
        };

        use crate::utils::{FileContent, classify_file};
        match classify_file(&bytes, mime, fname) {
            FileContent::Image => {
                let tmp_path =
                    std::env::temp_dir().join(format!("tg_doc_{}.{}", Uuid::new_v4(), ext));
                if let Err(e) = tokio::fs::write(&tmp_path, &bytes).await {
                    tracing::error!("Telegram: failed to write temp image: {}", e);
                    bot.send_message(msg.chat.id, "Failed to process file.")
                        .await?;
                    return Ok(());
                }
                let prompt = if caption.is_empty() {
                    "Analyze this image."
                } else {
                    caption
                };
                let result = format!("<<IMG:{}>> {}", tmp_path.display(), prompt);
                let cleanup = tmp_path.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    let _ = tokio::fs::remove_file(cleanup).await;
                });
                (result, false)
            }
            FileContent::Text(extracted) => {
                let result = if caption.is_empty() {
                    extracted
                } else {
                    format!("{caption}\n\n{extracted}")
                };
                (result, false)
            }
            FileContent::Unsupported(note) => (note, false),
        }
    } else {
        // Non-text, non-voice, non-photo message -- ignore
        return Ok(());
    };

    // Log ALL processed messages (voice transcripts, photo captions, doc text) for group context.
    // Text-only messages in groups were already logged above during respond_to filtering;
    // this catches voice, photo, and document messages that bypassed the early return paths.
    if !is_dm {
        let log_content = if is_voice {
            format!("[voice] {}", truncate_str(&text, 500))
        } else if msg.photo().is_some() {
            format!("[photo] {}", msg.caption().unwrap_or(""))
        } else if msg.document().is_some() {
            format!("[document] {}", msg.caption().unwrap_or(""))
        } else {
            String::new() // text was already logged above
        };
        if !log_content.is_empty() {
            store_channel_msg(log_content).await;
        }
    }

    // Strip @bot_username from text when responding to a mention in groups
    let text = if !is_dm && respond_to == &RespondTo::Mention {
        if let Some(ref uname) = telegram_state.bot_username().await {
            text.replace(&format!("@{}", uname), "").trim().to_string()
        } else {
            text
        }
    } else {
        text
    };

    tracing::info!(
        "Telegram: {} from user {} ({}): {}",
        if is_voice { "voice" } else { "text" },
        user_id,
        user.first_name,
        truncate_str(&text, 50)
    );

    // Start typing indicator loop — cancelled via guard on all return paths
    let typing_cancel = CancellationToken::new();
    let _typing_guard = TypingGuard(typing_cancel.clone());
    tokio::spawn({
        let bot = bot.clone();
        let chat = msg.chat.id;
        let cancel = typing_cancel.clone();
        async move {
            loop {
                let _ = bot.send_chat_action(chat, ChatAction::Typing).await;
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(4)) => {}
                }
            }
        }
    });

    // Resolve session: owner shares the TUI session, other users get their own.
    // Owner = first user in the config's allowed_users list (Vec order, not HashSet).
    let owner_id = tg_cfg
        .allowed_users
        .first()
        .and_then(|s| s.parse::<i64>().ok());
    let is_owner = allowed.is_empty() || owner_id == Some(user_id);

    tracing::info!(
        "Telegram: session resolve — is_owner={}, is_dm={}, chat=\"{}\" ({}), user={} ({})",
        is_owner,
        is_dm,
        chat_title,
        msg.chat.id.0,
        user.first_name,
        user_id,
    );

    // Track owner's chat ID for proactive messaging
    if is_owner {
        telegram_state.set_owner_chat_id(msg.chat.id.0).await;
    }

    let session_id = if is_owner && is_dm {
        // Owner DM shares the TUI's current session (or daemon's persisted session)
        let shared = shared_session.lock().await;
        match *shared {
            Some(id) => id,
            None => {
                drop(shared); // release lock before async calls
                // Try to resume the most recent active session from DB (survives daemon restarts)
                let restored = match session_svc.get_most_recent_session().await {
                    Ok(Some(session)) => {
                        tracing::info!(
                            "Telegram: restored most recent session {} for owner",
                            session.id
                        );
                        Some(session.id)
                    }
                    _ => None,
                };
                let id = match restored {
                    Some(id) => id,
                    None => {
                        tracing::info!("Telegram: no existing session, creating one for owner");
                        match session_svc.create_session(Some("Chat".to_string())).await {
                            Ok(session) => session.id,
                            Err(e) => {
                                tracing::error!("Telegram: failed to create session: {}", e);
                                bot.send_message(msg.chat.id, "Internal error creating session.")
                                    .await?;
                                return Ok(());
                            }
                        }
                    }
                };
                *shared_session.lock().await = Some(id);
                id
            }
        }
    } else {
        // Non-DM-owner sessions: persisted in DB by title — survives restarts.
        let session_title = if is_dm {
            format!("Telegram: {}", user.first_name)
        } else {
            format!("Telegram: {}", chat_title)
        };

        let existing = session_svc
            .find_session_by_title(&session_title)
            .await
            .ok()
            .flatten();

        if let Some(session) = existing {
            if idle_timeout_hours.is_some_and(|h| {
                let elapsed = (chrono::Utc::now() - session.updated_at).num_seconds();
                elapsed > (h * 3600.0) as i64
            }) {
                let _ = session_svc.archive_session(session.id).await;
                match session_svc.create_session(Some(session_title)).await {
                    Ok(new_session) => new_session.id,
                    Err(e) => {
                        tracing::error!("Telegram: failed to create session: {}", e);
                        bot.send_message(msg.chat.id, "Internal error creating session.")
                            .await?;
                        return Ok(());
                    }
                }
            } else {
                session.id
            }
        } else {
            match session_svc.create_session(Some(session_title)).await {
                Ok(session) => {
                    tracing::info!(
                        "Telegram: created new channel session {} for {}",
                        session.id,
                        chat_title
                    );
                    session.id
                }
                Err(e) => {
                    tracing::error!("Telegram: failed to create session: {}", e);
                    bot.send_message(msg.chat.id, "Internal error creating session.")
                        .await?;
                    return Ok(());
                }
            }
        }
    };

    tracing::info!(
        "Telegram: resolved session={} for {} in {} \"{}\" (chat_id={})",
        session_id,
        user.first_name,
        chat_kind,
        chat_title,
        msg.chat.id.0,
    );

    // Register session → chat for approval routing
    telegram_state
        .register_session_chat(session_id, msg.chat.id.0)
        .await;

    // Sync channel agent's provider with config (TUI or other channel may have switched)
    crate::channels::commands::sync_provider_from_config(&agent);

    // ── Channel commands (/help, /usage, /models) ──────────────────────────
    let mut text = text;
    if !is_voice {
        use crate::channels::commands::{self, ChannelCommand};
        let cmd = commands::handle_command(&text, session_id, &agent, &session_svc).await;

        // Handle simple text-response commands (Help, Usage, Evolve, Doctor, etc.)
        if let Some(reply) = commands::try_execute_text_command(&cmd).await {
            bot.send_message(msg.chat.id, md_to_html(&reply))
                .parse_mode(ParseMode::Html)
                .await?;
            return Ok(());
        }

        match cmd {
            ChannelCommand::Models(resp) => {
                let rows: Vec<Vec<InlineKeyboardButton>> = resp
                    .providers
                    .iter()
                    .map(|(name, label)| {
                        let display = if *name == resp.current_provider {
                            format!("✓ {}", label)
                        } else {
                            label.clone()
                        };
                        vec![InlineKeyboardButton::callback(
                            display,
                            format!("provider:{}", name),
                        )]
                    })
                    .collect();
                let keyboard = InlineKeyboardMarkup::new(rows);
                bot.send_message(msg.chat.id, md_to_html(&resp.text))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                return Ok(());
            }
            ChannelCommand::NewSession => {
                let session_title = if is_dm {
                    format!("Telegram: {}", user.first_name)
                } else {
                    format!("Telegram: {}", chat_title)
                };
                if !is_owner
                    && let Ok(Some(old)) = session_svc.find_session_by_title(&session_title).await
                {
                    let _ = session_svc.archive_session(old.id).await;
                }
                match session_svc.create_session(Some(session_title)).await {
                    Ok(new_session) => {
                        if is_owner {
                            *shared_session.lock().await = Some(new_session.id);
                        }
                        telegram_state
                            .register_session_chat(new_session.id, msg.chat.id.0)
                            .await;
                        bot.send_message(msg.chat.id, "✅ New session started.")
                            .await?;
                    }
                    Err(e) => {
                        tracing::error!("Telegram: failed to create session: {}", e);
                        bot.send_message(msg.chat.id, "Failed to create session.")
                            .await?;
                    }
                }
                return Ok(());
            }
            ChannelCommand::Sessions(resp) => {
                let rows: Vec<Vec<InlineKeyboardButton>> = resp
                    .sessions
                    .iter()
                    .map(|(id, label)| {
                        let display = if *id == resp.current_session_id {
                            format!("✓ {}", label)
                        } else {
                            label.clone()
                        };
                        vec![InlineKeyboardButton::callback(
                            display,
                            format!("session:{}", id),
                        )]
                    })
                    .collect();
                let keyboard = InlineKeyboardMarkup::new(rows);
                bot.send_message(msg.chat.id, md_to_html(&resp.text))
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
                return Ok(());
            }
            ChannelCommand::Stop => {
                let cancelled = telegram_state.cancel_session(session_id).await;
                let reply = if cancelled {
                    "Operation cancelled."
                } else {
                    "No operation in progress."
                };
                bot.send_message(msg.chat.id, reply).await?;
                return Ok(());
            }
            ChannelCommand::Compact => {
                bot.send_message(msg.chat.id, "⏳ Compacting context...")
                    .await?;
                text = "[SYSTEM: Compact context now. Summarize this conversation for continuity.]"
                    .to_string();
                // fall through to agent
            }
            ChannelCommand::UserPrompt(prompt) => {
                text = prompt;
                // fall through to agent with the prompt as the message
            }
            ChannelCommand::NotACommand => {} // fall through to agent
            // Help, Usage, Evolve, Doctor, UserSystem handled by try_execute_text_command above
            _ => {}
        }
    }

    // Extract replied-to message context so the agent knows what the user is referencing.
    let reply_context = msg.reply_to_message().and_then(|reply| {
        let reply_text = reply.text().or(reply.caption()).unwrap_or("").trim();
        if reply_text.is_empty() {
            return None;
        }
        let reply_sender = reply
            .from
            .as_ref()
            .map(|u| {
                if u.is_bot {
                    "assistant".to_string()
                } else {
                    u.first_name.clone()
                }
            })
            .unwrap_or_else(|| "unknown".to_string());
        Some(format!("[Replying to {reply_sender}: \"{reply_text}\"]"))
    });

    // Prepend sender identity and group context so the agent knows who and where.
    let agent_input = {
        let mut name = user.first_name.clone();
        if let Some(ref last) = user.last_name {
            name.push(' ');
            name.push_str(last);
        }
        let handle = user
            .username
            .as_ref()
            .map(|u| format!(" (@{})", u))
            .unwrap_or_default();
        if is_dm {
            if is_owner {
                text.clone()
            } else {
                format!("[Telegram DM from {name}{handle}, ID {user_id}]\n{text}")
            }
        } else {
            // Always include group context — even for the owner — so the agent
            // knows it's in a group and who is speaking.
            format!(
                "[Telegram group \"{}\" — {} from {name}{handle}]\n{text}",
                chat_title,
                if is_owner { "owner" } else { "user" },
            )
        }
    };

    // Prepend reply context if the user is replying to a specific message.
    let agent_input = if let Some(ref ctx) = reply_context {
        format!("{ctx}\n{agent_input}")
    } else {
        agent_input
    };

    // Inject recent group history so the agent has full conversation context.
    let agent_input = if !is_dm {
        let chat_id_str = msg.chat.id.0.to_string();
        match channel_msg_repo
            .recent(Some("telegram"), &chat_id_str, 30)
            .await
        {
            Ok(messages) if !messages.is_empty() => {
                let history: Vec<String> = messages
                    .iter()
                    .rev() // oldest first
                    .map(|m| {
                        let ts = m.created_at.format("%H:%M");
                        format!("[{}] {}: {}", ts, m.sender_name, m.content)
                    })
                    .collect();
                format!(
                    "[Recent group history ({} messages):\n{}\n--- end history ---]\n{}",
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
    // so it should NOT use telegram_send for simple text replies.
    let agent_input = format!(
        "[Channel: Telegram — your text response is automatically sent to this chat. \
         Do NOT call telegram_send to deliver your answer. Only use telegram_send for: \
         sending to a different chat_id, media, polls, buttons, reactions, or moderation.]\n{agent_input}"
    );

    // ── Streaming setup ───────────────────────────────────────────────────────
    let streaming = Arc::new(Mutex::new(StreamingState {
        msg_id: None,
        thinking: String::new(),
        tool_msgs: Vec::new(),
        pending_intermediate: Vec::new(),
        response: String::new(),
        dirty: false,
        recreate: false,
        status_msg_id: None,
        tool_round_count: 0,
        tools_started_at: None,
        quip_index: 0,
        status_shown_at: None,
    }));

    let edit_cancel = CancellationToken::new();

    // Edit loop: sends individual tool messages + streams response at bottom
    tokio::spawn({
        let bot = bot.clone();
        let chat = msg.chat.id;
        let st = streaming.clone();
        let cancel = edit_cancel.clone();
        async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(1500)) => {
                        let mut s = st.lock().await;
                        let any_tools_dirty = s.tool_msgs.iter().any(|t| t.dirty);
                        let has_intermediate = !s.pending_intermediate.is_empty();
                        let has_active_tools = s.tool_msgs.iter().any(|t| t.completed.is_none());

                        if !s.dirty && !s.recreate && !any_tools_dirty && !has_intermediate && !has_active_tools { continue; }

                        // ── Individual tool messages (each tool = its own message) ──
                        for tool in s.tool_msgs.iter_mut().filter(|t| t.dirty) {
                            let label = format!("**{}**{}", tool.name, tool.context);
                            let text = match tool.completed {
                                None => format!("⚙️ {}", label),
                                Some(true) => format!("✅ {}", label),
                                Some(false) => format!("❌ {}", label),
                            };
                            let html = markdown_to_telegram_html(&text);
                            if let Some(mid) = tool.msg_id {
                                let _ = bot
                                    .edit_message_text(chat, mid, &html)
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            } else if let Ok(m) = bot
                                .send_message(chat, &html)
                                .parse_mode(ParseMode::Html)
                                .await
                            {
                                tool.msg_id = Some(m.id);
                            }
                            tool.dirty = false;
                        }

                        // ── Rolling status quips during long tool execution ──
                        // Show → stick 5s → vanish → pause 2s → next quip.
                        // Keeps user informed during long gaps with no streaming text.
                        if has_active_tools {
                            let now = std::time::Instant::now();
                            let shown_elapsed = s.status_shown_at
                                .map(|t| now.duration_since(t).as_secs())
                                .unwrap_or(999);

                            if s.status_msg_id.is_some() && shown_elapsed >= 5 {
                                // Vanish: delete current quip after 5s
                                if let Some(mid) = s.status_msg_id.take() {
                                    let _ = bot.delete_message(chat, mid).await;
                                }
                                s.status_shown_at = Some(now);
                            } else if s.status_msg_id.is_none() && shown_elapsed >= 2 {
                                // Show next quip after 2s gap
                                let elapsed_total = s.tools_started_at
                                    .map(|t| t.elapsed().as_secs())
                                    .unwrap_or(0);
                                let quip = TOOL_STATUS_QUIPS[s.quip_index % TOOL_STATUS_QUIPS.len()];
                                s.quip_index += 1;

                                let mut status = format!("{} ({} tools", quip, s.tool_round_count);
                                if elapsed_total >= 5 {
                                    let mins = elapsed_total / 60;
                                    let secs = elapsed_total % 60;
                                    if mins > 0 {
                                        status.push_str(&format!(", {}m {}s", mins, secs));
                                    } else {
                                        status.push_str(&format!(", {}s", secs));
                                    }
                                }
                                status.push(')');

                                if let Ok(m) = bot.send_message(chat, &status).await {
                                    s.status_msg_id = Some(m.id);
                                    s.status_shown_at = Some(now);
                                }
                            }
                        }

                        // ── Delete status when real content arrives ──
                        if has_intermediate || (s.dirty && !s.response.is_empty()) {
                            if let Some(mid) = s.status_msg_id.take() {
                                let _ = bot.delete_message(chat, mid).await;
                            }
                            // Reset tool tracking for next round
                            s.tools_started_at = None;
                            s.tool_round_count = 0;
                        }

                        // ── Intermediate texts (agent commentary between tool rounds) ──
                        for text in s.pending_intermediate.drain(..) {
                            let text = crate::utils::sanitize::strip_llm_artifacts(&text);
                            let html = markdown_to_telegram_html(&text);
                            if !html.is_empty() {
                                let _ = bot
                                    .send_message(chat, &html)
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                        }

                        // ── Response message (thinking + response, always at bottom) ──
                        if s.dirty || s.recreate {
                            if s.recreate {
                                if let Some(old_mid) = s.msg_id.take() {
                                    let _ = bot.delete_message(chat, old_mid).await;
                                }
                                s.recreate = false;
                            }
                            let text = s.render();
                            if !text.is_empty() {
                                // Delete status msg if still present (response starting)
                                if let Some(mid) = s.status_msg_id.take() {
                                    let _ = bot.delete_message(chat, mid).await;
                                }
                                if s.msg_id.is_none()
                                    && let Ok(m) = bot.send_message(chat, "\u{258b}").await
                                {
                                    s.msg_id = Some(m.id);
                                }
                                if let Some(mid) = s.msg_id {
                                    let html = markdown_to_telegram_html(&text);
                                    let display = format!("{}\u{258b}", html); // ▋ cursor
                                    let _ = bot
                                        .edit_message_text(chat, mid, display)
                                        .parse_mode(ParseMode::Html)
                                        .await;
                                }
                            }
                            s.dirty = false;
                        }

                        // Re-send typing indicator after any bot message
                        // (Telegram clears typing on every bot message)
                        let _ = bot.send_chat_action(chat, ChatAction::Typing).await;
                    }
                }
            }
        }
    });

    // Progress callback: accumulates streaming chunks + tool status into shared state
    let progress_cb: ProgressCallback = {
        let st = streaming.clone();
        Arc::new(move |_sid, event| {
            match event {
                ProgressEvent::ReasoningChunk { text } => {
                    if let Ok(mut s) = st.try_lock() {
                        s.thinking.push_str(&text);
                        s.dirty = true;
                    }
                }
                ProgressEvent::StreamingChunk { text } => {
                    if let Ok(mut s) = st.try_lock() {
                        if !s.thinking.is_empty() {
                            s.thinking.clear();
                        }
                        s.response.push_str(&text);
                        s.dirty = true;
                    }
                }
                ProgressEvent::ToolStarted {
                    tool_name,
                    tool_input,
                } => {
                    if let Ok(mut s) = st.try_lock() {
                        s.thinking.clear();
                        if s.tools_started_at.is_none() {
                            s.tools_started_at = Some(std::time::Instant::now());
                        }
                        let ctx = tool_context(&tool_name, &tool_input);
                        s.tool_msgs.push(ToolMsg {
                            msg_id: None,
                            name: tool_name,
                            context: ctx,
                            completed: None,
                            dirty: true,
                        });
                    }
                }
                ProgressEvent::ToolCompleted {
                    tool_name, success, ..
                } => {
                    if let Ok(mut s) = st.try_lock() {
                        s.tool_round_count += 1;
                        if let Some(tool) = s
                            .tool_msgs
                            .iter_mut()
                            .rev()
                            .find(|t| t.name == tool_name && t.completed.is_none())
                        {
                            tool.completed = Some(success);
                            tool.dirty = true;
                        }
                        // Push response to bottom so it stays below tool/approval messages
                        if s.msg_id.is_some() {
                            s.recreate = true;
                        }
                    }
                }
                ProgressEvent::IntermediateText { text, .. } => {
                    if let Ok(mut s) = st.try_lock() {
                        s.thinking.clear();
                        // Clear accumulated streaming response — it's now captured
                        // as an intermediate message. Without this, text from
                        // consecutive tool rounds gets concatenated without spacing.
                        s.response.clear();
                        s.pending_intermediate.push(text);
                    }
                }
                _ => {}
            }
        })
    };

    // Build Telegram-native approval callback for this session
    let approval_cb = make_approval_callback(telegram_state.clone());

    // ── Agent call ────────────────────────────────────────────────────────────
    let cancel_token = tokio_util::sync::CancellationToken::new();
    telegram_state
        .store_cancel_token(session_id, cancel_token.clone())
        .await;

    let result = agent
        .send_message_with_tools_and_callback(
            session_id,
            agent_input.clone(),
            None,
            Some(cancel_token.clone()),
            Some(approval_cb),
            Some(progress_cb.clone()),
        )
        .await;

    // If session lookup failed (DB contention on restart), create a fresh session and retry once
    let result = if let Err(ref e) = result {
        let es = e.to_string();
        if es.contains("Failed to get session") || es.contains("Session not found") {
            tracing::warn!(
                "Telegram: session {} lookup failed ({}), creating fresh session and retrying",
                session_id,
                es
            );
            match session_svc.create_session(Some("Chat".to_string())).await {
                Ok(new_session) => {
                    let new_id = new_session.id;
                    if is_owner {
                        *shared_session.lock().await = Some(new_id);
                    }
                    telegram_state
                        .register_session_chat(new_id, msg.chat.id.0)
                        .await;
                    let approval_cb2 = make_approval_callback(telegram_state.clone());
                    let cancel_token2 = tokio_util::sync::CancellationToken::new();
                    telegram_state
                        .store_cancel_token(new_id, cancel_token2.clone())
                        .await;
                    let retry_result = agent
                        .send_message_with_tools_and_callback(
                            new_id,
                            agent_input,
                            None,
                            Some(cancel_token2),
                            Some(approval_cb2),
                            Some(progress_cb),
                        )
                        .await;
                    telegram_state.remove_cancel_token(new_id).await;
                    retry_result
                }
                Err(e2) => {
                    tracing::error!("Telegram: failed to create fallback session: {}", e2);
                    result
                }
            }
        } else {
            result
        }
    } else {
        result
    };

    // Clean up cancel token
    telegram_state.remove_cancel_token(session_id).await;

    // Stop edit loop — final content will be written below
    edit_cancel.cancel();
    // _typing_guard drop cancels typing loop

    // Grab streaming message id and clean up status message
    let (streaming_msg_id, status_msg_id) = {
        let s = streaming.lock().await;
        (s.msg_id, s.status_msg_id)
    };
    // Delete rolling status message if still present
    if let Some(mid) = status_msg_id {
        let _ = bot.delete_message(msg.chat.id, mid).await;
    }

    tracing::info!(
        "Telegram: agent call completed for session {} — delivering final response",
        session_id
    );

    // ── Final response ────────────────────────────────────────────────────────
    match result {
        Ok(response) => {
            // Extract <<IMG:path>> markers — send each as a Telegram photo.
            let (text_only, img_paths) = crate::utils::extract_img_markers(&response.content);
            // Strip LLM-hallucinated artifacts (<!-- tools-v2 -->, XML tool blocks)
            let text_only = crate::utils::sanitize::strip_llm_artifacts(&text_only);
            let text_only = redact_secrets(&text_only);

            for img_path in img_paths {
                match tokio::fs::read(&img_path).await {
                    Ok(bytes) => {
                        if let Err(e) = bot.send_photo(msg.chat.id, InputFile::memory(bytes)).await
                        {
                            tracing::error!("Telegram: failed to send generated image: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Telegram: failed to read image {}: {}", img_path, e);
                    }
                }
            }

            // Tool messages already sent individually — delete streaming placeholder
            if let Some(mid) = streaming_msg_id {
                let _ = bot.delete_message(msg.chat.id, mid).await;
            }

            // Send final response as a clean separate message
            let html = markdown_to_telegram_html(&text_only);
            if !html.is_empty() {
                for chunk in split_message(&html, 4096) {
                    match bot
                        .send_message(msg.chat.id, chunk.to_string())
                        .parse_mode(ParseMode::Html)
                        .await
                    {
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(
                                "Telegram: HTML send failed ({e}), retrying as plain text"
                            );
                            // Fallback: send as plain text (strip HTML tags)
                            let plain = chunk
                                .replace("<b>", "")
                                .replace("</b>", "")
                                .replace("<i>", "")
                                .replace("</i>", "")
                                .replace("<code>", "")
                                .replace("</code>", "")
                                .replace("<pre>", "")
                                .replace("</pre>", "")
                                .replace("&lt;", "<")
                                .replace("&gt;", ">")
                                .replace("&amp;", "&");
                            if let Err(e2) = bot.send_message(msg.chat.id, plain).await {
                                tracing::error!("Telegram: plain text send also failed: {e2}");
                            }
                        }
                    }
                }
            }

            // If input was voice AND TTS is enabled, also send voice note after text
            if is_voice && voice_config.tts_enabled {
                match crate::channels::voice::synthesize(&response.content, &voice_config).await {
                    Ok(audio_bytes) => {
                        bot.send_voice(msg.chat.id, InputFile::memory(audio_bytes))
                            .await?;
                    }
                    Err(e) => {
                        tracing::error!("Telegram: TTS error: {}", e);
                    }
                }
            }
        }
        Err(ref e) if matches!(e, crate::brain::agent::AgentError::Cancelled) => {
            tracing::info!("Telegram: agent call cancelled for session {}", session_id);
            // Silently clean up — user already received "Operation cancelled." from /stop
            if let Some(mid) = streaming_msg_id {
                let _ = bot.delete_message(msg.chat.id, mid).await;
            }
        }
        Err(e) => {
            tracing::error!("Telegram: agent error: {}", e);
            // If a streaming message was started, edit it to show the error
            if let Some(mid) = streaming_msg_id {
                let _ = bot
                    .edit_message_text(msg.chat.id, mid, format!("Error: {}", e))
                    .await;
            } else {
                bot.send_message(msg.chat.id, format!("Error: {}", e))
                    .await?;
            }
        }
    }

    Ok(())
}

/// Convert simple markdown (`*bold*`, `` `code` ``) to Telegram HTML.
pub(crate) fn md_to_html(s: &str) -> String {
    // Replace `code` with <code>code</code>, then *bold* with <b>bold</b>
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' {
            let code: String = chars.by_ref().take_while(|&ch| ch != '`').collect();
            out.push_str("<code>");
            out.push_str(&code);
            out.push_str("</code>");
        } else if c == '*' {
            let bold: String = chars.by_ref().take_while(|&ch| ch != '*').collect();
            out.push_str("<b>");
            out.push_str(&bold);
            out.push_str("</b>");
        } else {
            out.push(c);
        }
    }
    out
}

/// Shorthand — delegates to the shared utility in `crate::utils`.
fn tool_context(name: &str, input: &serde_json::Value) -> String {
    crate::utils::tool_context_hint(name, input)
}

/// Convert markdown to Telegram-safe HTML.
/// Handles: code blocks, inline code, bold, italic, underscore italic,
/// strikethrough, headers, links, and list items. Escapes HTML entities.
pub(crate) fn markdown_to_telegram_html(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 256);
    let mut in_code_block = false;
    let mut code_lang;

    for line in text.lines() {
        if line.starts_with("```") {
            if in_code_block {
                result.push_str("</code></pre>\n");
                in_code_block = false;
            } else {
                code_lang = line.trim_start_matches('`').trim().to_string();
                if code_lang.is_empty() {
                    result.push_str("<pre><code>");
                } else {
                    result.push_str(&format!(
                        "<pre><code class=\"language-{}\">",
                        escape_html(&code_lang)
                    ));
                }
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            result.push_str(&escape_html(line));
            result.push('\n');
            continue;
        }

        // Headers: # → bold
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let content = trimmed.trim_start_matches('#').trim();
            let escaped = escape_html(content);
            result.push_str(&format!("<b>{}</b>\n", format_inline(&escaped)));
            continue;
        }

        // List items: - or * at start of line → bullet
        if (trimmed.starts_with("- ") || trimmed.starts_with("* ")) && trimmed.len() > 2 {
            let content = &trimmed[2..];
            let escaped = escape_html(content);
            // Preserve leading indent
            let indent = line.len() - trimmed.len();
            let spaces = &line[..indent];
            result.push_str(&format!(
                "{}• {}\n",
                escape_html(spaces),
                format_inline(&escaped)
            ));
            continue;
        }

        let escaped = escape_html(line);
        let formatted = format_inline(&escaped);
        result.push_str(&formatted);
        result.push('\n');
    }

    if in_code_block {
        result.push_str("</code></pre>\n");
    }

    result.trim_end().to_string()
}

/// Escape HTML special characters
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Apply inline formatting: `code`, **bold**, *italic*, _italic_, ~~strikethrough~~, [text](url)
fn format_inline(text: &str) -> String {
    // First pass: convert markdown links [text](url) → <a href="url">text</a>
    // Links are processed first because their syntax contains special chars
    let text = convert_links(text);

    let mut result = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '`') {
                let code: String = chars[i + 1..i + 1 + end].iter().collect();
                result.push_str(&format!("<code>{}</code>", code));
                i += end + 2;
                continue;
            }
        } else if chars[i] == '~' && i + 1 < chars.len() && chars[i + 1] == '~' {
            // ~~strikethrough~~
            if let Some(end) = find_closing_marker(&chars[i + 2..], &['~', '~']) {
                let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                result.push_str(&format!("<s>{}</s>", inner));
                i += end + 4;
                continue;
            }
        } else if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            // **bold**
            if let Some(end) = find_closing_marker(&chars[i + 2..], &['*', '*']) {
                let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                result.push_str(&format!("<b>{}</b>", inner));
                i += end + 4;
                continue;
            }
        } else if chars[i] == '_' && i + 1 < chars.len() && chars[i + 1] == '_' {
            // __bold__ (underscore bold)
            if let Some(end) = find_closing_marker(&chars[i + 2..], &['_', '_']) {
                let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                result.push_str(&format!("<b>{}</b>", inner));
                i += end + 4;
                continue;
            }
        } else if chars[i] == '*' {
            // *italic*
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '*') {
                let inner: String = chars[i + 1..i + 1 + end].iter().collect();
                result.push_str(&format!("<i>{}</i>", inner));
                i += end + 2;
                continue;
            }
        } else if chars[i] == '_' {
            // _italic_ — only match if not part of a word (e.g. my_var should stay)
            let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
            if !prev_alnum && let Some(end) = chars[i + 1..].iter().position(|&c| c == '_') {
                let next_alnum =
                    i + 1 + end + 1 < chars.len() && chars[i + 1 + end + 1].is_alphanumeric();
                if !next_alnum && end > 0 {
                    let inner: String = chars[i + 1..i + 1 + end].iter().collect();
                    result.push_str(&format!("<i>{}</i>", inner));
                    i += end + 2;
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Convert markdown links [text](url) to Telegram HTML <a> tags.
/// Operates on already-HTML-escaped text, so we must unescape the URL.
fn convert_links(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        result.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        if let Some(close) = after_open.find("](") {
            let link_text = &after_open[..close];
            let after_paren = &after_open[close + 2..];
            if let Some(end_paren) = after_paren.find(')') {
                let url = &after_paren[..end_paren];
                // Unescape HTML entities in URL (escape_html ran before format_inline)
                let clean_url = url
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">");
                result.push_str(&format!("<a href=\"{}\">{}</a>", clean_url, link_text));
                rest = &after_paren[end_paren + 1..];
                continue;
            }
        }
        // Not a valid link, emit the '[' and continue
        result.push('[');
        rest = after_open;
    }
    result.push_str(rest);
    result
}

/// Find closing double-char marker (e.g. **) in a char slice
fn find_closing_marker(chars: &[char], marker: &[char]) -> Option<usize> {
    if marker.len() != 2 {
        return None;
    }
    (0..chars.len().saturating_sub(1)).find(|&i| chars[i] == marker[0] && chars[i + 1] == marker[1])
}

/// Split a message into chunks that fit Telegram's 4096 char limit
pub(crate) fn split_message(text: &str, max_len: usize) -> Vec<&str> {
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

/// Build an `ApprovalCallback` that sends an inline-keyboard message to Telegram
/// and waits (up to 5 min) for the user to tap Yes, Always, or No.
pub(crate) fn make_approval_callback(
    state: Arc<super::TelegramState>,
) -> crate::brain::agent::ApprovalCallback {
    use crate::brain::agent::ToolApprovalInfo;
    use crate::utils::{check_approval_policy, persist_auto_session_policy};
    use teloxide::payloads::SendMessageSetters;
    use teloxide::prelude::Requester;
    use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
    use tokio::sync::oneshot;

    Arc::new(move |info: ToolApprovalInfo| {
        let state = state.clone();
        Box::pin(async move {
            // Respect config-level approval policy (single source of truth)
            if let Some(result) = check_approval_policy() {
                return Ok(result);
            }

            // Find the chat this session is active in
            let chat_id = match state.session_chat(info.session_id).await {
                Some(id) => id,
                None => match state.owner_chat_id().await {
                    Some(id) => id,
                    None => {
                        tracing::warn!(
                            "Telegram approval: no chat_id for session {}",
                            info.session_id
                        );
                        return Ok((false, false));
                    }
                },
            };

            let bot = match state.bot().await {
                Some(b) => b,
                None => {
                    tracing::warn!("Telegram approval: bot not connected");
                    return Ok((false, false));
                }
            };

            // Build unique approval id
            let approval_id = uuid::Uuid::new_v4().to_string();

            // Build inline keyboard — Yes / Always (session) / YOLO (permanent) / No
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("✅ Yes", format!("approve:{}", approval_id)),
                    InlineKeyboardButton::callback(
                        "🔁 Always (session)",
                        format!("always:{}", approval_id),
                    ),
                ],
                vec![
                    InlineKeyboardButton::callback(
                        "🔥 YOLO (permanent)",
                        format!("yolo:{}", approval_id),
                    ),
                    InlineKeyboardButton::callback("❌ No", format!("deny:{}", approval_id)),
                ],
            ]);

            // Format message — redact secrets before display, truncate to fit Telegram limit
            let safe_input = crate::utils::redact_tool_input(&info.tool_input);
            let mut input_pretty = serde_json::to_string_pretty(&safe_input)
                .unwrap_or_else(|_| safe_input.to_string());
            if input_pretty.len() > 3500 {
                input_pretty.truncate(3500);
                input_pretty.push_str("\n... [truncated]");
            }
            let text = format!(
                "🔐 <b>Tool Approval Required</b>\n\nTool: <code>{}</code>\nInput:\n<pre>{}</pre>",
                info.tool_name,
                escape_html(&input_pretty),
            );

            // Register oneshot channel BEFORE sending the message to prevent
            // race condition where user clicks before registration completes
            let (tx, rx) = oneshot::channel();
            state
                .register_pending_approval(approval_id.clone(), tx)
                .await;
            tracing::info!(
                "Telegram approval: registered pending id={}, sending to chat={}",
                approval_id,
                chat_id
            );

            match bot
                .send_message(ChatId(chat_id), &text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        "Telegram approval: message sent, waiting for response (id={})",
                        approval_id
                    );
                }
                Err(e) => {
                    tracing::error!("Telegram approval: failed to send message: {}", e);
                    return Ok((false, false));
                }
            }

            // Wait up to 5 minutes
            match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                Ok(Ok((approved, always))) => {
                    tracing::info!(
                        "Telegram approval: user responded id={}, approved={}, always={}",
                        approval_id,
                        approved,
                        always
                    );
                    if always {
                        persist_auto_session_policy();
                    }
                    Ok((approved, always))
                }
                Ok(Err(_)) => {
                    tracing::warn!(
                        "Telegram approval: oneshot channel closed (id={})",
                        approval_id
                    );
                    Ok((false, false))
                }
                Err(_) => {
                    tracing::warn!(
                        "Telegram approval: 5-minute timeout — auto-denying (id={})",
                        approval_id
                    );
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
        let chunks = split_message("hello", 4096);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_long_message() {
        let text = "a\n".repeat(3000);
        let chunks = split_message(&text, 4096);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 4096);
        }
        let joined: String = chunks.into_iter().collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn test_split_no_newlines() {
        let text = "a".repeat(5000);
        let chunks = split_message(&text, 4096);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 4096);
        assert_eq!(chunks[1].len(), 904);
    }

    #[test]
    fn test_markdown_to_telegram_html_bold() {
        let html = markdown_to_telegram_html("**hello**");
        assert!(html.contains("<b>hello</b>"));
    }

    #[test]
    fn test_markdown_to_telegram_html_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let html = markdown_to_telegram_html(md);
        assert!(html.contains("<pre><code"));
        assert!(html.contains("fn main()"));
        assert!(html.contains("</code></pre>"));
    }

    #[test]
    fn test_markdown_to_telegram_html_inline_code() {
        let html = markdown_to_telegram_html("use `cargo build`");
        assert!(html.contains("<code>cargo build</code>"));
    }

    #[test]
    fn test_escape_html() {
        assert_eq!(
            escape_html("<script>alert('xss')</script>"),
            "&lt;script&gt;alert('xss')&lt;/script&gt;"
        );
        assert_eq!(escape_html("a & b"), "a &amp; b");
    }

    #[test]
    fn test_img_marker_format() {
        // Verify the <<IMG:path>> marker format used for photo attachments
        let path = "/tmp/tg_photo_abc.jpg";
        let caption = "What's in this image?";
        let text = format!("<<IMG:{}>> {}", path, caption);
        assert!(text.starts_with("<<IMG:"));
        assert!(text.contains(path));
        assert!(text.contains(caption));
    }
}
