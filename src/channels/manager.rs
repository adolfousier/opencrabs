//! Channel Manager
//!
//! Manages the lifecycle of channel agents (Telegram, WhatsApp, Discord, Slack, Trello).
//! Spawns and stops channels dynamically when the config changes at runtime,
//! so that toggling `channels.*.enabled` in config.toml takes effect without restart.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::task::JoinHandle;

use crate::channels::ChannelFactory;
use crate::config::Config;

/// Manages running channel agents, allowing dynamic spawn/stop on config reload.
pub struct ChannelManager {
    handles: Mutex<HashMap<String, JoinHandle<()>>>,
    channel_factory: Arc<ChannelFactory>,
    db_pool: deadpool_sqlite::Pool,

    #[cfg(feature = "telegram")]
    telegram_state: Arc<crate::channels::telegram::TelegramState>,
    #[cfg(feature = "whatsapp")]
    whatsapp_state: Arc<crate::channels::whatsapp::WhatsAppState>,
    #[cfg(feature = "discord")]
    discord_state: Arc<crate::channels::discord::DiscordState>,
    #[cfg(feature = "slack")]
    slack_state: Arc<crate::channels::slack::SlackState>,
    #[cfg(feature = "trello")]
    trello_state: Arc<crate::channels::trello::TrelloState>,
}

