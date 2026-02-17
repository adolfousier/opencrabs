//! WhatsApp Message Handler
//!
//! Processes incoming WhatsApp messages: text + images, allowlist enforcement,
//! session routing (owner shares TUI session, others get per-phone sessions).

use crate::config::VoiceConfig;
use crate::llm::agent::AgentService;
use crate::services::SessionService;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use wacore::types::message::MessageInfo;
use waproto::whatsapp::Message;
use whatsapp_rust::client::Client;

/// Header prepended to all outgoing messages so the user knows it's from the agent.
pub const MSG_HEADER: &str = "\u{1f980} *OpenCrabs*";

/// Unwrap nested message wrappers (device_sent, ephemeral, view_once, etc.)
/// Returns the innermost Message that contains actual content.
fn unwrap_message(msg: &Message) -> &Message {
    // device_sent_message: wraps messages synced across linked devices
    if let Some(ref dsm) = msg.device_sent_message {
        if let Some(ref inner) = dsm.message {
            return unwrap_message(inner);
        }
    }
    // ephemeral_message: disappearing messages
    if let Some(ref eph) = msg.ephemeral_message {
        if let Some(ref inner) = eph.message {
            return unwrap_message(inner);
        }
    }
    // view_once_message
    if let Some(ref vo) = msg.view_once_message {
        if let Some(ref inner) = vo.message {
            return unwrap_message(inner);
        }
    }
    // document_with_caption_message
    if let Some(ref dwc) = msg.document_with_caption_message {
        if let Some(ref inner) = dwc.message {
            return unwrap_message(inner);
        }
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

/// Download image from WhatsApp and save to a temp file.
/// Returns the file path on success.
async fn download_image(msg: &Message, client: &Client) -> Option<String> {
    let msg = unwrap_message(msg);
    let img = msg.image_message.as_ref()?;

    let mime = img
        .mimetype
        .as_deref()
        .unwrap_or("image/jpeg");
    let ext = match mime {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    };

    match client.download(img.as_ref()).await {
        Ok(bytes) => {
            let path = std::env::temp_dir().join(format!(
                "wa_img_{}.{}",
                uuid::Uuid::new_v4(),
                ext
            ));
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
/// JID format is "351933536442@s.whatsapp.net" — we return just "351933536442".
fn sender_phone(info: &MessageInfo) -> String {
    let full = info.source.sender.to_string();
    full.split('@').next().unwrap_or(&full).to_string()
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
    extra_sessions: Arc<Mutex<HashMap<String, Uuid>>>,
    _voice_config: Arc<VoiceConfig>,
    shared_session: Arc<Mutex<Option<Uuid>>>,
) {
    let phone = sender_phone(&info);
    tracing::debug!(
        "WhatsApp handler: from={}, is_from_me={}, has_text={}, has_image={}",
        phone,
        info.source.is_from_me,
        extract_text(&msg).is_some(),
        has_image(&msg),
    );

    // Skip bot's own outgoing replies (they echo back as is_from_me).
    // User messages from their phone are also is_from_me (same account),
    // so we only skip if the text starts with our agent header.
    if info.source.is_from_me {
        if let Some(text) = extract_text(&msg) {
            if text.starts_with(MSG_HEADER) {
                return;
            }
        } else {
            // No text and is_from_me — likely a media echo, skip
            return;
        }
    }

    // Build message content: text + optional image attachment
    let has_img = has_image(&msg);
    let text = extract_text(&msg);

    // Require at least text or image
    if text.is_none() && !has_img {
        return;
    }

    // Allowlist check — if allowed list is empty, accept all.
    // Normalize: strip '+' from allowed entries to match JID digits.
    if !allowed.is_empty()
        && !allowed.iter().any(|a| a.trim_start_matches('+') == phone)
    {
        tracing::debug!("WhatsApp: ignoring message from non-allowed phone {}", phone);
        return;
    }

    let text_preview = text
        .as_deref()
        .map(|t| &t[..t.len().min(50)])
        .unwrap_or("[image]");
    tracing::info!("WhatsApp: message from {}: {}", phone, text_preview);

    // Download image if present, append <<IMG:path>> marker
    let mut content = text.unwrap_or_default();
    if has_img {
        if let Some(img_path) = download_image(&msg, &client).await {
            if content.is_empty() {
                content = "Describe this image.".to_string();
            }
            content.push_str(&format!(" <<IMG:{}>>", img_path));
        }
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
        match map.get(&phone) {
            Some(id) => *id,
            None => {
                let title = format!("WhatsApp: {}", phone);
                match session_svc.create_session(Some(title)).await {
                    Ok(session) => {
                        map.insert(phone.clone(), session.id);
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("WhatsApp: failed to create session: {}", e);
                        return;
                    }
                }
            }
        }
    };

    // Send to agent
    match agent.send_message_with_tools(session_id, content, None).await {
        Ok(response) => {
            let reply_jid = info.source.sender.clone();
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
                .send_message(info.source.sender.clone(), error_msg)
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
