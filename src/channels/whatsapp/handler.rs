//! WhatsApp Message Handler
//!
//! Processes incoming WhatsApp messages: text + images, allowlist enforcement,
//! session routing (owner shares TUI session, others get per-phone sessions).

use crate::brain::agent::AgentService;
use crate::brain::agent::ApprovalCallback;
use crate::channels::whatsapp::WhatsAppState;
use crate::config::VoiceConfig;
use crate::services::SessionService;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use tokio_util::sync::CancellationToken;
use wacore::types::message::MessageInfo;
use waproto::whatsapp::Message;
use whatsapp_rust::client::Client;

/// Header prepended to all outgoing messages so the user knows it's from the agent.
pub const MSG_HEADER: &str = "\u{1f980} *OpenCrabs*";

/// Unwrap nested message wrappers (device_sent, ephemeral, view_once, etc.)
/// Returns the innermost Message that contains actual content.
fn unwrap_message(msg: &Message) -> &Message {
    // device_sent_message: wraps messages synced across linked devices
    if let Some(ref dsm) = msg.device_sent_message
        && let Some(ref inner) = dsm.message
    {
        return unwrap_message(inner);
    }
    // ephemeral_message: disappearing messages
    if let Some(ref eph) = msg.ephemeral_message
        && let Some(ref inner) = eph.message
    {
        return unwrap_message(inner);
    }
    // view_once_message
    if let Some(ref vo) = msg.view_once_message
        && let Some(ref inner) = vo.message
    {
        return unwrap_message(inner);
    }
    // document_with_caption_message
    if let Some(ref dwc) = msg.document_with_caption_message
        && let Some(ref inner) = dwc.message
    {
        return unwrap_message(inner);
    }
    msg
}

/// Extract plain text from a WhatsApp message.
fn extract_text(msg: &Message) -> Option<String> {
    let msg = unwrap_message(msg);
    // Try conversation field first (simple text messages)
    if let Some(ref conv) = msg.conversation
        && !conv.is_empty()
    {
        return Some(conv.clone());
    }
    // Try extended text message (messages with link previews, etc.)
    if let Some(ref ext) = msg.extended_text_message
        && let Some(ref text) = ext.text
    {
        return Some(text.clone());
    }
    // Try image caption
    if let Some(ref img) = msg.image_message
        && let Some(ref caption) = img.caption
        && !caption.is_empty()
    {
        return Some(caption.clone());
    }
    None
}

/// Check if the message has a downloadable image.
fn has_image(msg: &Message) -> bool {
    let msg = unwrap_message(msg);
    msg.image_message.is_some()
}

/// Check if the message has a downloadable audio/voice note.
fn has_audio(msg: &Message) -> bool {
    let msg = unwrap_message(msg);
    msg.audio_message.is_some()
}

/// Download audio from WhatsApp. Returns raw bytes on success.
async fn download_audio(msg: &Message, client: &Client) -> Option<Vec<u8>> {
    let msg = unwrap_message(msg);
    let audio = msg.audio_message.as_ref()?;
    match client.download(audio.as_ref()).await {
        Ok(bytes) => {
            tracing::debug!("WhatsApp: downloaded audio ({} bytes)", bytes.len());
            Some(bytes)
        }
        Err(e) => {
            tracing::error!("WhatsApp: failed to download audio: {e}");
            None
        }
    }
}

