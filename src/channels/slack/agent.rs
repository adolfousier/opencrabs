//! Slack Agent
//!
//! Agent struct and startup logic. Uses Socket Mode (WebSocket) —
//! no public HTTPS endpoint required, perfect for a CLI tool.

use super::SlackState;
use super::handler;
use crate::brain::agent::AgentService;
use crate::config::Config;
use crate::services::{ServiceContext, SessionService};
use slack_morphism::prelude::*;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Slack bot that forwards messages to the AgentService via Socket Mode
pub struct SlackAgent {
    agent_service: Arc<AgentService>,
    session_service: SessionService,
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    slack_state: Arc<SlackState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
}

impl SlackAgent {
    pub fn new(
        agent_service: Arc<AgentService>,
        service_context: ServiceContext,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        slack_state: Arc<SlackState>,
        config_rx: tokio::sync::watch::Receiver<Config>,
    ) -> Self {
        Self {
            agent_service,
            session_service: SessionService::new(service_context),
            shared_session_id,
            slack_state,
            config_rx,
        }
    }

    /// Start the bot as a background task using Socket Mode. Returns a JoinHandle.
    pub fn start(self, bot_token: String, app_token: String) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Validate tokens - Slack bot tokens start with "xoxb-" and app tokens with "xapp-"
            if bot_token.is_empty() || !bot_token.starts_with("xoxb-") {
                tracing::debug!("Slack bot token not configured or invalid, skipping bot start");
                return;
            }
            if app_token.is_empty() || !app_token.starts_with("xapp-") {
                tracing::debug!("Slack app token not configured or invalid, skipping bot start");
                return;
            }

            let cfg = self.config_rx.borrow().clone();
            tracing::info!(
                "Starting Slack bot via Socket Mode with {} allowed user(s)",
                cfg.channels.slack.allowed_users.len(),
            );

            let client = match SlackClientHyperConnector::new() {
                Ok(connector) => Arc::new(SlackClient::new(connector)),
                Err(e) => {
                    tracing::error!("Slack: failed to create HTTP connector: {}", e);
                    return;
                }
            };

            // Store connected state for proactive messaging
            self.slack_state
                .set_connected(client.clone(), bot_token.clone(), None)
                .await;

            // Fetch bot user ID via auth.test for @mention detection
            let bot_user_id = {
                let token = SlackApiToken::new(SlackApiTokenValue::from(bot_token.clone()));
                let session = client.open_session(&token);
                match session.auth_test().await {
                    Ok(resp) => {
                        let uid = resp.user_id.0.clone();
                        tracing::info!("Slack: bot user ID is {}", uid);
                        Some(uid)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Slack: auth.test failed, @mention detection disabled: {}",
                            e
                        );
                        None
                    }
                }
            };

            // Set up handler state (global static — one Slack instance per process)
            let handler_state = handler::HandlerState {
                agent: self.agent_service,
                session_svc: self.session_service,
                extra_sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
                shared_session: self.shared_session_id,
                slack_state: self.slack_state.clone(),
                bot_token: bot_token.clone(),
                bot_user_id,
                config_rx: self.config_rx,
            };
            handler::HANDLER_STATE
                .set(Arc::new(handler_state))
                .unwrap_or_else(|_| {
                    tracing::warn!("Slack: handler state already initialized");
                });

            let socket_mode_callbacks = SlackSocketModeListenerCallbacks::new()
                .with_push_events(handler::on_push_event)
                .with_interaction_events(handler::on_interaction);

            let listener_environment = Arc::new(
                SlackClientEventsListenerEnvironment::new(client)
                    .with_error_handler(handler::on_error),
            );

            let socket_mode_listener = SlackClientSocketModeListener::new(
                &SlackClientSocketModeConfig::new(),
                listener_environment,
                socket_mode_callbacks,
            );

            let slack_app_token = SlackApiToken::new(SlackApiTokenValue::from(app_token));

            tracing::info!("Slack: connecting via Socket Mode...");
            match socket_mode_listener.listen_for(&slack_app_token).await {
                Ok(()) => {
                    tracing::info!("Slack: Socket Mode connected");
                }
                Err(e) => {
                    tracing::error!("Slack: failed to connect Socket Mode: {}", e);
                    return;
                }
            }

            socket_mode_listener.serve().await;
        })
    }
}
