//! WhatsApp Send Tool
//!
//! Agent-callable tool for full WhatsApp control: send, reply, delete,
//! send photos/documents/audio/video/stickers, locations, contacts,
//! reactions, polls, typing indicators, and read receipts.
//! Uses the shared `WhatsAppState` to access the connected client.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolHints, ToolResult};
use crate::channels::whatsapp::WhatsAppState;
use crate::channels::whatsapp::broadcast;
use crate::channels::whatsapp::rate_limit::GateOutcome;
use crate::config::Config;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use wacore_binary::jid::Jid;

/// Tool for comprehensive WhatsApp control (14 actions).
pub struct WhatsAppSendTool {
    whatsapp_state: Arc<WhatsAppState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
}

impl WhatsAppSendTool {
    pub fn new(
        whatsapp_state: Arc<WhatsAppState>,
        config_rx: tokio::sync::watch::Receiver<Config>,
    ) -> Self {
        Self {
            whatsapp_state,
            config_rx,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a required non-empty string param, returning ToolResult::error on failure.
#[allow(clippy::result_large_err)]
pub(crate) fn get_str<'a>(input: &'a Value, key: &str) -> std::result::Result<&'a str, ToolResult> {
    match input.get(key).and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => Ok(s),
        _ => Err(ToolResult::error(format!(
            "Missing required parameter '{key}'."
        ))),
    }
}

/// Extract an optional f64 param.
pub(crate) fn get_f64(input: &Value, key: &str) -> Option<f64> {
    input.get(key).and_then(|v| v.as_f64())
}

/// Macro to early-return Ok(err_result) when a param helper returns Err.
macro_rules! pget {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => return Ok(e),
        }
    };
}

/// Pace one non-text send through the shared #1407 budget.
///
/// #1407 landed covering `send` and `reply` only, so every media send, poll
/// and reaction went straight to the wire and the budget under-counted by
/// however many of those a turn made. They cannot use `gate`: that parks an
/// over-cap send in a queue holding `{ jid, text }`, and the drainer replays
/// it as plain text, which would deliver a photo's caption as a bare message
/// and drop the image. So these pace on the bucket and, under a saturated
/// daily cap, refuse and tell the agent rather than pretending to have sent.
///
/// Gates the actions that DELIVER A MESSAGE. Receipts, presence and account
/// operations (typing, mark_read, delete, pin, block, profile) are not
/// messages and are not charged to a message budget.
macro_rules! gate_send {
    ($self:expr, $jid_str:expr, $what:expr) => {{
        let (cfg, owner) = {
            let wa = &$self.config_rx.borrow().channels.whatsapp;
            (
                wa.rate_limit.clone(),
                wa.is_owner($jid_str.split('@').next().unwrap_or(&$jid_str)),
            )
        };
        if !$self
            .whatsapp_state
            .rate_limiter
            .gate_action(&cfg, owner)
            .await
        {
            return Ok(ToolResult::error(format!(
                "Daily WhatsApp send cap reached; {} was NOT sent. Unlike text, this is \
                 not queued for retry - try again once the 24h window slides.",
                $what
            )));
        }
    }};
}

/// Resolve the WhatsApp client, returning an error if not connected.
#[allow(clippy::result_large_err)]
async fn get_client(
    whatsapp_state: &WhatsAppState,
) -> std::result::Result<Arc<whatsapp_rust::client::Client>, ToolResult> {
    whatsapp_state.client().await.ok_or_else(|| {
        ToolResult::error(
            "WhatsApp is not connected. Ask the user to connect WhatsApp first \
                 (use the whatsapp_connect tool)."
                .to_string(),
        )
    })
}

/// Resolve target JID from `phone` param or owner, with allowlist check.
#[allow(clippy::result_large_err)]
async fn resolve_jid(
    input: &Value,
    whatsapp_state: &WhatsAppState,
    config_rx: &tokio::sync::watch::Receiver<Config>,
) -> std::result::Result<(Jid, String), ToolResult> {
    if let Some(phone) = input.get("phone").and_then(|v| v.as_str()) {
        let allowed = &config_rx.borrow().channels.whatsapp.allowed_phones;
        let normalized = phone.trim_start_matches('+');
        let phone_allowed = allowed.is_empty()
            || allowed
                .iter()
                .any(|p| p.trim_start_matches('+') == normalized);
        if !phone_allowed {
            return Err(ToolResult::error(format!(
                "Sending to {} is not permitted. This number is not in the \
                 allowed_users config.",
                phone
            )));
        }
        let digits = phone.trim_start_matches('+');
        let jid_str = format!("{}@s.whatsapp.net", digits);
        let jid: Jid = jid_str
            .parse()
            .map_err(|e| ToolResult::error(format!("Invalid phone number format: {}", e)))?;
        if !crate::cron::send_scope::may_send("whatsapp", &jid_str) {
            let reason = crate::cron::send_scope::refusal_for("whatsapp", &jid_str);
            tracing::warn!("whatsapp_send: {reason}");
            return Err(ToolResult::error(reason));
        }
        Ok((jid, jid_str))
    } else {
        let jid_str = whatsapp_state.owner_jid().await.ok_or_else(|| {
            ToolResult::error(
                "No owner phone number configured and no 'phone' parameter provided. \
                 Specify a phone number to send to."
                    .to_string(),
            )
        })?;
        let jid: Jid = jid_str
            .parse()
            .map_err(|e| ToolResult::error(format!("Invalid owner JID: {}", e)))?;
        if !crate::cron::send_scope::may_send("whatsapp", &jid_str) {
            let reason = crate::cron::send_scope::refusal_for("whatsapp", &jid_str);
            tracing::warn!("whatsapp_send: {reason}");
            return Err(ToolResult::error(reason));
        }
        Ok((jid, jid_str))
    }
}

