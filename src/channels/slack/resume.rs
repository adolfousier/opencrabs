//! Background-task resume producer for Slack (#731).
//!
//! Mirrors Telegram's `build_enqueue_callback`: when a detached long command
//! finishes, resume the originating session and post the result to its Slack
//! channel via `chat.postMessage` (same path as crash recovery in `cli/ui.rs`).

use super::SlackState;
use super::governor::GatedWrites;
use crate::brain::agent::service::MessageEnqueueCallback;
use crate::channels::bg_resume::{self, AgentHolder};
use slack_morphism::prelude::{
    SlackApiChatPostMessageRequest, SlackApiToken, SlackApiTokenValue, SlackMessageContent, SlackTs,
};
use std::sync::Arc;

pub(crate) fn build_enqueue_callback(
    state: Arc<SlackState>,
    agent_holder: AgentHolder,
) -> MessageEnqueueCallback {
    Arc::new(move |session_id, msg| {
        let state = state.clone();
        let agent_holder = agent_holder.clone();
        tokio::spawn(async move {
            let Some(channel) = state.session_channel(session_id).await else {
                tracing::warn!("[bg-resume] slack: no channel for session {session_id}; dropping");
                return;
            };
            let Some(agent) = bg_resume::upgrade(&agent_holder) else {
                tracing::warn!("[bg-resume] slack: agent gone; dropping resume");
                return;
            };
            let Some(content) = bg_resume::run_resume_turn(
                agent.clone(),
                session_id,
                msg.context_text,
                "slack",
                &channel,
            )
            .await
            else {
                return;
            };
            // Bounded wait rather than a drop (#1242). Like WhatsApp, this
            // check sits AFTER the turn, so returning threw away a completed
            // answer and the provider call behind it. Slack needs both halves
            // to post, so the pair is what readiness means here.
            let Some((token_val, client)) =
                crate::channels::transport_ready::await_transport("slack", session_id, || async {
                    match (state.bot_token().await, state.client().await) {
                        (Some(token), Some(client)) => Some((token, client)),
                        _ => None,
                    }
                })
                .await
            else {
                return;
            };
            let api_token = SlackApiToken::new(SlackApiTokenValue::from(token_val.clone()));
            let session = client.open_session(&api_token);
            let mut req = SlackApiChatPostMessageRequest::new(
                channel.clone().into(),
                SlackMessageContent::new().with_text(content),
            );
            // Land the answer in the thread that started the turn (#1988):
            // top-level sessions have no entry and keep posting at channel
            // root, same as before.
            if let Some(thread) = state.session_thread(session_id).await {
                req = req.with_thread_ts(SlackTs::new(thread));
            }
            if let Err(e) = session.post(&req).await {
                tracing::warn!("[bg-resume] slack: chat_post_message failed: {e}");
            }
            // The completion just delivered is the event the channel's waiting
            // step group is holding for (#1988): flip it now instead of
            // leaving the ⏳ line until the next inbound message.
            super::handler::flip_waiting_group(
                &client,
                &state,
                agent.as_ref(),
                token_val,
                &channel,
            )
            .await;
        });
    })
}
