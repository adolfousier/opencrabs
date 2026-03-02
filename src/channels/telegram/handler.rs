//! Telegram Message Handler
//!
//! Processes incoming messages: text, voice (STT/TTS), photos, image documents, allowlist enforcement.
//! Supports live streaming (edit-based) and Telegram-native approval inline keyboards.

use super::TelegramState;
use crate::brain::agent::{AgentService, ProgressCallback, ProgressEvent};
use crate::config::{RespondTo, VoiceConfig};
use crate::services::SessionService;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{ChatAction, ChatKind, InputFile, MessageId, ParseMode};
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

/// Per-message streaming state shared between the progress callback and the edit loop.
struct StreamingState {
    msg_id: Option<MessageId>,
    text: String,
    dirty: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_message(
    bot: Bot,
    msg: Message,
    agent: Arc<AgentService>,
    session_svc: SessionService,
    allowed: Arc<HashSet<i64>>,
    extra_sessions: Arc<Mutex<HashMap<i64, (Uuid, std::time::Instant)>>>,
    voice_config: Arc<VoiceConfig>,
    openai_key: Arc<Option<String>>,
    bot_token: Arc<String>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    telegram_state: Arc<TelegramState>,
    respond_to: &RespondTo,
    allowed_channels: &HashSet<String>,
    idle_timeout_hours: Option<f64>,
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

    // Allowlist check -- reject non-allowed users
    if !allowed.contains(&user_id) {
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
    if !is_dm {
        let chat_id_str = msg.chat.id.0.to_string();

        // Check allowed_channels (empty = all channels allowed)
        if !allowed_channels.is_empty() && !allowed_channels.contains(&chat_id_str) {
            tracing::debug!(
                "Telegram: ignoring message in non-allowed chat {}",
                chat_id_str
            );
            return Ok(());
        }

        match respond_to {
            RespondTo::DmOnly => {
                tracing::debug!("Telegram: respond_to=dm_only, ignoring group message");
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

                if !mentioned_by_username && !replied_to_bot {
                    tracing::debug!("Telegram: respond_to=mention, bot not mentioned — ignoring");
                    return Ok(());
                }
            }
            RespondTo::All => {} // pass through
        }
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

        let stt_key = match &voice_config.stt_provider {
            Some(provider) => match &provider.api_key {
                Some(key) => key.clone(),
                None => {
                    tracing::warn!("Telegram: voice note received but no STT API key configured");
                    bot.send_message(
                        msg.chat.id,
                        "Voice transcription not configured (missing API key).",
                    )
                    .await?;
                    return Ok(());
                }
            },
            None => {
                tracing::warn!("Telegram: voice note received but no STT provider configured");
                bot.send_message(msg.chat.id, "Voice transcription not configured.")
                    .await?;
                return Ok(());
            }
        };

        tracing::info!(
            "Telegram: voice note from user {} ({}) — {}s",
            user_id,
            user.first_name,
            voice.duration,
        );

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

        // Transcribe with STT provider
        match crate::channels::voice::transcribe_audio(audio_bytes, &stt_key).await {
            Ok(transcript) => {
                tracing::info!(
                    "Telegram: transcribed voice: {}",
                    &transcript[..transcript.len().min(80)]
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
        // Document -- check if it's an image by MIME type
        let is_image = doc
            .mime_type
            .as_ref()
            .is_some_and(|m| m.as_ref().starts_with("image/"));

        if !is_image {
            bot.send_message(msg.chat.id, "Only image files are supported for now.")
                .await?;
            return Ok(());
        }

        tracing::info!(
            "Telegram: image document from user {} ({})",
            user_id,
            user.first_name,
        );

        let file = bot.get_file(&doc.file.id).await?;
        let download_url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            bot_token.as_str(),
            file.path
        );

        let img_bytes = match reqwest::get(&download_url).await {
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

        // Determine extension from filename or default to jpg
        let ext = doc
            .file_name
            .as_ref()
            .and_then(|n| n.rsplit('.').next())
            .unwrap_or("jpg");

        let tmp_path = std::env::temp_dir().join(format!("tg_doc_{}.{}", Uuid::new_v4(), ext));
        if let Err(e) = tokio::fs::write(&tmp_path, &img_bytes).await {
            tracing::error!("Telegram: failed to write temp doc: {}", e);
            bot.send_message(msg.chat.id, "Failed to process file.")
                .await?;
            return Ok(());
        }

        let caption = msg.caption().unwrap_or("Analyze this image");
        let text_with_img = format!("<<IMG:{}>> {}", tmp_path.display(), caption);

        let cleanup_path = tmp_path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let _ = tokio::fs::remove_file(cleanup_path).await;
        });

        (text_with_img, false)
    } else {
        // Non-text, non-voice, non-photo message -- ignore
        return Ok(());
    };

    // Strip @bot_username from text when responding to a mention in groups
    let text = if !is_dm && *respond_to == RespondTo::Mention {
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
        &text[..text.len().min(50)]
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

    // Resolve session: owner shares the TUI session, other users get their own
    let is_owner = allowed.len() == 1 || allowed.iter().next() == Some(&user_id);

    // Track owner's chat ID for proactive messaging
    if is_owner {
        telegram_state.set_owner_chat_id(msg.chat.id.0).await;
    }

    let session_id = if is_owner {
        // Owner shares the TUI's current session
        let shared = shared_session.lock().await;
        match *shared {
            Some(id) => id,
            None => {
                tracing::warn!("Telegram: no active TUI session, creating one for owner");
                drop(shared); // release lock before async create
                match session_svc.create_session(Some("Chat".to_string())).await {
                    Ok(session) => {
                        *shared_session.lock().await = Some(session.id);
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
        }
    } else {
        // Non-owner users get their own separate sessions
        let mut map = extra_sessions.lock().await;
        if let Some((old_id, last_activity)) = map.get(&user_id).copied() {
            if idle_timeout_hours
                .is_some_and(|h| last_activity.elapsed().as_secs() > (h * 3600.0) as u64)
            {
                let _ = session_svc.archive_session(old_id).await;
                map.remove(&user_id);
                let title = format!("Telegram: {}", user.first_name);
                match session_svc.create_session(Some(title)).await {
                    Ok(session) => {
                        map.insert(user_id, (session.id, std::time::Instant::now()));
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("Telegram: failed to create session: {}", e);
                        bot.send_message(msg.chat.id, "Internal error creating session.")
                            .await?;
                        return Ok(());
                    }
                }
            } else {
                map.insert(user_id, (old_id, std::time::Instant::now()));
                old_id
            }
        } else {
            let title = format!("Telegram: {}", user.first_name);
            match session_svc.create_session(Some(title)).await {
                Ok(session) => {
                    map.insert(user_id, (session.id, std::time::Instant::now()));
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

    // Register session → chat for approval routing
    telegram_state
        .register_session_chat(session_id, msg.chat.id.0)
        .await;

    // For non-owner users, prepend sender identity so the agent knows who
    // it's talking to and doesn't assume it's the owner.
    let agent_input = if !is_owner {
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
            format!("[Telegram DM from {name}{handle}, ID {user_id}]\n{text}")
        } else {
            let chat_title = msg.chat.title().unwrap_or("group");
            format!(
                "[Telegram message from {name}{handle}, ID {user_id} in group {chat_title}]\n{text}"
            )
        }
    } else {
        text
    };

    // ── Streaming setup ───────────────────────────────────────────────────────
    let streaming = Arc::new(Mutex::new(StreamingState {
        msg_id: None,
        text: String::new(),
        dirty: false,
    }));

    let edit_cancel = CancellationToken::new();

    // Edit loop: posts/edits a message every 1.5 s while response streams in
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
                        if !s.dirty { continue; }
                        if s.msg_id.is_none()
                            && let Ok(m) = bot.send_message(chat, "\u{258b}").await
                        {
                            s.msg_id = Some(m.id);
                        }
                        if let Some(mid) = s.msg_id {
                            let html = markdown_to_telegram_html(&s.text);
                            let display = format!("{}\u{258b}", html); // ▋ cursor
                            let _ = bot
                                .edit_message_text(chat, mid, display)
                                .parse_mode(ParseMode::Html)
                                .await;
                        }
                        s.dirty = false;
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
                ProgressEvent::StreamingChunk { text } => {
                    if let Ok(mut s) = st.try_lock() {
                        s.text.push_str(&text);
                        s.dirty = true;
                    }
                }
                ProgressEvent::ToolStarted {
                    tool_name,
                    tool_input,
                } => {
                    if let Ok(mut s) = st.try_lock() {
                        let ctx = tool_context(&tool_name, &tool_input);
                        s.text.push_str(&format!("\n\n⚙️ _{tool_name}_{ctx}"));
                        s.dirty = true;
                    }
                }
                ProgressEvent::IntermediateText { text, .. } => {
                    // Text produced between tool-call batches — show it while waiting
                    if let Ok(mut s) = st.try_lock() {
                        // Only append if this text isn't already in the buffer
                        // (avoids duplication when StreamingChunks already accumulated it)
                        if !s.text.contains(&text) {
                            s.text.push_str(&text);
                            s.dirty = true;
                        }
                    }
                }
                _ => {}
            }
        })
    };

    // Build Telegram-native approval callback for this session
    let approval_cb = TelegramState::make_approval_callback(telegram_state.clone());

    // ── Agent call ────────────────────────────────────────────────────────────
    let result = agent
        .send_message_with_tools_and_callback(
            session_id,
            agent_input.clone(),
            None,
            None,
            Some(approval_cb),
            Some(progress_cb.clone()),
        )
        .await;

    // If session lookup failed (DB contention on restart), create a fresh session and retry once
    let result = if let Err(ref e) = result {
        let es = e.to_string();
        if es.contains("Failed to get session") || es.contains("SessionNotFound") {
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
                    let approval_cb2 =
                        TelegramState::make_approval_callback(telegram_state.clone());
                    agent
                        .send_message_with_tools_and_callback(
                            new_id,
                            agent_input,
                            None,
                            None,
                            Some(approval_cb2),
                            Some(progress_cb),
                        )
                        .await
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

    // Stop edit loop — final content will be written below
    edit_cancel.cancel();
    // _typing_guard drop cancels typing loop

    // Grab streaming message id (if any was created during streaming)
    let streaming_msg_id = streaming.lock().await.msg_id;

    // ── Final response ────────────────────────────────────────────────────────
    match result {
        Ok(response) => {
            let html = markdown_to_telegram_html(&response.content);
            if let Some(mid) = streaming_msg_id {
                if html.len() <= 4096 {
                    // Edit streaming placeholder to final content (no cursor)
                    let _ = bot
                        .edit_message_text(msg.chat.id, mid, &html)
                        .parse_mode(ParseMode::Html)
                        .await;
                } else {
                    // Too long: delete placeholder, send split chunks
                    let _ = bot.delete_message(msg.chat.id, mid).await;
                    for chunk in split_message(&html, 4096) {
                        bot.send_message(msg.chat.id, chunk)
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                }
            } else if !html.is_empty() {
                // No streaming started (e.g. tool-only response with no text output)
                for chunk in split_message(&html, 4096) {
                    bot.send_message(msg.chat.id, chunk)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }

            // If input was voice AND TTS is enabled, also send voice note after text
            if is_voice
                && voice_config.tts_enabled
                && let Some(ref oai_key) = *openai_key
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
                        bot.send_voice(msg.chat.id, InputFile::memory(audio_bytes))
                            .await?;
                    }
                    Err(e) => {
                        tracing::error!("Telegram: TTS error: {}", e);
                    }
                }
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

/// Extract a short, meaningful context hint from a tool's input for display.
/// Runs the input through the secret sanitizer first so no API keys or tokens
/// can leak into the streaming indicator via command or url fields.
fn tool_context(name: &str, input: &serde_json::Value) -> String {
    let safe = crate::utils::redact_tool_input(input);
    let hint: Option<String> = match name {
        "bash" => safe
            .get("command")
            .and_then(|v| v.as_str())
            .map(String::from),
        "read" | "write" | "edit" => safe.get("path").and_then(|v| v.as_str()).map(String::from),
        "glob" => safe
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(String::from),
        "grep" => safe
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(String::from),
        "ls" => safe.get("path").and_then(|v| v.as_str()).map(String::from),
        "http_request" | "web_fetch" => safe.get("url").and_then(|v| v.as_str()).map(String::from),
        "brave_search" | "exa_search" | "web_search" | "memory_search" | "session_search" => {
            safe.get("query").and_then(|v| v.as_str()).map(String::from)
        }
        "telegram_send" | "discord_send" | "slack_send" | "trello_send" => safe
            .get("action")
            .and_then(|v| v.as_str())
            .map(String::from),
        // Fallback: first string value in the object
        _ => safe
            .as_object()
            .and_then(|m| m.values().find_map(|v| v.as_str().map(String::from))),
    };
    match hint {
        Some(h) if !h.is_empty() => {
            let truncated = &h[..h.len().min(60)];
            format!("(`{truncated}`)")
        }
        _ => String::new(),
    }
}

/// Convert markdown to Telegram-safe HTML
/// Handles: code blocks, inline code, bold, italic. Escapes HTML entities.
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

/// Apply inline formatting: `code`, **bold**, *italic*
fn format_inline(text: &str) -> String {
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
        } else if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if let Some(end) = find_closing_marker(&chars[i + 2..], &['*', '*']) {
                let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                result.push_str(&format!("<b>{}</b>", inner));
                i += end + 4;
                continue;
            }
        } else if chars[i] == '*'
            && let Some(end) = chars[i + 1..].iter().position(|&c| c == '*')
        {
            let inner: String = chars[i + 1..i + 1 + end].iter().collect();
            result.push_str(&format!("<i>{}</i>", inner));
            i += end + 2;
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
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
