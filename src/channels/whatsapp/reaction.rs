//! Self-reactions on completion turns (#1409).
//!
//! The lib has supported `ReactionMessage` all along and the `react` action on
//! `whatsapp_send` already constructs one, but the channel never reacted on
//! its own initiative: on Telegram the crab acknowledges a finished task with
//! an emoji, and WhatsApp stayed silent. What was missing was not the proto,
//! it was knowing WHICH message to react to, which is what the outbox (#1408)
//! now provides.

use std::sync::Arc;

use uuid::Uuid;
use wacore_binary::jid::Jid;
use whatsapp_rust::client::Client;

use super::WhatsAppState;

/// Emoji the crab uses to acknowledge a finished multi-step turn. Kept to the
/// utilitarian Telegram set rather than anything decorative.
pub(crate) const COMPLETION_EMOJI: &str = "✅";

/// Build the reaction proto for one of OUR OWN messages.
///
/// `from_me` is always true here: this reacts to something the bot sent, and
/// a false would address a message the recipient sent with the same id, which
/// either does not exist or is the wrong message entirely.
pub(crate) fn build_self_reaction(
    chat_jid: &str,
    message_id: &str,
    emoji: &str,
    timestamp_ms: i64,
) -> waproto::whatsapp::Message {
    let key = waproto::whatsapp::MessageKey {
        remote_jid: Some(chat_jid.to_string()),
        from_me: Some(true),
        id: Some(message_id.to_string()),
        ..Default::default()
    };
    let reaction = waproto::whatsapp::message::ReactionMessage {
        key: Some(key),
        // An empty emoji is how WhatsApp encodes REMOVING a reaction, so it
        // is carried as `None` rather than `Some("")`.
        text: if emoji.is_empty() {
            None
        } else {
            Some(emoji.to_string())
        },
        sender_timestamp_ms: Some(timestamp_ms),
        ..Default::default()
    };
    #[cfg(crates_publish)]
    let boxed = reaction;
    #[cfg(not(crates_publish))]
    let boxed = Box::new(reaction);
    waproto::whatsapp::Message {
        reaction_message: Some(boxed),
        ..Default::default()
    }
}

/// React to the message this turn left in the outbox.
///
/// Never turn-fatal: a missing outbox entry or a rejected send is logged and
/// the turn is already delivered either way. The acknowledgement is a nicety,
/// not part of the answer.
pub(crate) async fn acknowledge_completion(
    client: &Arc<Client>,
    jid: &Jid,
    state: &Arc<WhatsAppState>,
    session_id: Uuid,
    emoji: &str,
) {
    let Some(entry) = state.editable_outbound(session_id).await else {
        tracing::debug!(
            target: "whatsapp",
            session = %session_id,
            "no tracked message to acknowledge; skipping completion reaction"
        );
        return;
    };
    let message = build_self_reaction(
        &jid.to_string(),
        &entry.message_id,
        emoji,
        chrono::Utc::now().timestamp_millis(),
    );
    if let Err(e) = client.send_message(jid.clone(), message).await {
        tracing::warn!(
            target: "whatsapp",
            error = %e,
            message_id = %entry.message_id,
            "completion reaction failed; the answer itself was delivered"
        );
    }
}
