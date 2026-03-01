//! Trello Comment Handler
//!
//! Routes incoming card comments to the AI agent and posts responses back as comments.

use super::client::TrelloClient;
use super::models::Action;
use crate::brain::agent::AgentService;
use crate::services::SessionService;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Process a single Trello card comment: route to AI and post the response back.
pub async fn process_comment(
    comment: &Action,
    client: &TrelloClient,
    agent: Arc<AgentService>,
    session_svc: SessionService,
    shared_session: Arc<Mutex<Option<Uuid>>>,
    extra_sessions: Arc<Mutex<HashMap<String, Uuid>>>,
    owner_member_id: Option<&str>,
) {
    let card_id = match &comment.data.card {
        Some(c) => c.id.clone(),
        None => {
            tracing::warn!("Trello: comment action has no card reference, skipping");
            return;
        }
    };

    let card_name = comment
        .data
        .card
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("unknown card");

    let commenter_id = &comment.member_creator.id;
    let commenter_name = &comment.member_creator.full_name;
    let text = comment.data.text.trim();

    if text.is_empty() {
        return;
    }

    // Determine whether this commenter is the "owner" (first in allowed_users)
    let is_owner = owner_member_id
        .map(|id| id == commenter_id.as_str())
        .unwrap_or(false);

    // Resolve or create a session for this commenter
    let session_id = if is_owner {
        let shared = shared_session.lock().await;
        match *shared {
            Some(id) => id,
            None => {
                drop(shared);
                tracing::warn!("Trello: no active TUI session, creating one for owner");
                match session_svc.create_session(Some("Trello".to_string())).await {
                    Ok(s) => {
                        *shared_session.lock().await = Some(s.id);
                        s.id
                    }
                    Err(e) => {
                        tracing::error!("Trello: failed to create owner session: {}", e);
                        return;
                    }
                }
            }
        }
    } else {
        let mut map = extra_sessions.lock().await;
        match map.get(commenter_id.as_str()) {
            Some(id) => *id,
            None => {
                let title = format!("Trello: {}", commenter_name);
                match session_svc.create_session(Some(title)).await {
                    Ok(s) => {
                        map.insert(commenter_id.clone(), s.id);
                        s.id
                    }
                    Err(e) => {
                        tracing::error!(
                            "Trello: failed to create session for {}: {}",
                            commenter_name,
                            e
                        );
                        return;
                    }
                }
            }
        }
    };

    // Build context-enriched message
    let message = format!("[Trello card: {}]\n{}", card_name, text);

    tracing::info!(
        "Trello: comment on '{}' from {} — routing to agent (session {})",
        card_name,
        commenter_name,
        session_id
    );

    let response = match agent
        .send_message_with_tools(session_id, message, None)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Trello: agent error for card '{}': {}", card_name, e);
            return;
        }
    };

    let reply = response.content.trim().to_string();
    if reply.is_empty() {
        return;
    }

    // Split at ~4000 chars on newlines (Trello limit is ~16 384 chars per comment,
    // but we keep chunks short so they read well in the card activity feed).
    let chunks = split_comment(&reply, 4000);
    for chunk in chunks {
        if let Err(e) = client.add_comment_to_card(&card_id, &chunk).await {
            tracing::error!(
                "Trello: failed to post reply on card '{}': {}",
                card_name,
                e
            );
        }
    }
}

/// Split a long comment into chunks of at most `max_len` characters,
/// breaking preferably on newlines.
pub fn split_comment(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while remaining.len() > max_len {
        // Try to find a newline within the limit
        let split_at = match remaining[..max_len].rfind('\n') {
            Some(pos) => pos + 1,
            None => max_len,
        };
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }

    chunks
}
