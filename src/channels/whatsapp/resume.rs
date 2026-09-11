//! Background-task resume producer for WhatsApp (#731).
//!
//! Mirrors Telegram's `build_enqueue_callback`: when a detached long command
//! finishes, resume the originating session and send the result to its chat.
//! The session→JID map is populated per turn in the handler (`register_session_jid`).

use super::WhatsAppState;
use crate::brain::agent::service::MessageEnqueueCallback;
use crate::channels::bg_resume::{self, AgentHolder};
use std::sync::Arc;

pub(crate) fn build_enqueue_callback(
    state: Arc<WhatsAppState>,
    agent_holder: AgentHolder,
    wa_cfg: crate::config::types::WhatsAppConfig,
) -> MessageEnqueueCallback {
    Arc::new(move |session_id, msg| {
        let state = state.clone();
        let agent_holder = agent_holder.clone();
        let wa_cfg = wa_cfg.clone();
        tokio::spawn(async move {
            let Some(jid_str) = state.session_jid(session_id).await else {
                tracing::warn!(
                    "[bg-resume] whatsapp: no chat jid for session {session_id}; dropping"
                );
                return;
            };
            let Some(agent) = bg_resume::upgrade(&agent_holder) else {
                tracing::warn!("[bg-resume] whatsapp: agent gone; dropping resume");
                return;
            };
            let Some(content) = bg_resume::run_resume_turn(
                agent,
                session_id,
                msg.context_text,
                "whatsapp",
                &jid_str,
            )
            .await
            else {
                return;
            };
            // Bounded wait rather than a drop (#1242). Worse here than on
            // the surfaces that check first: the turn above has already run,
            // so returning threw away a completed answer AND the provider
            // call that produced it.
            let Some(client) =
                crate::channels::transport_ready::await_transport("whatsapp", session_id, || {
                    state.client()
                })
                .await
            else {
                return;
            };
            let Ok(jid) = jid_str.parse::<wacore_binary::jid::Jid>() else {
                tracing::warn!("[bg-resume] whatsapp: bad jid '{jid_str}'; dropping delivery");
                return;
            };
            // #1407: bg-resume results are agent-output sends: gate them
            // through the shared limiter. Over-budget sends park in the
            // FIFO queue (the drainer flushes and persists them as the
            // rolling 24h window slides); owner-bound resumes bypass.
            let rl_owner = wa_cfg.is_owner(jid_str.split('@').next().unwrap_or(&jid_str));
            match state
                .rate_limiter
                .gate(&wa_cfg.rate_limit, &jid_str, &content, rl_owner)
                .await
            {
                super::rate_limit::GateOutcome::Queued { .. } => {
                    tracing::info!(
                        "[bg-resume] whatsapp: daily cap reached; result queued for drainer flush"
                    );
                    return;
                }
                super::rate_limit::GateOutcome::SendNow => {}
            }
            let out = waproto::whatsapp::Message {
                conversation: Some(content),
                ..Default::default()
            };
            if let Err(e) = client.send_message(jid, out).await {
                tracing::warn!("[bg-resume] whatsapp: send_message failed: {e}");
            }
        });
    })
}
