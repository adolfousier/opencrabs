//! WhatsApp Message Handler
//!
//! Processes incoming WhatsApp messages: text + images, allowlist enforcement,
//! session routing (owner shares TUI session, others get per-phone sessions).

use crate::brain::agent::AgentService;
use crate::brain::agent::{ApprovalCallback, ProgressCallback, ProgressEvent};
use crate::channels::whatsapp::WhatsAppState;
use crate::config::Config;
use crate::db::ChannelMessageRepository;
use crate::db::models::ChannelMessage as DbChannelMessage;
use crate::services::SessionService;
use crate::utils::sanitize::redact_secrets;
use crate::utils::truncate_str;
use std::collections::HashSet;
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

/// Extract quoted/replied-to message text from a WhatsApp message.
fn extract_reply_context(msg: &Message) -> Option<String> {
    let msg = unwrap_message(msg);
    let ctx = msg.extended_text_message.as_ref()?.context_info.as_ref()?;
    let quoted = ctx.quoted_message.as_ref()?;
    let quoted_text = extract_text(quoted)?;
    if quoted_text.is_empty() {
        return None;
    }
    let sender = ctx
        .participant
        .as_ref()
        .map(|p| p.split('@').next().unwrap_or(p).to_string())
        .unwrap_or_else(|| "unknown".to_string());
    Some(format!("[Replying to {sender}: \"{quoted_text}\"]"))
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

/// Check if the message has a document attachment.
fn has_document(msg: &Message) -> bool {
    let msg = unwrap_message(msg);
    msg.document_message.is_some()
}

/// Download a document from WhatsApp. Returns (bytes, mime, filename) on success.
async fn download_document(msg: &Message, client: &Client) -> Option<(Vec<u8>, String, String)> {
    let msg = unwrap_message(msg);
    let doc = msg.document_message.as_ref()?;
    let mime = doc.mimetype.clone().unwrap_or_default();
    let fname = doc.file_name.clone().unwrap_or_else(|| "file".to_string());
    match client.download(doc.as_ref()).await {
        Ok(bytes) => {
            tracing::debug!(
                "WhatsApp: downloaded document {} ({} bytes)",
                fname,
                bytes.len()
            );
            Some((bytes, mime, fname))
        }
        Err(e) => {
            tracing::error!("WhatsApp: failed to download document: {e}");
            None
        }
    }
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
/// Extract sender phone from MessageInfo.
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

/// Extract recipient phone from MessageInfo (who the message is TO).
fn recipient_phone(info: &MessageInfo) -> Option<String> {
    info.source.recipient.as_ref().map(|r| {
        let full = r.to_string();
        let without_server = full.split('@').next().unwrap_or(&full);
        without_server
            .split(':')
            .next()
            .unwrap_or(without_server)
            .to_string()
    })
}

/// Split a message into chunks that fit WhatsApp's limit (~65536 chars, but we use 4000 for readability).
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_message(
    msg: Message,
    info: MessageInfo,
    client: Arc<Client>,
    agent: Arc<AgentService>,
    session_svc: SessionService,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    wa_state: Arc<WhatsAppState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    channel_msg_repo: ChannelMessageRepository,
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

    // Build message content: text, image, audio, or document
    let has_img = has_image(&msg);
    let has_aud = has_audio(&msg);
    let has_doc = has_document(&msg);
    let text = extract_text(&msg);

    // Require at least text, image, audio, or document
    if text.is_none() && !has_img && !has_aud && !has_doc {
        return;
    }

    // Passively capture message for channel history (groups and DMs)
    if let Some(ref t) = text
        && !t.is_empty()
    {
        let chat_id = format!("{}", info.source.chat);
        let is_group = info.source.is_group;
        let push_name = info.push_name.clone();
        let cm = DbChannelMessage::new(
            "whatsapp".into(),
            chat_id,
            if is_group {
                Some(format!("{}", info.source.chat))
            } else {
                None
            },
            phone.clone(),
            push_name,
            t.clone(),
            "text".into(),
            None,
        );
        if let Err(e) = channel_msg_repo.insert(&cm).await {
            tracing::warn!("Failed to store WhatsApp channel message: {e}");
        }
    }

    // Read latest config from watch channel — single source of truth
    let cfg = config_rx.borrow().clone();
    let wa_cfg = &cfg.channels.whatsapp;
    let allowed: HashSet<String> = wa_cfg.allowed_phones.iter().cloned().collect();
    let idle_timeout_hours = wa_cfg.session_idle_hours;
    let voice_config = cfg.voice_config();

    // SECURITY: When allowed_phones is configured, only respond to the owner.
    // Also check the recipient: when owner sends a message TO a contact,
    // sender=owner but recipient=contact — must not treat that as "owner messaging bot".
    // If allowed_phones is empty (unconfigured), fall through without filtering.
    if !allowed.is_empty() {
        let owner_phone_raw = allowed.iter().next().cloned().unwrap_or_default();
        let owner_phone = owner_phone_raw.trim_start_matches('+');
        let sender_normalized = phone.trim_start_matches('+');
        let recipient = recipient_phone(&info);
        let recipient_normalized = recipient.as_ref().map(|r| r.trim_start_matches('+'));
        let is_to_owner = recipient_normalized
            .map(|r| r == owner_phone)
            .unwrap_or(false);
        let is_from_owner = sender_normalized == owner_phone;
        if !is_from_owner || (recipient.is_some() && !is_to_owner) {
            tracing::debug!(
                "WhatsApp: ignoring message from={} to={:?} (owner={})",
                phone,
                recipient,
                owner_phone
            );
            return;
        }
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
                "wa_approve_yolo" => Some(WaApproval::Yolo),
                "wa_approve_no" => Some(WaApproval::No),
                _ => None,
            }
        } else if let Some(raw_text) = extract_text(&msg) {
            let answer = raw_text.trim().to_lowercase();
            if matches!(answer.as_str(), "yes" | "y" | "sim" | "s") {
                Some(WaApproval::Yes)
            } else if matches!(answer.as_str(), "always" | "sempre") {
                Some(WaApproval::Always)
            } else if matches!(answer.as_str(), "yolo") {
                Some(WaApproval::Yolo)
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
                crate::utils::persist_auto_session_policy();
            } else if c == WaApproval::Yolo {
                crate::utils::persist_auto_always_policy();
            }
            return;
        }
    }

    let text_preview = text
        .as_deref()
        .map(|t| truncate_str(t, 50))
        .unwrap_or("[image]");
    tracing::info!("WhatsApp: message from {}: {}", phone, text_preview);

    // Audio/voice note → show typing immediately and transcribe
    if has_aud && voice_config.stt_enabled {
        let _ = client.chatstate().send_composing(&info.source.chat).await;
    }
    let mut content;
    if has_aud
        && voice_config.stt_enabled
        && let Some(audio_bytes) = download_audio(&msg, &client).await
    {
        match crate::channels::voice::transcribe(audio_bytes, &voice_config).await {
            Ok(transcript) => {
                tracing::info!(
                    "WhatsApp: transcribed voice: {}",
                    truncate_str(&transcript, 80)
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

    // Handle document attachment
    if has_doc
        && !has_aud
        && !has_img
        && let Some((bytes, mime, fname)) = download_document(&msg, &client).await
    {
        use crate::utils::{FileContent, classify_file};
        match classify_file(&bytes, &mime, &fname) {
            FileContent::Image => {
                let ext = fname.rsplit('.').next().unwrap_or("jpg");
                let tmp =
                    std::env::temp_dir().join(format!("wa_doc_{}.{}", uuid::Uuid::new_v4(), ext));
                if std::fs::write(&tmp, &bytes).is_ok() {
                    if content.is_empty() {
                        content = "Describe this image.".to_string();
                    }
                    content.push_str(&format!(" <<IMG:{}>>", tmp.display()));
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
                if content.is_empty() {
                    content = note;
                } else {
                    content.push_str(&format!("\n\n{note}"));
                }
            }
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
                drop(shared);
                // Resume most recent session from DB (survives daemon restarts)
                let restored = match session_svc.get_most_recent_session().await {
                    Ok(Some(s)) => {
                        tracing::info!("WhatsApp: restored most recent session {}", s.id);
                        Some(s.id)
                    }
                    _ => None,
                };
                let id = match restored {
                    Some(id) => id,
                    None => {
                        tracing::info!("WhatsApp: no existing session, creating one for owner");
                        match session_svc.create_session(Some("Chat".to_string())).await {
                            Ok(session) => session.id,
                            Err(e) => {
                                tracing::error!("WhatsApp: failed to create session: {}", e);
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
        // Non-owner sessions: persisted in DB by title — survives restarts.
        let session_title = format!("WhatsApp: {}", phone);

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
                if let Err(e) = session_svc.archive_session(session.id).await {
                    tracing::error!("WhatsApp: failed to archive session {}: {}", session.id, e);
                }
                match session_svc.create_session(Some(session_title)).await {
                    Ok(new_session) => new_session.id,
                    Err(e) => {
                        tracing::error!("WhatsApp: failed to create session: {}", e);
                        return;
                    }
                }
            } else {
                session.id
            }
        } else {
            match session_svc.create_session(Some(session_title)).await {
                Ok(session) => {
                    tracing::info!("WhatsApp: created new session {} for {}", session.id, phone);
                    session.id
                }
                Err(e) => {
                    tracing::error!("WhatsApp: failed to create session: {}", e);
                    return;
                }
            }
        }
    };

    // Restore session's own provider (each session keeps its provider independently)
    let session_meta = session_svc.get_session(session_id).await.ok().flatten();
    crate::channels::commands::sync_provider_for_session(
        &agent,
        session_meta.as_ref().and_then(|s| s.provider_name.as_deref()),
        session_meta.as_ref().and_then(|s| s.model.as_deref()),
    );

    // ── Channel commands (/help, /usage, /models, /stop) ────────────────────
    {
        use crate::channels::commands::{self, ChannelCommand};
        let cmd = commands::handle_command(&content, session_id, &agent, &session_svc).await;

        // Handle simple text-response commands (Help, Usage, Evolve, Doctor, etc.)
        if let Some(reply_text) = commands::try_execute_text_command(&cmd).await {
            let reply = waproto::whatsapp::Message {
                conversation: Some(reply_text),
                ..Default::default()
            };
            let _ = client.send_message(info.source.chat.clone(), reply).await;
            return;
        }

        match cmd {
            ChannelCommand::Models(resp) => {
                // WhatsApp has no inline buttons — send plain text list
                let reply = waproto::whatsapp::Message {
                    conversation: Some(resp.text),
                    ..Default::default()
                };
                let _ = client.send_message(info.source.chat.clone(), reply).await;
                return;
            }
            ChannelCommand::NewSession => {
                let session_title = format!("WhatsApp: {}", phone);
                if !is_owner
                    && let Ok(Some(old)) = session_svc.find_session_by_title(&session_title).await
                    && let Err(e) = session_svc.archive_session(old.id).await
                {
                    tracing::error!("WhatsApp: failed to archive old session {}: {}", old.id, e);
                }
                match session_svc.create_session(Some(session_title)).await {
                    Ok(new_session) => {
                        if is_owner {
                            *shared_session.lock().await = Some(new_session.id);
                        }
                        let reply = waproto::whatsapp::Message {
                            conversation: Some("✅ New session started.".to_string()),
                            ..Default::default()
                        };
                        let _ = client.send_message(info.source.chat.clone(), reply).await;
                    }
                    Err(e) => {
                        tracing::error!("WhatsApp: failed to create session: {}", e);
                        let reply = waproto::whatsapp::Message {
                            conversation: Some("Failed to create session.".to_string()),
                            ..Default::default()
                        };
                        let _ = client.send_message(info.source.chat.clone(), reply).await;
                    }
                }
                return;
            }
            ChannelCommand::Sessions(resp) => {
                // WhatsApp has no inline buttons — send plain text list
                let reply = waproto::whatsapp::Message {
                    conversation: Some(resp.text),
                    ..Default::default()
                };
                let _ = client.send_message(info.source.chat.clone(), reply).await;
                return;
            }
            ChannelCommand::Stop => {
                let cancelled = wa_state.cancel_session(session_id).await;
                let text = if cancelled {
                    "Operation cancelled."
                } else {
                    "No operation in progress."
                };
                let reply = waproto::whatsapp::Message {
                    conversation: Some(text.to_string()),
                    ..Default::default()
                };
                let _ = client.send_message(info.source.chat.clone(), reply).await;
                return;
            }
            ChannelCommand::Compact => {
                let status = waproto::whatsapp::Message {
                    conversation: Some("⏳ Compacting context...".to_string()),
                    ..Default::default()
                };
                let _ = client.send_message(info.source.chat.clone(), status).await;
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

    // Extract replied-to message context so the agent knows what the user is referencing.
    let reply_context = extract_reply_context(&msg);

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

    // Prepend reply context if the user is replying to a specific message.
    let agent_input = if let Some(ref ctx) = reply_context {
        format!("{ctx}\n{agent_input}")
    } else {
        agent_input
    };

    // Inject recent group history so the agent has full conversation context.
    let agent_input = if info.source.is_group {
        let chat_id_str = info.source.chat.to_string();
        match channel_msg_repo
            .recent(Some("whatsapp"), &chat_id_str, 30)
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

    // Tell the LLM its text response is automatically delivered to the chat.
    let agent_input = format!(
        "[Channel: WhatsApp — your text response is automatically sent to this chat. \
         There is no whatsapp_send tool. Just reply with text.]\n{agent_input}"
    );

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

    // Progress callback: forward intermediate texts (between tool-call iterations)
    // to WhatsApp in real time. WhatsApp doesn't support message editing, so we
    // send each chunk as a new message. Images (<<IMG:...>>) are stripped here —
    // the main handler delivers them as actual WhatsApp image messages.
    let was_streamed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let progress_cb: ProgressCallback = {
        let client_cb = client.clone();
        let jid_cb = info.source.chat.clone();
        let was_streamed_cb = was_streamed.clone();
        Arc::new(move |_session_id, event| match event {
            ProgressEvent::IntermediateText { text, .. } => {
                let (clean, _) = crate::utils::extract_img_markers(&text);
                let clean = redact_secrets(&clean);
                let clean = crate::utils::slack_fmt::markdown_to_mrkdwn(&clean);
                if !clean.trim().is_empty() {
                    was_streamed_cb.store(true, std::sync::atomic::Ordering::Relaxed);
                    let client = client_cb.clone();
                    let jid = jid_cb.clone();
                    let tagged = format!("{}\n\n{}", MSG_HEADER, clean.trim());
                    tokio::spawn(async move {
                        for chunk in split_message(&tagged, 4000) {
                            let msg = waproto::whatsapp::Message {
                                conversation: Some(chunk.to_string()),
                                ..Default::default()
                            };
                            if let Err(e) = client.send_message(jid.clone(), msg).await {
                                tracing::error!("WhatsApp: intermediate text send failed: {}", e);
                            }
                        }
                    });
                }
            }
            ProgressEvent::SelfHealingAlert { message } => {
                let client = client_cb.clone();
                let jid = jid_cb.clone();
                let alert = format!("{}\n\n🔧 {}", MSG_HEADER, message);
                tokio::spawn(async move {
                    let msg = waproto::whatsapp::Message {
                        conversation: Some(alert),
                        ..Default::default()
                    };
                    if let Err(e) = client.send_message(jid, msg).await {
                        tracing::error!("WhatsApp: self-healing alert send failed: {}", e);
                    }
                });
            }
            _ => {}
        })
    };

    // Build per-call approval callback.
    // If the user previously chose "Always (session)", auto-approve without asking.
    // Otherwise send a 3-button message (Yes / Always / No) and wait up to 5 min.
    let approval_cb: ApprovalCallback = {
        use crate::channels::whatsapp::WaApproval;
        use crate::utils::{check_approval_policy, persist_auto_session_policy};

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
                // Respect config-level approval policy (single source of truth)
                if let Some(result) = check_approval_policy() {
                    return Ok(result);
                }

                // Redact secrets before display
                let safe_input = crate::utils::redact_tool_input(&tool_info.tool_input);
                let input_preview = serde_json::to_string_pretty(&safe_input).unwrap_or_default();
                let body = format!(
                    "🔐 *Tool Approval Required*\n\nTool: `{}`\n```\n{}\n```",
                    tool_info.tool_name,
                    truncate_str(&input_preview, 600),
                );

                // Send plain text approval request (ButtonsMessage is deprecated
                // by WhatsApp and silently never renders — use text only)
                let text_msg = waproto::whatsapp::Message {
                    conversation: Some(format!(
                        "{}\n\n{}\n\nReply *yes*, *always* (session), *yolo* (permanent), or *no* (5 min timeout).",
                        MSG_HEADER, body
                    )),
                    ..Default::default()
                };
                tracing::info!(
                    "WhatsApp approval: sending request for tool '{}' to {}",
                    tool_info.tool_name,
                    phone_key
                );
                if let Err(e) = client.send_message(chat_jid.clone(), text_msg).await {
                    tracing::error!("WhatsApp: failed to send approval request: {}", e);
                    return Ok((false, false));
                }

                let (tx, rx) = tokio::sync::oneshot::channel::<WaApproval>();
                wa_state
                    .register_pending_approval(phone_key.clone(), tx)
                    .await;
                tracing::info!(
                    "WhatsApp approval: registered pending for phone={}, waiting for response",
                    phone_key
                );

                match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                    Ok(Ok(WaApproval::Yes)) => {
                        tracing::info!("WhatsApp approval: user approved (phone={})", phone_key);
                        Ok((true, false))
                    }
                    Ok(Ok(WaApproval::Always)) => {
                        tracing::info!(
                            "WhatsApp approval: user chose Always (phone={})",
                            phone_key
                        );
                        persist_auto_session_policy();
                        Ok((true, true))
                    }
                    Ok(Ok(WaApproval::Yolo)) => {
                        tracing::info!("WhatsApp approval: user chose YOLO (phone={})", phone_key);
                        crate::utils::persist_auto_always_policy();
                        Ok((true, true))
                    }
                    Ok(Ok(WaApproval::No)) => {
                        tracing::info!("WhatsApp approval: user denied (phone={})", phone_key);
                        Ok((false, false))
                    }
                    _ => {
                        tracing::warn!(
                            "WhatsApp: approval timed out or channel dropped — denying (phone={})",
                            phone_key
                        );
                        let timeout_msg = waproto::whatsapp::Message {
                            conversation: Some(format!(
                                "{}\n\n⏰ No response in 5 minutes — *{}* was denied.\n\nSend your message again and reply *yes*, *always*, or *no* when prompted.",
                                MSG_HEADER, tool_info.tool_name,
                            )),
                            ..Default::default()
                        };
                        let _ = client.send_message(chat_jid, timeout_msg).await;
                        Ok((false, false))
                    }
                }
            })
        })
    };

    // Send to agent with WhatsApp approval + progress callbacks
    let cancel_token = CancellationToken::new();
    wa_state
        .store_cancel_token(session_id, cancel_token.clone())
        .await;

    let wa_chat_id = format!("{}", info.source.chat);
    let result = agent
        .send_message_with_tools_and_callback(
            session_id,
            agent_input,
            None,
            Some(cancel_token),
            Some(approval_cb),
            Some(progress_cb),
            "whatsapp",
            Some(&wa_chat_id),
        )
        .await;

    wa_state.remove_cancel_token(session_id).await;
    typing_cancel.cancel();

    match result {
        Ok(response) => {
            let reply_jid = info.source.chat.clone();

            // Extract <<IMG:path>> markers — send each as a real WhatsApp image message.
            let (text_content, img_paths) = crate::utils::extract_img_markers(&response.content);
            let text_content = crate::utils::sanitize::strip_llm_artifacts(&text_content);
            let text_content = redact_secrets(&text_content);
            let text_content = crate::utils::slack_fmt::markdown_to_mrkdwn(&text_content);

            // Send images before text
            for img_path in img_paths {
                match tokio::fs::read(&img_path).await {
                    Ok(bytes) => {
                        use wacore::download::MediaType;
                        use waproto::whatsapp::message::ImageMessage;
                        match client.upload(bytes, MediaType::Image).await {
                            Ok(upload) => {
                                let mime = if img_path.ends_with(".png") {
                                    "image/png"
                                } else {
                                    "image/jpeg"
                                };
                                let img_msg = waproto::whatsapp::Message {
                                    image_message: Some(Box::new(ImageMessage {
                                        url: Some(upload.url),
                                        direct_path: Some(upload.direct_path),
                                        media_key: Some(upload.media_key),
                                        file_enc_sha256: Some(upload.file_enc_sha256),
                                        file_sha256: Some(upload.file_sha256),
                                        file_length: Some(upload.file_length),
                                        mimetype: Some(mime.to_string()),
                                        ..Default::default()
                                    })),
                                    ..Default::default()
                                };
                                if let Err(e) =
                                    client.send_message(reply_jid.clone(), img_msg).await
                                {
                                    tracing::error!(
                                        "WhatsApp: failed to send generated image: {}",
                                        e
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "WhatsApp: image upload failed for {}: {}",
                                    img_path,
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("WhatsApp: failed to read image {}: {}", img_path, e);
                    }
                }
            }

            // Send text response (markers stripped).
            // Skip if already delivered progressively via the intermediate-text callback
            // (happens when the agent used tool calls — text was sent between iterations).
            if !text_content.is_empty() && !was_streamed.load(std::sync::atomic::Ordering::Relaxed)
            {
                let tagged = format!("{}\n\n{}", MSG_HEADER, text_content);
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

            // If input was voice AND TTS is enabled, also send voice note after text
            if has_aud && voice_config.tts_enabled {
                match crate::channels::voice::synthesize(&response.content, &voice_config).await {
                    Ok(audio_bytes) => {
                        // WhatsApp requires uploading media to its servers first,
                        // then sending the message with the returned URL + crypto keys.
                        use wacore::download::MediaType;
                        use waproto::whatsapp::message::AudioMessage;
                        match client.upload(audio_bytes, MediaType::Audio).await {
                            Ok(upload) => {
                                let audio_msg = waproto::whatsapp::Message {
                                    audio_message: Some(Box::new(AudioMessage {
                                        url: Some(upload.url),
                                        direct_path: Some(upload.direct_path),
                                        media_key: Some(upload.media_key),
                                        file_enc_sha256: Some(upload.file_enc_sha256),
                                        file_sha256: Some(upload.file_sha256),
                                        file_length: Some(upload.file_length),
                                        mimetype: Some("audio/ogg; codecs=opus".to_string()),
                                        ptt: Some(true),
                                        ..Default::default()
                                    })),
                                    ..Default::default()
                                };
                                if let Err(e) =
                                    client.send_message(reply_jid.clone(), audio_msg).await
                                {
                                    tracing::error!("WhatsApp: failed to send TTS voice: {}", e);
                                }
                            }
                            Err(e) => {
                                tracing::error!("WhatsApp: TTS audio upload failed: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("WhatsApp: TTS synthesis error: {}", e);
                    }
                }
            }
        }
        Err(ref e) if matches!(e, crate::brain::agent::AgentError::Cancelled) => {
            tracing::info!("WhatsApp: agent call cancelled for session {}", session_id);
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
            image_message: Some(Box::default()),
            ..Default::default()
        };
        assert!(has_image(&img_msg));
    }
}
