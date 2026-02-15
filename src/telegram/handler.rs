//! Telegram Message Handler
//!
//! Processes incoming messages: text, voice (STT/TTS), allowlist enforcement.

use crate::config::VoiceConfig;
use crate::llm::agent::AgentService;
use crate::services::SessionService;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::InputFile;
use tokio::sync::Mutex;
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_message(
    bot: Bot,
    msg: Message,
    agent: Arc<AgentService>,
    session_svc: SessionService,
    allowed: Arc<HashSet<i64>>,
    extra_sessions: Arc<Mutex<HashMap<i64, Uuid>>>,
    voice_config: Arc<VoiceConfig>,
    openai_key: Arc<Option<String>>,
    bot_token: Arc<String>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
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
        tracing::info!("Telegram: /start from user {} ({})", user_id, user.first_name);
        return Ok(());
    }

    // Allowlist check -- reject non-allowed users
    if !allowed.contains(&user_id) {
        tracing::debug!("Telegram: ignoring message from non-allowed user {}", user_id);
        bot.send_message(msg.chat.id, "You are not authorized. Send /start to get your user ID.")
            .await?;
        return Ok(());
    }

    // Extract text from either text message or voice note (via STT)
    let (text, is_voice) = if let Some(t) = msg.text() {
        if t.is_empty() {
            return Ok(());
        }
        (t.to_string(), false)
    } else if let Some(voice) = msg.voice() {
        // Voice note -- transcribe via Groq Whisper
        if !voice_config.stt_enabled {
            bot.send_message(msg.chat.id, "Voice notes are not enabled.")
                .await?;
            return Ok(());
        }

        let groq_key = match &voice_config.groq_api_key {
            Some(key) => key.clone(),
            None => {
                tracing::warn!("Telegram: voice note received but no GROQ_API_KEY configured");
                bot.send_message(msg.chat.id, "Voice transcription not configured (missing GROQ_API_KEY).")
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

        // Transcribe with Groq Whisper
        match crate::voice::transcribe_audio(audio_bytes, &groq_key).await {
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
    } else {
        // Non-text, non-voice message -- ignore
        return Ok(());
    };

    tracing::info!(
        "Telegram: {} from user {} ({}): {}",
        if is_voice { "voice" } else { "text" },
        user_id,
        user.first_name,
        &text[..text.len().min(50)]
    );

    // Resolve session: owner shares the TUI session, other users get their own
    let is_owner = allowed.len() == 1 || allowed.iter().next() == Some(&user_id);

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
        match map.get(&user_id) {
            Some(id) => *id,
            None => {
                let title = format!("Telegram: {}", user.first_name);
                match session_svc.create_session(Some(title)).await {
                    Ok(session) => {
                        map.insert(user_id, session.id);
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
    };

    // Send to agent (with tools so the agent can use file ops, search, etc.)
    match agent.send_message_with_tools(session_id, text, None).await {
        Ok(response) => {
            // Always send text reply first (keeps chat searchable)
            let html = markdown_to_telegram_html(&response.content);
            for chunk in split_message(&html, 4096) {
                bot.send_message(msg.chat.id, chunk)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await?;
            }

            // If input was voice AND TTS is enabled, also send voice note after text
            if is_voice && voice_config.tts_enabled
                && let Some(ref oai_key) = *openai_key
            {
                match crate::voice::synthesize_speech(
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
            bot.send_message(msg.chat.id, format!("Error: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Convert markdown to Telegram-safe HTML
/// Handles: code blocks, inline code, bold, italic. Escapes HTML entities.
fn markdown_to_telegram_html(text: &str) -> String {
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
    (0..chars.len().saturating_sub(1))
        .find(|&i| chars[i] == marker[0] && chars[i + 1] == marker[1])
}

/// Split a message into chunks that fit Telegram's 4096 char limit
fn split_message(text: &str, max_len: usize) -> Vec<&str> {
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
}
