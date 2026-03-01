//! Trello Agent
//!
//! Polls Trello board(s) every 30 seconds for new card comments
//! and routes them to the AI agent.

use super::TrelloState;
use super::client::TrelloClient;
use super::handler;
use crate::brain::agent::AgentService;
use crate::services::{ServiceContext, SessionService};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

const POLL_INTERVAL_SECS: u64 = 30;

/// Background agent that polls Trello boards for new card comments.
pub struct TrelloAgent {
    agent_service: Arc<AgentService>,
    service_context: ServiceContext,
    allowed_users: Vec<String>,
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    trello_state: Arc<TrelloState>,
    board_ids: Vec<String>,
}

impl TrelloAgent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_service: Arc<AgentService>,
        service_context: ServiceContext,
        allowed_users: Vec<String>,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        trello_state: Arc<TrelloState>,
        board_ids: Vec<String>,
    ) -> Self {
        Self {
            agent_service,
            service_context,
            allowed_users,
            shared_session_id,
            trello_state,
            board_ids,
        }
    }

    /// Spawn the polling loop as a background task.
    pub fn start(self, api_key: String, api_token: String) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if api_key.is_empty() || api_token.is_empty() {
                tracing::warn!("Trello: credentials missing, not starting agent");
                return;
            }

            let client = TrelloClient::new(&api_key, &api_token);

            // Verify credentials and get bot member ID
            let bot_member = match client.get_member_me().await {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("Trello: credential verification failed: {}", e);
                    return;
                }
            };

            tracing::info!(
                "Trello: authenticated as @{} ({}), polling {} board(s) every {}s",
                bot_member.username,
                bot_member.id,
                self.board_ids.len(),
                POLL_INTERVAL_SECS,
            );

            self.trello_state
                .set_bot_member_id(bot_member.id.clone())
                .await;
            self.trello_state.set_credentials(api_key, api_token).await;
            self.trello_state.set_connected(true).await;

            let session_svc = SessionService::new(self.service_context.clone());
            let extra_sessions: Arc<Mutex<HashMap<String, Uuid>>> =
                Arc::new(Mutex::new(HashMap::new()));

            // Owner = first allowed_user (shares TUI session)
            let owner_member_id = self.allowed_users.first().cloned();

            // Set last_checked BEFORE entering the loop so we don't replay
            // comments that existed before the agent started.
            let mut last_checked = chrono::Utc::now().to_rfc3339();

            loop {
                // Sleep first — on startup we want to establish last_checked
                // and then start polling from that point forward.
                tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;

                // Snapshot "now" before fetching, so any comments that arrive
                // during processing are caught in the next tick.
                let tick_time = chrono::Utc::now().to_rfc3339();

                for board_id in &self.board_ids {
                    let actions = match client
                        .get_board_actions_since(board_id, &last_checked)
                        .await
                    {
                        Ok(a) => a,
                        Err(e) => {
                            tracing::warn!(
                                "Trello: failed to fetch actions for board {}: {}",
                                board_id,
                                e
                            );
                            continue;
                        }
                    };

                    for action in &actions {
                        // Only process card comments
                        if action.action_type != "commentCard" {
                            continue;
                        }

                        // Skip the bot's own comments
                        if action.id_member_creator == bot_member.id {
                            continue;
                        }

                        // Apply allowed_users filter if non-empty
                        if !self.allowed_users.is_empty()
                            && !self.allowed_users.contains(&action.id_member_creator)
                        {
                            tracing::debug!(
                                "Trello: skipping comment from non-allowed member {}",
                                action.id_member_creator
                            );
                            continue;
                        }

                        handler::process_comment(
                            action,
                            &client,
                            self.agent_service.clone(),
                            session_svc.clone(),
                            self.shared_session_id.clone(),
                            extra_sessions.clone(),
                            owner_member_id.as_deref(),
                        )
                        .await;
                    }
                }

                last_checked = tick_time;
            }
        })
    }
}
