//! Slack Message Handler
//!
//! Processes incoming Slack messages: text, allowlist enforcement,
//! session routing (owner shares TUI session, others get per-user sessions).
//!
//! Uses a module-level static for handler state because slack-morphism's
//! Socket Mode callbacks require plain function pointers (not closures).

use super::SlackState;
use crate::brain::agent::AgentService;
use crate::config::RespondTo;
use crate::services::SessionService;
use slack_morphism::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Socket Mode interaction callback — handles button clicks for tool approvals.
pub async fn on_interaction(
    event: SlackInteractionEvent,
    _client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let SlackInteractionEvent::BlockActions(block_actions) = event {
        let state = match HANDLER_STATE.get() {
            Some(s) => s.clone(),
            None => return Ok(()),
        };

        if let Some(actions) = block_actions.actions {
            for action in actions {
                let action_id = action.action_id.0.as_str();
                let (approved, always, id) = if let Some(id) = action_id.strip_prefix("approve:") {
                    (true, false, id.to_string())
                } else if let Some(id) = action_id.strip_prefix("always:") {
                    (true, true, id.to_string())
                } else if let Some(id) = action_id.strip_prefix("deny:") {
                    (false, false, id.to_string())
                } else {
                    continue;
                };
                state
                    .slack_state
                    .resolve_pending_approval(&id, approved, always)
                    .await;
            }
        }
    }
    Ok(())
}

/// Global handler state — set once by the agent before starting the listener.
pub static HANDLER_STATE: OnceLock<Arc<HandlerState>> = OnceLock::new();

/// Shared state for the Slack message handler callbacks.
pub struct HandlerState {
    pub agent: Arc<AgentService>,
    pub session_svc: SessionService,
    pub allowed: Arc<HashSet<String>>,
    pub extra_sessions: Arc<Mutex<HashMap<String, (Uuid, std::time::Instant)>>>,
    pub shared_session: Arc<Mutex<Option<Uuid>>>,
    pub slack_state: Arc<SlackState>,
    pub bot_token: String,
    pub respond_to: RespondTo,
    pub allowed_channels: Arc<HashSet<String>>,
    pub bot_user_id: Option<String>,
    pub idle_timeout_hours: Option<f64>,
}

/// Split a message into chunks that fit Slack's limit (conservative 3000 chars).
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

/// Socket Mode push event callback (function pointer — required by slack-morphism).
pub async fn on_push_event(
    event: SlackPushEventCallback,
    client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::debug!("Slack: received push event");
    match event.event {
        SlackEventCallbackBody::Message(msg) => {
            tracing::debug!(
                "Slack: message event from user={:?}, channel={:?}, bot_id={:?}",
                msg.sender.user,
                msg.origin.channel,
                msg.sender.bot_id
            );
            handle_message(&msg, client).await;
        }
        _ => {
            tracing::debug!("Slack: unhandled event type");
        }
    }
    Ok(())
}

/// Socket Mode error handler.
pub fn on_error(
    err: Box<dyn std::error::Error + Send + Sync>,
    _client: Arc<SlackHyperClient>,
    _states: SlackClientEventsUserState,
) -> HttpStatusCode {
    tracing::error!("Slack: socket mode error: {}", err);
    HttpStatusCode::OK
}