/// Prefix outgoing text with the channel attribution header. Shared by the
/// send and reply paths so persisted history carries ONE consistent format
/// for every agent-authored message (#1490-E: reply persisted bare text
/// while send persisted the tagged form).
pub(crate) fn tag_with_header(message: &str) -> String {
    format!(
        "{}\n\n{}",
        crate::channels::whatsapp::handler::MSG_HEADER,
        message
    )
}

/// Concatenate the chunks that actually left. `split_message` returns
/// contiguous slices of its input, so the concat is the exact delivered
/// prefix of the tagged text (#1490-B).
pub(crate) fn delivered_prefix(delivered: &[&str]) -> String {
    delivered.concat()
}

/// Error text for a chunked send that died partway: names how many chunks
/// were delivered (and persisted) vs which one failed, and tells the model
/// what a retry may and must-not resend (#1490-B; blind retries used to
/// duplicate chunk 1).
pub(crate) fn partial_failure_report(total: usize, delivered: usize, err: &str) -> String {
    if delivered == 0 {
        format!(
            "Failed to send WhatsApp message: {err}. No chunks were delivered; safe to retry the whole message."
        )
    } else {
        format!(
            "Failed to send WhatsApp message: chunk {} of {} failed: {err}. Chunks 1-{} were DELIVERED and persisted; do NOT resend them. Retry only the remaining text if needed.",
            delivered + 1,
            total,
            delivered
        )
    }
}

/// Persist outgoing messages to `channel_messages` for reply-recovery.
pub(crate) async fn persist_outgoing(jid: &Jid, content: &str) {
    if content.trim().is_empty() {
        return;
    }
    let Some(pool) = crate::db::global_pool() else {
        return;
    };
    let repo = crate::db::ChannelMessageRepository::new(pool.clone());
    let chat_id = jid.to_string();
    let cm = crate::db::models::ChannelMessage::new(
        "whatsapp".to_string(),
        chat_id,
        None,
        "bot:opencrabs".to_string(),
        "OpenCrabs".to_string(),
        content.to_string(),
        "text".to_string(),
        None,
    );
    if let Err(e) = repo.insert(&cm).await {
        tracing::warn!("whatsapp_send: failed to persist outgoing message: {}", e);
    }
}

/// Read a local file, expanding tilde. Returns (bytes, detected mime, filename).
#[allow(clippy::result_large_err)]
async fn read_local_media(
    path: &str,
    default_mime: &str,
) -> std::result::Result<(Vec<u8>, String, String), ToolResult> {
    let expanded = crate::brain::tools::error::expand_tilde(path);
    let bytes = tokio::fs::read(&expanded).await.map_err(|e| {
        ToolResult::error(format!(
            "Failed to read file '{}': {}",
            expanded.display(),
            e
        ))
    })?;
    let filename = expanded
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let mime = mime_from_extension(&expanded.to_string_lossy())
        .unwrap_or_else(|| default_mime.to_string());
    Ok((bytes, mime, filename))
}

/// Detect MIME type from file extension.
pub(crate) fn mime_from_extension(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?.to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "png" => Some("image/png".to_string()),
        "gif" => Some("image/gif".to_string()),
        "webp" => Some("image/webp".to_string()),
        "mp4" => Some("video/mp4".to_string()),
        "3gp" => Some("video/3gpp".to_string()),
        "avi" => Some("video/x-msvideo".to_string()),
        "mov" => Some("video/quicktime".to_string()),
        "mp3" => Some("audio/mpeg".to_string()),
        "ogg" | "opus" => Some("audio/ogg".to_string()),
        "aac" => Some("audio/aac".to_string()),
        "m4a" => Some("audio/mp4".to_string()),
        "pdf" => Some("application/pdf".to_string()),
        "doc" | "docx" => Some("application/msword".to_string()),
        "xls" | "xlsx" => Some("application/vnd.ms-excel".to_string()),
        "ppt" | "pptx" => Some("application/vnd.ms-powerpoint".to_string()),
        "txt" => Some("text/plain".to_string()),
        "csv" => Some("text/csv".to_string()),
        "zip" => Some("application/zip".to_string()),
        _ => None,
    }
}

/// Upload media to WhatsApp servers and return the upload response.
/// Uses the same pattern as the WhatsApp handler.
#[allow(clippy::result_large_err)]
async fn upload_media(
    client: &whatsapp_rust::client::Client,
    data: Vec<u8>,
    media_type: wacore::download::MediaType,
) -> std::result::Result<whatsapp_rust::upload::UploadResponse, ToolResult> {
    client
        .upload(
            data,
            media_type,
            whatsapp_rust::upload::UploadOptions::new(),
        )
        .await
        .map_err(|e| ToolResult::error(format!("Media upload failed: {}", e)))
}

/// Build a vCard string from name and phone number.
pub(crate) fn build_vcard(name: &str, phone: &str) -> String {
    format!(
        "BEGIN:VCARD\nVERSION:3.0\nFN:{}\nTEL;TYPE=CELL:{}\nEND:VCARD",
        name, phone
    )
}

