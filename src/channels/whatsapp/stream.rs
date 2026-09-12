//! Edit-in-place streaming delivery (#1408).
//!
//! Streaming used to post every intermediate chunk as its own WhatsApp
//! message, on the strength of a comment claiming the protocol has no edit.
//! It does: `Client::edit_message` exists and its window covers 15 minutes,
//! which is longer than virtually every turn. So a streamed turn is now one
//! living message that grows.
//!
//! Two constraints shape the design:
//!
//! * **Ordering.** The progress callback is synchronous and used to
//!   `tokio::spawn` one task per chunk, so two chunks could race and either
//!   could win the edit. Chunks now go down an unbounded channel to a single
//!   consumer task, which is the only thing that sends or edits. Order is the
//!   channel's order, by construction.
//! * **Edit volume.** Every edit is a stanza on the wire and counts against
//!   the #1407 limiter. The consumer drains everything already queued before
//!   acting, so a burst of five chunks costs one edit rather than five.
//!
//! Failure degrades, never drops: a rejected edit clears the tracked message
//! and the rest of the turn falls back to appending new messages.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;
use wacore_binary::jid::Jid;
use whatsapp_rust::client::Client;

use super::WhatsAppState;
use super::outbox::OutboxEntry;

/// WhatsApp's per-message text ceiling as the channel already applies it
/// (`split_message(&tagged, 4000)` at the old chunk loop).
pub(crate) const CHUNK_LIMIT: usize = 4000;

/// What to do with the next piece of streamed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// Edit the tracked message so it reads exactly this.
    Edit(String),
    /// Post this as a new message: nothing is tracked yet, or appending
    /// would overflow the per-message ceiling.
    Send(String),
}

/// Decide between growing the tracked message and starting a new one.
///
/// `current` is the full text the tracked message currently shows, or `None`
/// when the turn has not sent anything yet. Length is measured in characters
/// rather than bytes: the ceiling WhatsApp applies is a character count, and
/// a byte check would truncate accented or CJK text early.
pub(crate) fn plan(current: Option<&str>, chunk: &str, limit: usize) -> Delivery {
    let chunk = chunk.trim();
    let Some(body) = current else {
        return Delivery::Send(chunk.to_string());
    };
    let merged = format!("{body}\n\n{chunk}");
    if merged.chars().count() <= limit {
        Delivery::Edit(merged)
    } else {
        Delivery::Send(chunk.to_string())
    }
}

/// Join chunks that arrived while the consumer was busy into one payload, so
/// a burst costs one edit instead of one per chunk.
pub(crate) fn coalesce(chunks: &[String]) -> String {
    chunks
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Handle the progress callback writes streamed text to.
///
/// Cloneable and cheap; dropping every clone closes the channel, which is
/// how the consumer learns the turn is over.
#[derive(Clone)]
pub(crate) struct StreamSink {
    tx: mpsc::UnboundedSender<String>,
}

impl StreamSink {
    /// Queue a chunk for delivery. A closed channel means the consumer already
    /// exited (turn ended or cancelled); log it rather than dropping silently,
    /// because a lost chunk is content the user never sees.
    pub(crate) fn push(&self, text: String) {
        if let Err(e) = self.tx.send(text) {
            tracing::warn!(
                target: "whatsapp",
                error = %e,
                "streamed chunk arrived after the delivery task exited; not delivered"
            );
        }
    }
}

/// Everything the consumer needs that is not the channel itself.
pub(crate) struct StreamConfig {
    pub client: Arc<Client>,
    pub jid: Jid,
    pub session_id: Uuid,
    pub state: Arc<WhatsAppState>,
    pub rate_limit: crate::config::WaRateLimitConfig,
    pub is_owner: bool,
    /// Prefix stamped on the first message of the turn, e.g. the channel
    /// header. Subsequent chunks grow underneath it.
    pub header: String,
}

/// Start the single task that owns send/edit ordering for one turn.
///
/// Returns the sink for the progress callback and the task handle. Drop every
/// sink clone and await the handle to flush the turn.
pub(crate) fn spawn(config: StreamConfig) -> (StreamSink, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let handle = tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Drain whatever else is already queued: one edit for the burst.
            let mut batch = vec![first];
            while let Ok(next) = rx.try_recv() {
                batch.push(next);
            }
            let chunk = coalesce(&batch);
            if chunk.is_empty() {
                continue;
            }
            deliver(&config, &chunk).await;
        }
        // The turn is over; the final-response path decides what to do with
        // whatever is tracked, so leave the entry in place.
    });
    (StreamSink { tx }, handle)
}