/// Download image from WhatsApp and save to a temp file.
/// Returns the file path on success.
async fn download_image(msg: &Message, client: &Client) -> Option<String> {
    let msg = unwrap_message(msg);
    let img = msg.image_message.as_ref()?;

    let mime = img.mimetype.as_deref().unwrap_or("image/jpeg");
    let ext = match mime {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    };

    match client.download(img.as_ref()).await {
        Ok(bytes) => {
            let path =
                std::env::temp_dir().join(format!("wa_img_{}.{}", uuid::Uuid::new_v4(), ext));
            match std::fs::write(&path, &bytes) {
                Ok(()) => {
                    tracing::debug!(
                        "WhatsApp: downloaded image ({} bytes) to {}",
                        bytes.len(),
                        path.display()
                    );
                    Some(path.to_string_lossy().to_string())
                }
                Err(e) => {
                    tracing::error!("WhatsApp: failed to save image: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            tracing::error!("WhatsApp: failed to download image: {}", e);
            None
        }
    }
}

/// Extract the sender's phone number (digits only) from message info.
/// JID format is "351933536442@s.whatsapp.net" or "351933536442:34@s.whatsapp.net"
/// (linked device suffix) — we return just "351933536442" in both cases.
fn sender_phone(info: &MessageInfo) -> String {
    let full = info.source.sender.to_string();
    let without_server = full.split('@').next().unwrap_or(&full);
    // Strip linked-device suffix (e.g. ":34" for WhatsApp Web/Desktop)
    without_server
        .split(':')
        .next()
        .unwrap_or(without_server)
        .to_string()
}

/// Split a message into chunks that fit WhatsApp's limit (~65536 chars, but we use 4000 for readability).
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
    msg: Message,
    info: MessageInfo,
    client: Arc<Client>,
    agent: Arc<AgentService>,
    session_svc: SessionService,
    allowed: Arc<HashSet<String>>,
    extra_sessions: Arc<Mutex<HashMap<String, (Uuid, std::time::Instant)>>>,
    voice_config: Arc<VoiceConfig>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    idle_timeout_hours: Option<f64>,
    wa_state: Arc<WhatsAppState>,
) {
    let phone = sender_phone(&info);
    tracing::debug!(
        "WhatsApp handler: from={}, is_from_me={}, has_text={}, has_image={}, has_audio={}",
        phone,
        info.source.is_from_me,
        extract_text(&msg).is_some(),
        has_image(&msg),
        has_audio(&msg),
    );

    // Skip bot's own outgoing replies (they echo back as is_from_me).
    // User messages from their phone are also is_from_me (same account),
    // so we only skip if the text starts with our agent header.
    // Never skip audio/image — those are real user messages even when is_from_me.
    if info.source.is_from_me {
        if let Some(text) = extract_text(&msg) {
            if text.starts_with(MSG_HEADER) {
                return;
            }
        } else if !has_audio(&msg) && !has_image(&msg) {
            // No text, no audio, no image and is_from_me — non-content echo, skip
            return;
        }
    }

    // Build message content: text, image, or audio
    let has_img = has_image(&msg);
    let has_aud = has_audio(&msg);
    let text = extract_text(&msg);

    // Require at least text, image, or audio
    if text.is_none() && !has_img && !has_aud {
        return;
    }

    // Allowlist check — if allowed list is empty, accept all.
    // Normalize: strip '+' from allowed entries to match JID digits.
    if !allowed.is_empty() && !allowed.iter().any(|a| a.trim_start_matches('+') == phone) {
        tracing::debug!(
            "WhatsApp: ignoring message from non-allowed phone {}",
            phone
        );
        return;
    }

    // Pending approval check: if a tool approval is waiting for this phone,
    // interpret this message as Yes / Always / No instead of routing to the agent.
    // Handles both button taps (ButtonsResponseMessage) and plain text replies.
    {
        use crate::channels::whatsapp::WaApproval;

        let btn_id = unwrap_message(&msg)
            .buttons_response_message
            .as_ref()
            .and_then(|b| b.selected_button_id.as_deref());

        let choice: Option<WaApproval> = if let Some(id) = btn_id {
            match id {
                "wa_approve_yes" => Some(WaApproval::Yes),
                "wa_approve_always" => Some(WaApproval::Always),
                "wa_approve_no" => Some(WaApproval::No),
                _ => None,
            }
        } else if let Some(raw_text) = extract_text(&msg) {
            let answer = raw_text.trim().to_lowercase();
            if matches!(answer.as_str(), "yes" | "y" | "sim" | "s") {
                Some(WaApproval::Yes)
            } else if matches!(answer.as_str(), "always" | "sempre") {
                Some(WaApproval::Always)
            } else if matches!(answer.as_str(), "no" | "n" | "nao" | "não") {
                Some(WaApproval::No)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(c) = choice
            && wa_state.resolve_pending_approval(&phone, c).await.is_some()
        {
            tracing::info!("WhatsApp: approval from {}: {:?}", phone, c);
            if c == WaApproval::Always {
                wa_state.set_auto_approve_session().await;
            }
            return;
        }
    }

    let text_preview = text
        .as_deref()
        .map(|t| &t[..t.len().min(50)])
        .unwrap_or("[image]");
    tracing::info!("WhatsApp: message from {}: {}", phone, text_preview);

    // Audio/voice note → STT transcription
    let mut content;
    if has_aud
        && voice_config.stt_enabled
        && let Some(ref stt_provider) = voice_config.stt_provider
        && let Some(ref stt_key) = stt_provider.api_key
        && let Some(audio_bytes) = download_audio(&msg, &client).await
    {
        match crate::channels::voice::transcribe_audio(audio_bytes, stt_key).await {
            Ok(transcript) => {
                tracing::info!(
                    "WhatsApp: transcribed voice: {}",
                    &transcript[..transcript.len().min(80)]
                );
                content = transcript;
            }
            Err(e) => {
                tracing::error!("WhatsApp: STT error: {e}");
                content = text.unwrap_or_default();
            }
        }
    } else {
        content = text.unwrap_or_default();
    }

    // Download image if present, append <<IMG:path>> marker
    if has_img
        && !has_aud
        && let Some(img_path) = download_image(&msg, &client).await
    {
        if content.is_empty() {
            content = "Describe this image.".to_string();
        }
        content.push_str(&format!(" <<IMG:{}>>", img_path));
    }

    if content.is_empty() {
        return;
    }

    // Resolve session: owner (first in allowed list) shares TUI session, others get their own
    let is_owner = allowed.is_empty()
        || allowed
            .iter()
            .next()
            .map(|a| a.trim_start_matches('+') == phone)
            .unwrap_or(false);

    let session_id = if is_owner {
        let shared = shared_session.lock().await;
        match *shared {
            Some(id) => id,
            None => {
                tracing::warn!("WhatsApp: no active TUI session, creating one for owner");
                drop(shared);
                match session_svc.create_session(Some("Chat".to_string())).await {
                    Ok(session) => {
                        *shared_session.lock().await = Some(session.id);
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("WhatsApp: failed to create session: {}", e);
                        return;
                    }
                }
            }
        }
    } else {
        let mut map = extra_sessions.lock().await;
        if let Some((old_id, last_activity)) = map.get(&phone).copied() {
            if idle_timeout_hours
                .is_some_and(|h| last_activity.elapsed().as_secs() > (h * 3600.0) as u64)
            {
                let _ = session_svc.archive_session(old_id).await;
                map.remove(&phone);
                let title = format!("WhatsApp: {}", phone);
                match session_svc.create_session(Some(title)).await {
                    Ok(session) => {
                        map.insert(phone.clone(), (session.id, std::time::Instant::now()));
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("WhatsApp: failed to create session: {}", e);
                        return;
                    }
                }
            } else {
                map.insert(phone.clone(), (old_id, std::time::Instant::now()));
                old_id
            }
        } else {
            let title = format!("WhatsApp: {}", phone);
            match session_svc.create_session(Some(title)).await {
                Ok(session) => {
                    map.insert(phone.clone(), (session.id, std::time::Instant::now()));
                    session.id
                }
                Err(e) => {
                    tracing::error!("WhatsApp: failed to create session: {}", e);
                    return;
                }
            }
        }
    };

    // For non-owner contacts, prepend sender identity so the agent knows who
    // it's talking to and doesn't assume it's the owner messaging themselves.
    let agent_input = if !is_owner {
        let name = info.push_name.trim().to_string();
        let from = if name.is_empty() {
            format!("+{}", phone)
        } else {
            format!("{} (+{})", name, phone)
        };
        if info.source.is_group {
            let group = info.source.chat.to_string();
            let group_id = group.split('@').next().unwrap_or(&group);
            format!(
                "[WhatsApp group message from {} in group {}]\n{}",
                from, group_id, content
            )
        } else {
            format!("[WhatsApp message from {}]\n{}", from, content)
        }
    } else {
        content
    };

    // Typing indicator — send composing every 5 s while the agent thinks
    let typing_cancel = CancellationToken::new();
    tokio::spawn({
        let client = client.clone();
        let chat_jid = info.source.chat.clone();
        let cancel = typing_cancel.clone();
        async move {
            loop {
                let _ = client.chatstate().send_composing(&chat_jid).await;
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                }
            }
            let _ = client.chatstate().send_paused(&chat_jid).await;
        }
    });

    // Build per-call approval callback.
    // If the user previously chose "Always (session)", auto-approve without asking.
    // Otherwise send a 3-button message (Yes / Always / No) and wait up to 5 min.
    let approval_cb: ApprovalCallback = {
        use crate::channels::whatsapp::WaApproval;
        use waproto::whatsapp::message::{ButtonsMessage, buttons_message};

        let client = client.clone();
        let chat_jid = info.source.chat.clone();
        let phone_key = phone.clone();
        let wa_state = wa_state.clone();
        Arc::new(move |tool_info| {
            let client = client.clone();
            let chat_jid = chat_jid.clone();
            let phone_key = phone_key.clone();
            let wa_state = wa_state.clone();
            Box::pin(async move {
                // Auto-approve if user already chose "Always" this session
                if wa_state.is_auto_approve_session().await {
                    return Ok(true);
                }

                // Redact secrets before display
                let safe_input = crate::utils::redact_tool_input(&tool_info.tool_input);
                let input_preview =
                    serde_json::to_string_pretty(&safe_input).unwrap_or_default();
                let body = format!(
                    "🔐 *Tool Approval Required*\n\nTool: `{}`\n```\n{}\n```",
                    tool_info.tool_name,
                    &input_preview[..input_preview.len().min(600)],
                );

                // Try to send interactive buttons message
                let btn = |id: &str, label: &str| buttons_message::Button {
                    button_id: Some(id.to_string()),
                    button_text: Some(buttons_message::button::ButtonText {
                        display_text: Some(label.to_string()),
                    }),
                    r#type: Some(1), // Response
                    ..Default::default()
                };
                let buttons_msg = waproto::whatsapp::Message {
                    buttons_message: Some(Box::new(ButtonsMessage {
                        content_text: Some(body.clone()),
                        footer_text: Some("5 min timeout — no reply = deny".to_string()),
                        buttons: vec![
                            btn("wa_approve_yes", "✅ Yes"),
                            btn("wa_approve_always", "🔁 Always (session)"),
                            btn("wa_approve_no", "❌ No"),
                        ],
                        ..Default::default()
                    })),
                    ..Default::default()
                };

                if client
                    .send_message(chat_jid.clone(), buttons_msg)
                    .await
                    .is_err()
                {
                    // Fallback to plain text if buttons fail
                    let text_msg = waproto::whatsapp::Message {
                        conversation: Some(format!(
                            "{}\n\n{}\n\nReply *yes*, *always* (session), or *no* (5 min timeout).",
                            MSG_HEADER, body
                        )),
                        ..Default::default()
                    };
                    if let Err(e) = client.send_message(chat_jid, text_msg).await {
                        tracing::error!("WhatsApp: failed to send approval request: {}", e);
                        return Ok(false);
                    }
                }

                let (tx, rx) = tokio::sync::oneshot::channel::<WaApproval>();
                wa_state.register_pending_approval(phone_key, tx).await;

                match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                    Ok(Ok(WaApproval::Yes)) => Ok(true),
                    Ok(Ok(WaApproval::Always)) => {
                        wa_state.set_auto_approve_session().await;
                        Ok(true)
                    }
                    Ok(Ok(WaApproval::No)) => Ok(false),
                    _ => {
                        tracing::warn!("WhatsApp: approval timed out or channel dropped — denying");
                        Ok(false)
                    }
                }
            })
        })
    };

    // Send to agent with WhatsApp approval callback
    let result = agent
        .send_message_with_tools_and_callback(
            session_id,
            agent_input,
            None,
            None,
            Some(approval_cb),
            None,
        )
        .await;

    typing_cancel.cancel();

    match result {
        Ok(response) => {
            let reply_jid = info.source.chat.clone();
            let tagged = format!("{}\n\n{}", MSG_HEADER, response.content);
            for chunk in split_message(&tagged, 4000) {
                let reply_msg = waproto::whatsapp::Message {
                    conversation: Some(chunk.to_string()),
                    ..Default::default()
                };
                if let Err(e) = client.send_message(reply_jid.clone(), reply_msg).await {
                    tracing::error!("WhatsApp: failed to send reply: {}", e);
                }
            }
        }
        Err(e) => {
            tracing::error!("WhatsApp: agent error: {}", e);
            let error_msg = waproto::whatsapp::Message {
                conversation: Some(format!("{}\n\nError: {}", MSG_HEADER, e)),
                ..Default::default()
            };
            let _ = client
                .send_message(info.source.chat.clone(), error_msg)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_short_message() {
        let chunks = split_message("hello", 4000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_long_message() {
        let text = "a\n".repeat(3000);
        let chunks = split_message(&text, 4000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 4000);
        }
        let joined: String = chunks.into_iter().collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn test_extract_text_conversation() {
        let msg = Message {
            conversation: Some("hello".to_string()),
            ..Default::default()
        };
        assert_eq!(extract_text(&msg), Some("hello".to_string()));
    }

    #[test]
    fn test_extract_text_image_caption() {
        let msg = Message {
            image_message: Some(Box::new(waproto::whatsapp::message::ImageMessage {
                caption: Some("look at this".to_string()),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(extract_text(&msg), Some("look at this".to_string()));
    }

    #[test]
    fn test_has_image() {
        let text_msg = Message {
            conversation: Some("hi".to_string()),
            ..Default::default()
        };
        assert!(!has_image(&text_msg));

        let img_msg = Message {
            image_message: Some(Box::new(Default::default())),
            ..Default::default()
        };
        assert!(has_image(&img_msg));
    }
}