// ---------------------------------------------------------------------------
// Tool implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Tool for WhatsAppSendTool {
    fn name(&self) -> &str {
        "whatsapp_send"
    }

    fn description(&self) -> &str {
        "Full WhatsApp control: send messages, reply, delete, send photos/documents/audio/video/stickers, \
         locations, contacts, emoji reactions, polls, typing indicators, and mark messages as read. \
         If no phone number is specified, messages go to the owner (primary user). \
         Requires WhatsApp to be connected first."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "send", "reply", "delete",
                        "pin", "unpin", "forward", "set_profile_name", "set_profile_status",
                        "send_photo", "send_document", "send_audio", "send_video", "send_sticker",
                        "send_location", "send_contact",
                        "react", "send_poll",
                        "typing", "mark_read",
                        "block_contact", "unblock_contact", "list_blocked",
                        "status_update", "list_newsletters",
                        "create_label", "assign_label"
                    ],
                    "description": "The WhatsApp action to perform."
                },
                "message": {
                    "type": "string",
                    "description": "Message text (send, reply, caption for media); the new name or status text for set_profile_name / set_profile_status"
                },
                "phone": {
                    "type": "string",
                    "description": "Phone number in E.164 format (e.g. '+15551234567'). Omit to message the owner."
                },
                "message_id": {
                    "type": "string",
                    "description": "WhatsApp message ID for reply, delete, react, mark_read, forward"
                },
                "from_me": {
                    "type": "boolean",
                    "description": "Whether the target message was sent by us (for react/mark_read). Default false."
                },
                "media_path": {
                    "type": "string",
                    "description": "Local file path for media (send_photo, send_document, send_audio, send_video, send_sticker)"
                },
                "multi_select": {
                    "type": "boolean",
                    "description": "For send_poll: true lets a voter pick several options. Default false (single choice)."
                },
                "voice_note": {
                    "type": "boolean",
                    "description": "For send_audio: true (default) renders a native WhatsApp voice note (tap-to-play bubble) and shows a recording indicator while uploading. false sends a plain audio file attachment."
                },
                "targets": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "For status_update: phone numbers to post the status to. Every one must appear in channels.whatsapp.broadcast.allowed_targets or NOTHING is sent. Omit to use the whole allowlist."
                },
                "label_id": {
                    "type": "string",
                    "description": "For create_label (omit to make a new label; supplying an existing id renames/recolors it) and assign_label (required)"
                },
                "label_color": {
                    "type": "integer",
                    "description": "For create_label: WhatsApp colour index. Default 0."
                },
                "remove": {
                    "type": "boolean",
                    "description": "For assign_label: true removes the label from the chat instead of adding it."
                },
                "caption": {
                    "type": "string",
                    "description": "Caption for media messages (send_photo, send_video, send_document)"
                },
                "latitude": {
                    "type": "number",
                    "description": "Latitude for send_location"
                },
                "longitude": {
                    "type": "number",
                    "description": "Longitude for send_location"
                },
                "location_name": {
                    "type": "string",
                    "description": "Optional location name for send_location"
                },
                "location_address": {
                    "type": "string",
                    "description": "Optional location address for send_location"
                },
                "contact_name": {
                    "type": "string",
                    "description": "Contact display name for send_contact"
                },
                "contact_phone": {
                    "type": "string",
                    "description": "Contact phone number for send_contact"
                },
                "emoji": {
                    "type": "string",
                    "description": "Emoji for react action (e.g. '👍', '❤️'). Empty string to remove reaction."
                },
                "poll_question": {
                    "type": "string",
                    "description": "Poll question text for send_poll"
                },
                "poll_options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Array of poll option strings (2-12) for send_poll"
                },
                "typing_active": {
                    "type": "boolean",
                    "description": "For typing action: true = composing, false = paused. Default true."
                },
                "ephemeral": {
                    "type": "integer",
                    "description": "For send: disappearing-message TTL in seconds (86400 = 24h, 604800 = 7d, 7776000 = 90d max). 0 means the message does not expire, overriding the channel default. Omit to use the channel default."
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: true,
        }
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) if !a.is_empty() => a.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing required 'action' parameter.".to_string(),
                ));
            }
        };

        let client = pget!(get_client(&self.whatsapp_state).await);

        match action.as_str() {
            // ── send ─────────────────────────────────────────────────────────
            "send" => {
                let message = match input.get("message").and_then(|v| v.as_str()) {
                    Some(m) if !m.is_empty() => m.to_string(),
                    _ => {
                        return Ok(ToolResult::error(
                            "Missing or empty 'message' parameter.".to_string(),
                        ));
                    }
                };
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);

                // Convert markdown to WhatsApp format and prepend agent header
                let message = crate::utils::slack_fmt::markdown_to_mrkdwn(&message);
                let tagged = tag_with_header(&message);
                let chunks = crate::channels::whatsapp::handler::split_message(&tagged, 4000);
                let total = chunks.len();
                let mut delivered: Vec<&str> = Vec::with_capacity(total);
                // #1407: pace sends through the shared limiter; over-cap
                // chunks park FIFO for the drainer (queued text must NOT
                // be resent by the model). Owner-bound sends bypass.
                let (rl_cfg, rl_owner) = {
                    let wa = &self.config_rx.borrow().channels.whatsapp;
                    (
                        wa.rate_limit.clone(),
                        wa.is_owner(jid_str.split('@').next().unwrap_or(&jid_str)),
                    )
                };
                // #1487: disappearing messages. An explicit `ephemeral: 0`
                // means "do not expire" and overrides the channel default,
                // so this is not a plain `or`.
                let ttl = crate::channels::whatsapp::ephemeral::resolve(
                    input.get("ephemeral").and_then(|v| v.as_u64()),
                    self.config_rx.borrow().channels.whatsapp.ephemeral_ttl,
                );
                let send_opts = whatsapp_rust::send::SendOptions {
                    ephemeral_expiration: ttl,
                    ..Default::default()
                };
                let mut queued_any = false;
                for chunk in chunks {
                    if let GateOutcome::Queued { .. } = self
                        .whatsapp_state
                        .rate_limiter
                        .gate(&rl_cfg, &jid_str, chunk, rl_owner)
                        .await
                    {
                        queued_any = true;
                        continue;
                    }
                    let wa_msg = waproto::whatsapp::Message {
                        conversation: Some(chunk.to_string()),
                        ..Default::default()
                    };
                    if let Err(e) = client
                        .send_message_with_options(jid.clone(), wa_msg, send_opts.clone())
                        .await
                    {
                        // #1490-B: account for what already left the building.
                        // Delivered chunks are persisted so history matches
                        // reality, and the error names exactly what arrived vs
                        // what failed so a retry cannot duplicate chunk 1.
                        let prefix = delivered_prefix(&delivered);
                        if !prefix.trim().is_empty() {
                            persist_outgoing(&jid, &prefix).await;
                        }
                        return Ok(ToolResult::error(partial_failure_report(
                            total,
                            delivered.len(),
                            &e.to_string(),
                        )));
                    }
                    delivered.push(chunk);
                }

                if queued_any {
                    // Queued chunks flush via the drainer (which persists
                    // them); record only what actually left now.
                    if !delivered.is_empty() {
                        let prefix = delivered_prefix(&delivered);
                        if !prefix.trim().is_empty() {
                            persist_outgoing(&jid, &prefix).await;
                        }
                    }
                    return Ok(ToolResult::success(format!(
                        "WhatsApp daily cap reached: {} of {} chunk(s) queued for automatic delivery as the 24h window slides (owner alerted once); {} delivered now. Do NOT resend the queued part.",
                        total - delivered.len(),
                        total,
                        delivered.len()
                    )));
                }
                persist_outgoing(&jid, &tagged).await;
                Ok(ToolResult::success(match ttl {
                    Some(seconds) => format!(
                        "Message sent to {} via WhatsApp; it disappears after {}.",
                        jid_str,
                        crate::channels::whatsapp::ephemeral::describe(seconds)
                    ),
                    None => format!("Message sent to {} via WhatsApp.", jid_str),
                }))
            }

            // ── reply ────────────────────────────────────────────────────────
            "reply" => {
                let message = pget!(get_str(&input, "message")).to_string();
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                let msg_id = pget!(get_str(&input, "message_id")).to_string();

                let formatted = crate::utils::slack_fmt::markdown_to_mrkdwn(&message);
                // #1490-E: the reply carries and persists the same tagged form
                // as the send path - one event class, one format.
                let tagged = tag_with_header(&formatted);
                // #1490-C: chunk the reply like send does. WhatsApp rejects
                // or truncates over-long message text, so a long reply must
                // not ride a single ExtendedTextMessage. Chunk 0 keeps the
                // quote context; later chunks go as plain conversation
                // messages (one quoted reply plus N-1 normal follow-ups -
                // quoting the same stanza N times is noise).
                let chunks = crate::channels::whatsapp::handler::split_message(&tagged, 4000);
                let total = chunks.len();
                let mut delivered: Vec<&str> = Vec::with_capacity(total);
                // #1407: same limiter gating as the send arm. Queued reply
                // chunks lose their quote context on drainer flush
                // (documented tradeoff); ordering is preserved FIFO.
                let (rl_cfg, rl_owner) = {
                    let wa = &self.config_rx.borrow().channels.whatsapp;
                    (
                        wa.rate_limit.clone(),
                        wa.is_owner(jid_str.split('@').next().unwrap_or(&jid_str)),
                    )
                };
                let mut queued_any = false;
                for (i, chunk) in chunks.into_iter().enumerate() {
                    if let GateOutcome::Queued { .. } = self
                        .whatsapp_state
                        .rate_limiter
                        .gate(&rl_cfg, &jid_str, chunk, rl_owner)
                        .await
                    {
                        queued_any = true;
                        continue;
                    }
                    let wa_msg = if i == 0 {
                        waproto::whatsapp::Message {
                            extended_text_message: Some(Box::new(
                                waproto::whatsapp::message::ExtendedTextMessage {
                                    text: Some(chunk.to_string()),
                                    context_info: Some(Box::new(waproto::whatsapp::ContextInfo {
                                        stanza_id: Some(msg_id.clone()),
                                        remote_jid: Some(jid_str.clone()),
                                        ..Default::default()
                                    })),
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }
                    } else {
                        waproto::whatsapp::Message {
                            conversation: Some(chunk.to_string()),
                            ..Default::default()
                        }
                    };
                    if let Err(e) = client.send_message(jid.clone(), wa_msg).await {
                        // Same accounting as the send path (#1490-B).
                        let prefix = delivered_prefix(&delivered);
                        if !prefix.trim().is_empty() {
                            persist_outgoing(&jid, &prefix).await;
                        }
                        return Ok(ToolResult::error(partial_failure_report(
                            total,
                            delivered.len(),
                            &e.to_string(),
                        )));
                    }
                    delivered.push(chunk);
                }

                if queued_any {
                    if !delivered.is_empty() {
                        let prefix = delivered_prefix(&delivered);
                        if !prefix.trim().is_empty() {
                            persist_outgoing(&jid, &prefix).await;
                        }
                    }
                    return Ok(ToolResult::success(format!(
                        "WhatsApp daily cap reached: {} of {} reply chunk(s) queued for automatic delivery as the 24h window slides (owner alerted once); {} delivered now. Do NOT resend the queued part.",
                        total - delivered.len(),
                        total,
                        delivered.len()
                    )));
                }
                persist_outgoing(&jid, &tagged).await;
                Ok(ToolResult::success(format!(
                    "Reply sent to {} via WhatsApp.",
                    jid_str
                )))
            }

            // ── delete ───────────────────────────────────────────────────────
            "delete" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                let msg_id = pget!(get_str(&input, "message_id")).to_string();

                match client
                    .revoke_message(jid, msg_id.clone(), Default::default())
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message {} deleted for {}.",
                        msg_id, jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to delete message: {}",
                        e
                    ))),
                }
            }

            // ── pin / unpin ──────────────────────────────────────────────────
            // #1484. The lib pins a CHAT, not a message (`pin_chat` /
            // `unpin_chat` in features/chat_actions.rs), so these take a
            // phone and no message id - the issue's "chat_id + message_id"
            // shape does not exist in the protocol.
            "pin" | "unpin" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                let pinning = action == "pin";
                let result = if pinning {
                    client.chat_actions().pin_chat(&jid).await
                } else {
                    client.chat_actions().unpin_chat(&jid).await
                };
                match result {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Chat with {} {}.",
                        jid_str,
                        if pinning { "pinned" } else { "unpinned" }
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to {} chat {jid_str}: {e}",
                        if pinning { "pin" } else { "unpin" }
                    ))),
                }
            }

            // ── forward ──────────────────────────────────────────────────────
            // #1484. `Client::forward_message` needs the ORIGINAL proto, not an
            // id: it rebuilds the body with the forward flags and relays media
            // from the same CDN blob. The channel remembers the last 200
            // inbound messages for exactly this (see `recent.rs`); anything
            // older is honestly reported as out of the window rather than
            // silently sending nothing.
            "forward" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the forwarded message");
                let msg_id = pget!(get_str(&input, "message_id")).to_string();
                let Some(original) = self.whatsapp_state.recent.get(&msg_id).await else {
                    return Ok(ToolResult::error(format!(
                        "Message {msg_id} is not in the recent-message window, so it cannot be \
                         forwarded. Only messages this session has seen can be forwarded."
                    )));
                };
                match client.forward_message(jid, &original).await {
                    Ok(result) => Ok(ToolResult::success(format!(
                        "Forwarded message {} to {} via WhatsApp (new id {}).",
                        msg_id, jid_str, result.message_id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to forward {msg_id} to {jid_str}: {e}"
                    ))),
                }
            }

            // ── set_profile_name / set_profile_status ────────────────────────
            // #1484 asked for a separate `whatsapp_profile` tool. Kept as
            // actions here instead: the channel already routes every WhatsApp
            // operation through one tool, and a second one would split that
            // surface for two setters.
            "set_profile_name" => {
                let name = pget!(get_str(&input, "message")).to_string();
                match client.profile().set_push_name(&name).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "WhatsApp profile name set to '{name}'."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to set the profile name: {e}"
                    ))),
                }
            }

            "set_profile_status" => {
                let status = pget!(get_str(&input, "message")).to_string();
                match client.profile().set_status_text(&status).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "WhatsApp profile status set to '{status}'."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to set the profile status: {e}"
                    ))),
                }
            }

            // ── send_photo ───────────────────────────────────────────────────
            "send_photo" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the photo");
                let path = pget!(get_str(&input, "media_path")).to_string();
                let caption = input
                    .get("caption")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let (bytes, mime, _filename) = pget!(read_local_media(&path, "image/jpeg").await);
                let upload =
                    pget!(upload_media(&client, bytes, wacore::download::MediaType::Image).await);

                let wa_msg = waproto::whatsapp::Message {
                    image_message: Some(Box::new(waproto::whatsapp::message::ImageMessage {
                        url: Some(upload.url),
                        direct_path: Some(upload.direct_path),
                        media_key: Some(upload.media_key.to_vec()),
                        file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                        file_sha256: Some(upload.file_sha256.to_vec()),
                        file_length: Some(upload.file_length),
                        mimetype: Some(mime),
                        caption,
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                match client.send_message(jid, wa_msg).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Photo sent to {} via WhatsApp.",
                        jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send photo: {}", e))),
                }
            }

            // ── send_document ────────────────────────────────────────────────
            "send_document" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the document");
                let path = pget!(get_str(&input, "media_path")).to_string();
                let caption = input
                    .get("caption")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let (bytes, mime, filename) =
                    pget!(read_local_media(&path, "application/octet-stream").await);
                let upload = pget!(
                    upload_media(&client, bytes, wacore::download::MediaType::Document).await
                );

                let wa_msg = waproto::whatsapp::Message {
                    document_message: Some(Box::new(waproto::whatsapp::message::DocumentMessage {
                        url: Some(upload.url),
                        direct_path: Some(upload.direct_path),
                        media_key: Some(upload.media_key.to_vec()),
                        file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                        file_sha256: Some(upload.file_sha256.to_vec()),
                        file_length: Some(upload.file_length),
                        mimetype: Some(mime),
                        file_name: Some(filename),
                        caption,
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                match client.send_message(jid, wa_msg).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Document sent to {} via WhatsApp.",
                        jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send document: {}", e))),
                }
            }

            // ── send_audio ───────────────────────────────────────────────────
            "send_audio" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the audio");
                let path = pget!(get_str(&input, "media_path")).to_string();

                let (bytes, mime, _filename) = pget!(read_local_media(&path, "audio/ogg").await);
                // #1486: WhatsApp renders a tap-to-play voice bubble only when
                // the message carries ptt. Without it the same bytes arrive as
                // an unstyled file attachment. Default on, because the caller
                // that wants a plain audio FILE is the rare one, and pass
                // `voice_note: false` to get the old rendering.
                let ptt = crate::channels::whatsapp::voice_note::wants_voice_note(&input);
                let mime = crate::channels::whatsapp::voice_note::voice_note_mimetype(&mime, ptt);
                let upload =
                    pget!(upload_media(&client, bytes, wacore::download::MediaType::Audio).await);

                // Show "recording..." while the upload is in flight, exactly
                // as a human sending a voice note would appear (#1486).
                if ptt && let Err(e) = client.chatstate().send_recording(&jid).await {
                    tracing::warn!(error = %e, "WhatsApp: recording indicator failed");
                }

                let wa_msg = waproto::whatsapp::Message {
                    audio_message: Some(Box::new(waproto::whatsapp::message::AudioMessage {
                        url: Some(upload.url),
                        direct_path: Some(upload.direct_path),
                        media_key: Some(upload.media_key.to_vec()),
                        file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                        file_sha256: Some(upload.file_sha256.to_vec()),
                        file_length: Some(upload.file_length),
                        mimetype: Some(mime),
                        ptt: Some(ptt),
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                let sent = client.send_message(jid.clone(), wa_msg).await;
                // Clear the indicator whether or not the send worked, so the
                // chat is not left showing "recording..." forever.
                if ptt && let Err(e) = client.chatstate().send_paused(&jid).await {
                    tracing::warn!(error = %e, "WhatsApp: clearing recording indicator failed");
                }
                match sent {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "{} sent to {} via WhatsApp.",
                        if ptt { "Voice note" } else { "Audio" },
                        jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send audio: {}", e))),
                }
            }

            // ── send_video ───────────────────────────────────────────────────
            "send_video" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the video");
                let path = pget!(get_str(&input, "media_path")).to_string();
                let caption = input
                    .get("caption")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let (bytes, mime, _filename) = pget!(read_local_media(&path, "video/mp4").await);
                let upload =
                    pget!(upload_media(&client, bytes, wacore::download::MediaType::Video).await);

                let wa_msg = waproto::whatsapp::Message {
                    video_message: Some(Box::new(waproto::whatsapp::message::VideoMessage {
                        url: Some(upload.url),
                        direct_path: Some(upload.direct_path),
                        media_key: Some(upload.media_key.to_vec()),
                        file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                        file_sha256: Some(upload.file_sha256.to_vec()),
                        file_length: Some(upload.file_length),
                        mimetype: Some(mime),
                        caption,
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                match client.send_message(jid, wa_msg).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Video sent to {} via WhatsApp.",
                        jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send video: {}", e))),
                }
            }

            // ── send_sticker ─────────────────────────────────────────────────
            "send_sticker" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the sticker");
                let path = pget!(get_str(&input, "media_path")).to_string();

                let (bytes, mime, _filename) = pget!(read_local_media(&path, "image/webp").await);
                let upload =
                    pget!(upload_media(&client, bytes, wacore::download::MediaType::Sticker).await);

                let wa_msg = waproto::whatsapp::Message {
                    sticker_message: Some(Box::new(waproto::whatsapp::message::StickerMessage {
                        url: Some(upload.url),
                        direct_path: Some(upload.direct_path),
                        media_key: Some(upload.media_key.to_vec()),
                        file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                        file_sha256: Some(upload.file_sha256.to_vec()),
                        file_length: Some(upload.file_length),
                        mimetype: Some(mime),
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                match client.send_message(jid, wa_msg).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Sticker sent to {} via WhatsApp.",
                        jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send sticker: {}", e))),
                }
            }

            // ── send_location ────────────────────────────────────────────────
            "send_location" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the location");
                let lat = match get_f64(&input, "latitude") {
                    Some(v) => v,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing required 'latitude' parameter.".to_string(),
                        ));
                    }
                };
                let lng = match get_f64(&input, "longitude") {
                    Some(v) => v,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing required 'longitude' parameter.".to_string(),
                        ));
                    }
                };
                let name = input
                    .get("location_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let address = input
                    .get("location_address")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let wa_msg = waproto::whatsapp::Message {
                    location_message: Some(Box::new(waproto::whatsapp::message::LocationMessage {
                        degrees_latitude: Some(lat),
                        degrees_longitude: Some(lng),
                        name,
                        address,
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                match client.send_message(jid, wa_msg).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Location ({}, {}) sent to {} via WhatsApp.",
                        lat, lng, jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send location: {}", e))),
                }
            }

            // ── send_contact ─────────────────────────────────────────────────
            "send_contact" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the contact card");
                let contact_name = pget!(get_str(&input, "contact_name")).to_string();
                let contact_phone = pget!(get_str(&input, "contact_phone")).to_string();

                let vcard = build_vcard(&contact_name, &contact_phone);
                let wa_msg = waproto::whatsapp::Message {
                    contact_message: Some(Box::new(waproto::whatsapp::message::ContactMessage {
                        display_name: Some(contact_name.clone()),
                        vcard: Some(vcard),
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                match client.send_message(jid, wa_msg).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Contact '{}' sent to {} via WhatsApp.",
                        contact_name, jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to send contact: {}", e))),
                }
            }

            // ── react ────────────────────────────────────────────────────────
            "react" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the reaction");
                let msg_id = pget!(get_str(&input, "message_id")).to_string();
                let emoji = input
                    .get("emoji")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let from_me = input
                    .get("from_me")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let message_key = waproto::whatsapp::MessageKey {
                    remote_jid: Some(jid.to_string()),
                    from_me: Some(from_me),
                    id: Some(msg_id.clone()),
                    ..Default::default()
                };

                #[cfg(crates_publish)]
                let boxed_reaction = waproto::whatsapp::message::ReactionMessage {
                    key: Some(message_key),
                    text: if emoji.is_empty() {
                        None
                    } else {
                        Some(emoji.clone())
                    },
                    sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
                    ..Default::default()
                };
                #[cfg(not(crates_publish))]
                let boxed_reaction = Box::new(waproto::whatsapp::message::ReactionMessage {
                    key: Some(message_key),
                    text: if emoji.is_empty() {
                        None
                    } else {
                        Some(emoji.clone())
                    },
                    sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
                    ..Default::default()
                });

                let reaction_msg = waproto::whatsapp::Message {
                    reaction_message: Some(boxed_reaction),
                    ..Default::default()
                };
                match client.send_message(jid, reaction_msg).await {
                    Ok(_) => {
                        if emoji.is_empty() {
                            Ok(ToolResult::success(format!(
                                "Reaction removed from message {} for {}.",
                                msg_id, jid_str
                            )))
                        } else {
                            Ok(ToolResult::success(format!(
                                "Reaction '{}' set on message {} for {}.",
                                emoji, msg_id, jid_str
                            )))
                        }
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to send reaction: {}", e))),
                }
            }

            // ── send_poll ────────────────────────────────────────────────────
            "send_poll" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                gate_send!(self, jid_str, "the poll");
                let question = pget!(get_str(&input, "poll_question")).to_string();
                let opts: Vec<String> = match input.get("poll_options").and_then(|v| v.as_array()) {
                    Some(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                    None => {
                        return Ok(ToolResult::error(
                            "Missing required 'poll_options' parameter.".to_string(),
                        ));
                    }
                };
                if opts.len() < 2 {
                    return Ok(ToolResult::error(
                        "'poll_options' must have at least 2 options.".to_string(),
                    ));
                }
                if opts.len() > 12 {
                    return Ok(ToolResult::error(
                        "'poll_options' supports a maximum of 12 options.".to_string(),
                    ));
                }

                // #1482: built through the lib rather than by hand. A poll
                // whose MessageContextInfo carries no `message_secret` can
                // never have its votes decrypted - voters derive their
                // encryption key from it - and the hand-rolled version set
                // none, so every vote on a bot poll was unreadable by
                // construction. `polls().create` mints the secret, persists it
                // for the decode path, and picks the proto version WhatsApp
                // Web uses for the select count (v3 single, v1 multi).
                // WhatsApp encodes "how many options a voter may pick".
                // Default 1 (single choice); `multi_select: true` lets a voter
                // pick any number, which is what the count means at its max.
                let selectable = if input
                    .get("multi_select")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    opts.len() as u32
                } else {
                    1
                };
                match client
                    .polls()
                    .create(jid, &question, &opts, selectable)
                    .await
                {
                    Ok((result, _secret)) => {
                        // Votes name their options by hash, never by label, so
                        // a vote can only be read back against the poll's own
                        // option list. Remember it, keyed on the message id.
                        self.whatsapp_state
                            .polls
                            .remember(result.message_id, opts)
                            .await;
                        Ok(ToolResult::success(format!(
                            "Poll '{}' sent to {} via WhatsApp.",
                            question, jid_str
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to send poll: {}", e))),
                }
            }

            // ── typing ───────────────────────────────────────────────────────
            "typing" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                let active = input
                    .get("typing_active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                let result = if active {
                    client.chatstate().send_composing(&jid).await
                } else {
                    client.chatstate().send_paused(&jid).await
                };
                match result {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Typing {} sent to {}.",
                        if active { "composing" } else { "paused" },
                        jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to send typing indicator: {}",
                        e
                    ))),
                }
            }

            // ── mark_read ────────────────────────────────────────────────────
            "mark_read" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                let msg_id = pget!(get_str(&input, "message_id")).to_string();

                // Build read receipt node manually (send_protocol_receipt is pub(crate))
                // Receipt goes TO the message sender (jid), not ourselves
                let node = wacore_binary::builder::NodeBuilder::new("receipt")
                    .attrs([
                        ("id", msg_id.clone()),
                        ("type", "read".to_string()),
                        ("to", jid.to_string()),
                    ])
                    .build();
                match client.send_node(node).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Message {} marked as read for {}.",
                        msg_id, jid_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to mark message as read: {}",
                        e
                    ))),
                }
            }

            // ── status_update ────────────────────────────────────────────────
            // #1485. A status goes to a LIST of recipients, so it is the one
            // action here that can reach many people from one call. Both rails
            // apply: every target must be on the configured allowlist, and the
            // sends are paced, so there is no fire-and-forget mass-send path.
            "status_update" => {
                let text = pget!(get_str(&input, "message")).to_string();
                let (allowlist, pace) = {
                    let wa = &self.config_rx.borrow().channels.whatsapp;
                    (
                        wa.broadcast.allowed_targets.clone(),
                        broadcast::delay(wa.broadcast.min_delay_seconds),
                    )
                };
                let targets: Vec<String> = match input.get("targets").and_then(|v| v.as_array()) {
                    Some(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                    None => allowlist.clone(),
                };
                if targets.is_empty() {
                    return Ok(ToolResult::error(
                        "No status recipients. Set channels.whatsapp.broadcast.allowed_targets \
                         in config.toml, or pass 'targets'. An empty allowlist allows nobody on \
                         purpose."
                            .to_string(),
                    ));
                }
                let (allowed, refused) = broadcast::partition(&targets, &allowlist);
                if !refused.is_empty() {
                    return Ok(ToolResult::error(format!(
                        "{} target(s) are not on channels.whatsapp.broadcast.allowed_targets and \
                         NOTHING was sent: {}. Broadcast is opt-in per number; add them to the \
                         config first.",
                        refused.len(),
                        refused
                            .iter()
                            .map(|t| t.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
                let mut jids = Vec::with_capacity(allowed.len());
                for target in &allowed {
                    match broadcast::target_jid(target) {
                        Some(jid) => jids.push(jid),
                        None => {
                            return Ok(ToolResult::error(format!(
                                "'{target}' is not a usable phone number; nothing was sent."
                            )));
                        }
                    }
                }
                // Pacing is the point: charge the shared budget once per
                // recipient and sleep between them, so a status to N people
                // costs N slots and at least (N-1) * pace seconds.
                for (i, jid) in jids.iter().enumerate() {
                    let jid_str = jid.to_string();
                    gate_send!(self, jid_str, "the status update");
                    if i > 0 {
                        tokio::time::sleep(pace).await;
                    }
                }
                let opts = whatsapp_rust::features::StatusSendOptions::default();
                match client
                    .status()
                    .send_text(&text, broadcast::STATUS_BACKGROUND_ARGB, 0, &jids, opts)
                    .await
                {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Status update posted to {} allowlisted recipient(s), paced at {}s \
                         (at least {}s of wall clock).",
                        jids.len(),
                        pace.as_secs(),
                        broadcast::burst_floor(jids.len(), pace).as_secs()
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to post the status update: {e}"
                    ))),
                }
            }

            // ── list_newsletters ─────────────────────────────────────────────
            // #1485. Read-only, so no rails: it names what this account already
            // follows and is the only way to learn a newsletter JID to post to.
            "list_newsletters" => match client.newsletter().list_subscribed().await {
                Ok(list) if list.is_empty() => Ok(ToolResult::success(
                    "This WhatsApp account follows no newsletters (channels).".to_string(),
                )),
                Ok(list) => Ok(ToolResult::success(format!(
                    "{} newsletter(s):\n{}",
                    list.len(),
                    list.iter()
                        .map(|n| format!("- {} ({})", n.name, n.jid))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to list newsletters: {e}"
                ))),
            },

            // ── create_label / assign_label ──────────────────────────────────
            // #1485. Labels are private organisation on the owner's own account:
            // nothing is delivered to anyone, so the broadcast rails do not
            // apply and the send budget is not charged.
            "create_label" => {
                let name = pget!(get_str(&input, "message")).to_string();
                let label_id = input
                    .get("label_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        // App state is an upsert keyed by label_id, so a caller
                        // that supplies none gets a fresh label rather than
                        // silently renaming an existing one.
                        uuid::Uuid::new_v4().to_string()
                    });
                let color = input
                    .get("label_color")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                match client.labels().create_label(&label_id, &name, color).await {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Label '{name}' saved with id {label_id}."
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to save the label: {e}"))),
                }
            }

            "assign_label" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                let label_id = pget!(get_str(&input, "label_id")).to_string();
                let remove = input
                    .get("remove")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let result = if remove {
                    client.labels().remove_chat_label(&label_id, &jid).await
                } else {
                    client.labels().add_chat_label(&label_id, &jid).await
                };
                match result {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Label {label_id} {} chat with {jid_str}.",
                        if remove { "removed from" } else { "added to" }
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to update the label on {jid_str}: {e}"
                    ))),
                }
            }

            // ── block_contact ────────────────────────────────────────────────
            // #1487: the owner's first-line defence against a spamming number.
            // The resolved JID is logged so the block is auditable after the
            // fact, and the local mirror is updated so the inbound guard drops
            // that sender on the very next message without a server round trip.
            "block_contact" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                match client.blocking().block(&jid).await {
                    Ok(_) => {
                        tracing::info!(
                            target: "whatsapp",
                            jid = %jid,
                            "blocked contact on owner request (#1487)"
                        );
                        self.whatsapp_state.blocklist.insert(&jid.to_string()).await;
                        Ok(ToolResult::success(format!(
                            "Blocked {} on WhatsApp. Their messages no longer reach the bot.",
                            jid_str
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to block {jid_str}: {e}"))),
                }
            }

            // ── unblock_contact ──────────────────────────────────────────────
            "unblock_contact" => {
                let (jid, jid_str) =
                    pget!(resolve_jid(&input, &self.whatsapp_state, &self.config_rx).await);
                match client.blocking().unblock(&jid).await {
                    Ok(_) => {
                        tracing::info!(
                            target: "whatsapp",
                            jid = %jid,
                            "unblocked contact on owner request (#1487)"
                        );
                        self.whatsapp_state.blocklist.remove(&jid.to_string()).await;
                        Ok(ToolResult::success(format!(
                            "Unblocked {} on WhatsApp.",
                            jid_str
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to unblock {jid_str}: {e}"
                    ))),
                }
            }

            // ── list_blocked ─────────────────────────────────────────────────
            // Reads the server, not the mirror: the owner may have blocked
            // someone from their phone, and reporting a stale local set as
            // fact would be worse than one extra round trip. The mirror is
            // refreshed from the answer.
            "list_blocked" => match client.blocking().get_blocklist().await {
                Ok(entries) => {
                    let jids: Vec<String> = entries.iter().map(|e| e.jid.to_string()).collect();
                    self.whatsapp_state.blocklist.replace(jids.clone()).await;
                    if jids.is_empty() {
                        return Ok(ToolResult::success(
                            "No blocked contacts on this WhatsApp account.".to_string(),
                        ));
                    }
                    Ok(ToolResult::success(format!(
                        "{} blocked contact(s):\n{}",
                        jids.len(),
                        jids.join("\n")
                    )))
                }
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to fetch the blocklist: {e}"
                ))),
            },

            unknown => Ok(ToolResult::error(format!(
                "Unknown action '{}'. Valid actions: send, reply, delete, send_photo, \
                 send_document, send_audio, send_video, send_sticker, send_location, \
                 send_contact, react, send_poll, typing, mark_read, block_contact, \
                 unblock_contact, list_blocked, pin, unpin, forward, set_profile_name, \
                 set_profile_status, status_update, list_newsletters, create_label, \
                 assign_label",
                unknown
            ))),
        }
    }
}