/// Handle an incoming Slack message event.
async fn handle_message(msg: &SlackMessageEvent, client: Arc<SlackHyperClient>) {
    let state = match HANDLER_STATE.get() {
        Some(s) => s.clone(),
        None => {
            tracing::error!("Slack: handler state not initialized");
            return;
        }
    };

    // Skip bot messages
    if msg.sender.bot_id.is_some() {
        tracing::debug!(
            "Slack: skipping bot message (bot_id={:?})",
            msg.sender.bot_id
        );
        return;
    }

    // Extract user ID
    let user_id = match &msg.sender.user {
        Some(uid) => uid.to_string(),
        None => {
            tracing::debug!("Slack: message has no sender user ID, ignoring");
            return;
        }
    };

    // Extract channel ID
    let channel_id = match &msg.origin.channel {
        Some(ch) => ch.to_string(),
        None => {
            tracing::debug!("Slack: message has no channel ID, ignoring");
            return;
        }
    };

    // Extract text
    let text = match &msg.content {
        Some(content) => match &content.text {
            Some(t) if !t.is_empty() => t.clone(),
            _ => {
                tracing::debug!("Slack: message has empty text, ignoring");
                return;
            }
        },
        None => {
            tracing::debug!("Slack: message has no content, ignoring");
            return;
        }
    };

    // Allowlist check — if allowed list is empty, accept all
    if !state.allowed.is_empty() && !state.allowed.contains(&user_id) {
        tracing::debug!("Slack: ignoring message from non-allowed user {}", user_id);
        return;
    }

    // respond_to / allowed_channels filtering — DMs (channel starts with 'D') always pass
    let is_dm = channel_id.starts_with('D');
    if !is_dm {
        // Check allowed_channels (empty = all channels allowed)
        if !state.allowed_channels.is_empty() && !state.allowed_channels.contains(&channel_id) {
            tracing::debug!(
                "Slack: ignoring message in non-allowed channel {}",
                channel_id
            );
            return;
        }

        match state.respond_to {
            RespondTo::DmOnly => {
                tracing::debug!("Slack: respond_to=dm_only, ignoring channel message");
                return;
            }
            RespondTo::Mention => {
                let mentioned = state
                    .bot_user_id
                    .as_ref()
                    .is_some_and(|bid| text.contains(&format!("<@{}>", bid)));
                if !mentioned {
                    tracing::debug!("Slack: respond_to=mention, bot not mentioned — ignoring");
                    return;
                }
            }
            RespondTo::All => {} // pass through
        }
    }

    // Strip <@BOT_ID> from text when responding to a mention
    let text = if !is_dm && state.respond_to == RespondTo::Mention {
        if let Some(ref bid) = state.bot_user_id {
            text.replace(&format!("<@{}>", bid), "").trim().to_string()
        } else {
            text
        }
    } else {
        text
    };

    let text_preview = &text[..text.len().min(50)];
    tracing::info!("Slack: message from {}: {}", user_id, text_preview);

    // Track owner's channel for proactive messaging
    let is_owner = state.allowed.is_empty()
        || state
            .allowed
            .iter()
            .next()
            .map(|a| *a == user_id)
            .unwrap_or(false);

    if is_owner {
        state
            .slack_state
            .set_owner_channel(channel_id.clone())
            .await;
    }

    // Resolve session: owner shares TUI session, others get per-user sessions
    let session_id = if is_owner {
        let shared = state.shared_session.lock().await;
        match *shared {
            Some(id) => id,
            None => {
                tracing::warn!("Slack: no active TUI session, creating one for owner");
                drop(shared);
                match state
                    .session_svc
                    .create_session(Some("Chat".to_string()))
                    .await
                {
                    Ok(session) => {
                        *state.shared_session.lock().await = Some(session.id);
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("Slack: failed to create session: {}", e);
                        return;
                    }
                }
            }
        }
    } else {
        let mut map = state.extra_sessions.lock().await;
        if let Some((old_id, last_activity)) = map.get(&user_id).copied() {
            if state
                .idle_timeout_hours
                .is_some_and(|h| last_activity.elapsed().as_secs() > (h * 3600.0) as u64)
            {
                let _ = state.session_svc.archive_session(old_id).await;
                map.remove(&user_id);
                let title = format!("Slack: {}", user_id);
                match state.session_svc.create_session(Some(title)).await {
                    Ok(session) => {
                        map.insert(user_id.clone(), (session.id, std::time::Instant::now()));
                        session.id
                    }
                    Err(e) => {
                        tracing::error!("Slack: failed to create session: {}", e);
                        return;
                    }
                }
            } else {
                map.insert(user_id.clone(), (old_id, std::time::Instant::now()));
                old_id
            }
        } else {
            let title = format!("Slack: {}", user_id);
            match state.session_svc.create_session(Some(title)).await {
                Ok(session) => {
                    map.insert(user_id.clone(), (session.id, std::time::Instant::now()));
                    session.id
                }
                Err(e) => {
                    tracing::error!("Slack: failed to create session: {}", e);
                    return;
                }
            }
        }
    };

    // For non-owner users, prepend sender identity so the agent knows who
    // it's talking to and doesn't assume it's the owner.
    let agent_input = if !is_owner {
        if is_dm {
            format!("[Slack DM from {user_id}]\n{text}")
        } else {
            format!("[Slack message from {user_id} in channel {channel_id}]\n{text}")
        }
    } else {
        text
    };

    // Register channel for approval routing, then send with approval callback
    state
        .slack_state
        .register_session_channel(session_id, channel_id.clone())
        .await;
    let approval_cb = SlackState::make_approval_callback(state.slack_state.clone());

    match state
        .agent
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
            let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
            let session = client.open_session(&token);

            for chunk in split_message(&tagged, 3000) {
                let request = SlackApiChatPostMessageRequest::new(
                    SlackChannelId::new(channel_id.clone()),
                    SlackMessageContent::new().with_text(chunk.to_string()),
                );
                if let Err(e) = session.chat_post_message(&request).await {
                    tracing::error!("Slack: failed to send reply: {}", e);
                }
            }
        }
        Err(e) => {
            tracing::error!("Slack: agent error: {}", e);
            let token = SlackApiToken::new(SlackApiTokenValue::from(state.bot_token.clone()));
            let session = client.open_session(&token);
            let error_msg = format!("Error: {}", e);
            let request = SlackApiChatPostMessageRequest::new(
                SlackChannelId::new(channel_id),
                SlackMessageContent::new().with_text(error_msg),
            );
            let _ = session.chat_post_message(&request).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_short_message() {
        let chunks = split_message("hello", 3000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_long_message() {
        let text = "a\n".repeat(2000);
        let chunks = split_message(&text, 3000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 3000);
        }
        let joined: String = chunks.into_iter().collect();
        assert_eq!(joined, text);
    }
}