impl ChannelManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        channel_factory: Arc<ChannelFactory>,
        db_pool: deadpool_sqlite::Pool,
        #[cfg(feature = "telegram")] telegram_state: Arc<crate::channels::telegram::TelegramState>,
        #[cfg(feature = "whatsapp")] whatsapp_state: Arc<crate::channels::whatsapp::WhatsAppState>,
        #[cfg(feature = "discord")] discord_state: Arc<crate::channels::discord::DiscordState>,
        #[cfg(feature = "slack")] slack_state: Arc<crate::channels::slack::SlackState>,
        #[cfg(feature = "trello")] trello_state: Arc<crate::channels::trello::TrelloState>,
    ) -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
            channel_factory,
            db_pool,
            #[cfg(feature = "telegram")]
            telegram_state,
            #[cfg(feature = "whatsapp")]
            whatsapp_state,
            #[cfg(feature = "discord")]
            discord_state,
            #[cfg(feature = "slack")]
            slack_state,
            #[cfg(feature = "trello")]
            trello_state,
        }
    }

    /// Compare running channels against config and spawn/stop as needed.
    pub fn reconcile(&self, config: &Config) {
        let mut handles = self.handles.lock().unwrap();

        #[cfg(feature = "telegram")]
        self.reconcile_telegram(config, &mut handles);

        #[cfg(feature = "whatsapp")]
        self.reconcile_whatsapp(config, &mut handles);

        #[cfg(feature = "discord")]
        self.reconcile_discord(config, &mut handles);

        #[cfg(feature = "slack")]
        self.reconcile_slack(config, &mut handles);

        #[cfg(feature = "trello")]
        self.reconcile_trello(config, &mut handles);
    }

    #[cfg(feature = "telegram")]
    fn reconcile_telegram(&self, config: &Config, handles: &mut HashMap<String, JoinHandle<()>>) {
        let tg = &config.channels.telegram;
        let has_valid_token = tg
            .token
            .as_ref()
            .map(|t| {
                if t.is_empty() || !t.contains(':') {
                    return false;
                }
                let parts: Vec<&str> = t.splitn(2, ':').collect();
                parts.len() == 2 && parts[0].parse::<u64>().is_ok() && parts[1].len() >= 30
            })
            .unwrap_or(false);

        let should_run = tg.enabled && has_valid_token;
        let is_running = handles.contains_key("telegram");

        if should_run && !is_running {
            if let Some(ref token) = tg.token {
                let token_hash = crate::config::profile::hash_token(token);
                if let Err(e) = crate::config::profile::acquire_token_lock("telegram", &token_hash)
                {
                    tracing::warn!("ChannelManager: Telegram token lock denied — {}", e);
                    return;
                }
                let agent = crate::channels::telegram::TelegramAgent::new(
                    self.channel_factory.create_agent_service(),
                    self.channel_factory.service_context(),
                    self.channel_factory.shared_session_id(),
                    self.telegram_state.clone(),
                    self.channel_factory.config_rx(),
                    crate::db::ChannelMessageRepository::new(self.db_pool.clone()),
                );
                tracing::info!(
                    "ChannelManager: spawning Telegram bot ({} allowed users)",
                    tg.allowed_users.len()
                );
                handles.insert("telegram".to_string(), agent.start(token.clone()));
            }
        } else if !should_run
            && is_running
            && let Some(handle) = handles.remove("telegram")
        {
            tracing::info!("ChannelManager: stopping Telegram bot");
            handle.abort();
        }
    }

    #[cfg(feature = "whatsapp")]
    fn reconcile_whatsapp(&self, config: &Config, handles: &mut HashMap<String, JoinHandle<()>>) {
        let wa = &config.channels.whatsapp;
        let should_run = wa.enabled;
        let is_running = handles.contains_key("whatsapp");

        if should_run && !is_running {
            let agent = crate::channels::whatsapp::WhatsAppAgent::new(
                self.channel_factory.create_agent_service(),
                self.channel_factory.service_context(),
                self.channel_factory.shared_session_id(),
                self.whatsapp_state.clone(),
                self.channel_factory.config_rx(),
                crate::db::ChannelMessageRepository::new(self.db_pool.clone()),
            );
            tracing::info!(
                "ChannelManager: spawning WhatsApp agent ({} allowed phones)",
                wa.allowed_phones.len()
            );
            handles.insert("whatsapp".to_string(), agent.start());
        } else if !should_run
            && is_running
            && let Some(handle) = handles.remove("whatsapp")
        {
            tracing::info!("ChannelManager: stopping WhatsApp agent");
            handle.abort();
        }
    }

    #[cfg(feature = "discord")]
    fn reconcile_discord(&self, config: &Config, handles: &mut HashMap<String, JoinHandle<()>>) {
        let dc = &config.channels.discord;
        let has_valid_token = dc
            .token
            .as_ref()
            .map(|t| !t.is_empty() && t.len() > 50)
            .unwrap_or(false);
        let should_run = dc.enabled && has_valid_token;
        let is_running = handles.contains_key("discord");

        if should_run && !is_running {
            if let Some(ref token) = dc.token {
                let token_hash = crate::config::profile::hash_token(token);
                if let Err(e) = crate::config::profile::acquire_token_lock("discord", &token_hash) {
                    tracing::warn!("ChannelManager: Discord token lock denied — {}", e);
                    return;
                }
                let agent = crate::channels::discord::DiscordAgent::new(
                    self.channel_factory.create_agent_service(),
                    self.channel_factory.service_context(),
                    self.channel_factory.shared_session_id(),
                    self.discord_state.clone(),
                    self.channel_factory.config_rx(),
                    crate::db::ChannelMessageRepository::new(self.db_pool.clone()),
                );
                tracing::info!(
                    "ChannelManager: spawning Discord bot ({} allowed users)",
                    dc.allowed_users.len()
                );
                handles.insert("discord".to_string(), agent.start(token.clone()));
            }
        } else if !should_run
            && is_running
            && let Some(handle) = handles.remove("discord")
        {
            tracing::info!("ChannelManager: stopping Discord bot");
            handle.abort();
        }
    }

    #[cfg(feature = "slack")]
    fn reconcile_slack(&self, config: &Config, handles: &mut HashMap<String, JoinHandle<()>>) {
        let sl = &config.channels.slack;
        let has_valid_tokens = sl
            .token
            .as_ref()
            .map(|t| !t.is_empty() && t.starts_with("xoxb-"))
            .unwrap_or(false)
            && sl
                .app_token
                .as_ref()
                .map(|t| !t.is_empty() && t.starts_with("xapp-"))
                .unwrap_or(false);
        let should_run = sl.enabled && has_valid_tokens;
        let is_running = handles.contains_key("slack");

        if should_run && !is_running {
            if let (Some(bot_tok), Some(app_tok)) = (sl.token.clone(), sl.app_token.clone()) {
                let token_hash = crate::config::profile::hash_token(&bot_tok);
                if let Err(e) = crate::config::profile::acquire_token_lock("slack", &token_hash) {
                    tracing::warn!("ChannelManager: Slack token lock denied — {}", e);
                    return;
                }
                let agent = crate::channels::slack::SlackAgent::new(
                    self.channel_factory.create_agent_service(),
                    self.channel_factory.service_context(),
                    self.channel_factory.shared_session_id(),
                    self.slack_state.clone(),
                    self.channel_factory.config_rx(),
                    crate::db::ChannelMessageRepository::new(self.db_pool.clone()),
                );
                tracing::info!(
                    "ChannelManager: spawning Slack bot ({} allowed users)",
                    sl.allowed_users.len()
                );
                handles.insert("slack".to_string(), agent.start(bot_tok, app_tok));
            }
        } else if !should_run
            && is_running
            && let Some(handle) = handles.remove("slack")
        {
            tracing::info!("ChannelManager: stopping Slack bot");
            handle.abort();
        }
    }

    #[cfg(feature = "trello")]
    fn reconcile_trello(&self, config: &Config, handles: &mut HashMap<String, JoinHandle<()>>) {
        let tr = &config.channels.trello;
        let has_valid_creds = tr
            .app_token
            .as_ref()
            .map(|k| !k.is_empty())
            .unwrap_or(false)
            && tr.token.as_ref().map(|t| !t.is_empty()).unwrap_or(false);
        let has_boards = !tr.board_ids.is_empty();
        let should_run = tr.enabled && has_valid_creds && has_boards;
        let is_running = handles.contains_key("trello");

        if should_run && !is_running {
            if let (Some(api_key), Some(api_token)) = (tr.app_token.clone(), tr.token.clone()) {
                let token_hash = crate::config::profile::hash_token(&api_token);
                if let Err(e) = crate::config::profile::acquire_token_lock("trello", &token_hash) {
                    tracing::warn!("ChannelManager: Trello token lock denied — {}", e);
                    return;
                }
                let agent = crate::channels::trello::TrelloAgent::new(
                    self.channel_factory.create_agent_service(),
                    self.channel_factory.service_context(),
                    tr.allowed_users.clone(),
                    self.channel_factory.shared_session_id(),
                    self.trello_state.clone(),
                    tr.board_ids.clone(),
                    tr.poll_interval_secs,
                    tr.session_idle_hours,
                );
                tracing::info!(
                    "ChannelManager: spawning Trello agent ({} boards)",
                    tr.board_ids.len()
                );
                handles.insert("trello".to_string(), agent.start(api_key, api_token));
            }
        } else if !should_run
            && is_running
            && let Some(handle) = handles.remove("trello")
        {
            tracing::info!("ChannelManager: stopping Trello agent");
            handle.abort();
        }
    }
}
