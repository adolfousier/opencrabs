//! Discord Message Handler
//!
//! Processes incoming Discord messages: text + image attachments, allowlist enforcement,
//! session routing (owner shares TUI session, others get per-user sessions).

use super::DiscordState;
use crate::brain::agent::AgentService;
use crate::config::{RespondTo, VoiceConfig};
use crate::services::SessionService;
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
    allowed: Arc<HashSet<i64>>,
    extra_sessions: Arc<Mutex<HashMap<u64, (Uuid, std::time::Instant)>>>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    discord_state: Arc<DiscordState>,
    respond_to: &RespondTo,
    allowed_channels: &HashSet<String>,
    voice_config: Arc<VoiceConfig>,
    openai_key: Arc<Option<String>>,
    idle_timeout_hours: Option<f64>,
) {
    let user_id = msg.author.id.get() as i64;

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
            return;
        }

        match respond_to {
            RespondTo::DmOnly => {
                tracing::debug!("Discord: respond_to=dm_only, ignoring channel message");
                return;
            }
            RespondTo::Mention => {
                let bot_id = discord_state.bot_user_id().await;
                let mentioned =
                    bot_id.is_some_and(|bid| msg.mentions.iter().any(|u| u.id.get() == bid));
                if !mentioned {
                    tracing::debug!("Discord: respond_to=mention, bot not mentioned — ignoring");
                    return;
                }
            }
            RespondTo::All => {} // pass through
        }
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
                    &transcript[..transcript.len().min(80)]
                );
                content = transcript;
                is_voice = true;
            }
            Err(e) => tracing::error!("Discord: STT error: {e}"),
        }
    }

    // Strip bot @mention from content when responding to a mention
    if !is_dm
        && *respond_to == RespondTo::Mention
        && let Some(bot_id) = discord_state.bot_user_id().await
    {
        let mention_tag = format!("<@{}>", bot_id);
        content = content.replace(&mention_tag, "").trim().to_string();
    }
    if content.is_empty() && msg.attachments.is_empty() {
        return;
    }

    // Handle image attachments — append <<IMG:url>> markers
    if !is_voice {
        for attachment in &msg.attachments {
            if let Some(ref content_type) = attachment.content_type
                && content_type.starts_with("image/")
            {
                if content.is_empty() {
                    content = "Describe this image.".to_string();
                }
                content.push_str(&format!(" <<IMG:{}>>", attachment.url));
            }
        }
    }

    if content.is_empty() {
        return;
    }

    let text_preview = &content[..content.len().min(50)];
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
                tracing::warn!("Discord: no active TUI session, creating one for owner");
                drop(shared);
                match session_svc.create_session(Some("Chat".to_string())).await {
                    Ok(session) => {
                        *shared_session.lock().await = Some(session.id);
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("Discord: failed to create session: {}", e);
                        return;
                    }
                }
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

    // Register channel for approval routing, then send with approval callback
    discord_state
        .register_session_channel(session_id, msg.channel_id.get())
        .await;
    let approval_cb = DiscordState::make_approval_callback(discord_state.clone());

    match agent
        .send_message_with_tools_and_callback(
            session_id,
            agent_input,
            None,
            None,
            Some(approval_cb),
            None,
        )
        .await
    {
        Ok(response) => {
            let tagged = response.content.clone();
            for chunk in split_message(&tagged, 2000) {
                if let Err(e) = msg.channel_id.say(&ctx.http, chunk).await {
                    tracing::error!("Discord: failed to send reply: {}", e);
                }
            }

            // TTS: send voice reply if input was audio and TTS is enabled
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