/// Deliver one coalesced chunk, editing the tracked message when possible.
async fn deliver(config: &StreamConfig, chunk: &str) {
    let tracked = config.state.editable_outbound(config.session_id).await;
    let plan = plan(
        tracked.as_ref().map(|e| e.body.as_str()),
        chunk,
        CHUNK_LIMIT,
    );

    match (plan, tracked) {
        (Delivery::Edit(full), Some(entry)) => {
            // An edit is one stanza: pace it on the bucket, and drop rather
            // than queue under a saturated daily cap, matching how the old
            // chunk path treated ephemeral progress (#1407).
            if !config
                .state
                .rate_limiter
                .gate_ephemeral(&config.rate_limit, config.is_owner)
                .await
            {
                tracing::debug!(
                    target: "whatsapp",
                    "daily cap reached; skipping streamed edit"
                );
                return;
            }
            let message = waproto::whatsapp::Message {
                conversation: Some(full.clone()),
                ..Default::default()
            };
            match config
                .client
                .edit_message(config.jid.clone(), entry.message_id.clone(), message)
                .await
            {
                Ok(_) => {
                    config
                        .state
                        .update_outbound_body(config.session_id, full)
                        .await;
                }
                Err(e) => {
                    // Degrade, never drop: forget the tracked message so the
                    // rest of the turn appends, then post this chunk now so
                    // its content still reaches the user.
                    tracing::warn!(
                        target: "whatsapp",
                        error = %e,
                        message_id = %entry.message_id,
                        "streamed edit rejected; falling back to append for the rest of this turn"
                    );
                    config.state.clear_outbound(config.session_id).await;
                    send_new(config, chunk, false).await;
                }
            }
        }
        (Delivery::Send(text), tracked) => {
            // Overflowing the ceiling starts a fresh message, which becomes
            // the new edit target.
            let first_of_turn = tracked.is_none();
            send_new(config, &text, first_of_turn).await;
        }
        // `plan` only returns Edit when `current` was Some, so this arm is
        // unreachable; treat it as "nothing tracked" rather than panicking on
        // a future refactor.
        (Delivery::Edit(full), None) => {
            send_new(config, &full, true).await;
        }
    }
}

/// Post a new message and track it as the turn's edit target.
async fn send_new(config: &StreamConfig, text: &str, with_header: bool) {
    let body = if with_header && !config.header.is_empty() {
        format!("{}\n\n{}", config.header, text.trim())
    } else {
        text.trim().to_string()
    };
    for piece in super::handler::split_message(&body, CHUNK_LIMIT) {
        if !config
            .state
            .rate_limiter
            .gate_ephemeral(&config.rate_limit, config.is_owner)
            .await
        {
            tracing::debug!(
                target: "whatsapp",
                "daily cap reached; dropping streamed chunk"
            );
            continue;
        }
        let message = waproto::whatsapp::Message {
            conversation: Some(piece.to_string()),
            ..Default::default()
        };
        match config
            .client
            .send_message(config.jid.clone(), message)
            .await
        {
            Ok(result) => {
                config
                    .state
                    .record_outbound(
                        config.session_id,
                        OutboxEntry::new(result.message_id, piece),
                    )
                    .await;
            }
            Err(e) => {
                tracing::error!(target: "whatsapp", error = %e, "streamed chunk send failed");
            }
        }
    }
}

/// Bring the turn's tracked message up to the finished answer (#1408 AC1).
///
/// Returns false when the streamed intermediates have to stand as the
/// delivery: nothing was tracked, the finished text no longer fits one
/// message, or the server refused the edit. That is exactly the pre-#1408
/// behaviour, so a false here loses nothing.
pub(crate) async fn finalize(
    client: &Arc<Client>,
    jid: &Jid,
    state: &Arc<WhatsAppState>,
    session_id: Uuid,
    full_text: &str,
) -> bool {
    let Some(entry) = state.editable_outbound(session_id).await else {
        return false;
    };
    if entry.body.trim() == full_text.trim() {
        // The last streamed edit already reads exactly like the final answer.
        return true;
    }
    if full_text.chars().count() > CHUNK_LIMIT {
        tracing::debug!(
            target: "whatsapp",
            chars = full_text.chars().count(),
            "final answer exceeds one message; leaving the streamed intermediates in place"
        );
        return false;
    }
    let message = waproto::whatsapp::Message {
        conversation: Some(full_text.to_string()),
        ..Default::default()
    };
    match client
        .edit_message(jid.clone(), entry.message_id.clone(), message)
        .await
    {
        Ok(_) => {
            state.update_outbound_body(session_id, full_text).await;
            true
        }
        Err(e) => {
            tracing::warn!(
                target: "whatsapp",
                error = %e,
                message_id = %entry.message_id,
                "final edit rejected; streamed intermediates stand as the delivery"
            );
            state.clear_outbound(session_id).await;
            false
        }
    }
}
