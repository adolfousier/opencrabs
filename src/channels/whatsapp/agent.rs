//! WhatsApp Agent
//!
//! Agent struct and startup logic. Mirrors the Telegram agent pattern.

use super::WhatsAppState;
use super::handler;
use crate::brain::agent::AgentService;
use crate::config::Config;
use crate::db::ChannelMessageRepository;
use crate::services::{ServiceContext, SessionService};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::sqlx_store::SqlxStore;
use wacore::types::events::Event;
use whatsapp_rust::bot::Bot;
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
use whatsapp_rust_ureq_http_client::UreqHttpClient;

/// WhatsApp agent that forwards messages to the AgentService
pub struct WhatsAppAgent {
    agent_service: Arc<AgentService>,
    session_service: SessionService,
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    whatsapp_state: Arc<WhatsAppState>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    channel_msg_repo: ChannelMessageRepository,
}

impl WhatsAppAgent {
    pub fn new(
        agent_service: Arc<AgentService>,
        service_context: ServiceContext,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        whatsapp_state: Arc<WhatsAppState>,
        config_rx: tokio::sync::watch::Receiver<Config>,
        channel_msg_repo: ChannelMessageRepository,
    ) -> Self {
        Self {
            agent_service,
            session_service: SessionService::new(service_context),
            shared_session_id,
            whatsapp_state,
            config_rx,
            channel_msg_repo,
        }
    }

    /// Start as a background task. Returns JoinHandle.
    /// If already paired (session.db exists), reconnects silently.
    /// If not paired, QR events are logged.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let db_path = crate::config::opencrabs_home()
                .join("whatsapp")
                .join("session.db");

            // Ensure parent directory exists
            if let Some(parent) = db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let backend = match SqlxStore::new(db_path.to_string_lossy().as_ref()).await {
                Ok(store) => Arc::new(store),
                Err(e) => {
                    tracing::error!("WhatsApp: failed to open session store: {}", e);
                    return;
                }
            };

            // Only start if already paired — unpaired sessions should use the
            // whatsapp_connect tool which displays the QR code in the TUI.
            match backend.device_exists().await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::info!(
                        "WhatsApp: no paired session found — use 'connect WhatsApp' in chat to pair"
                    );
                    return;
                }
                Err(e) => {
                    tracing::warn!("WhatsApp: couldn't check device state: {}", e);
                    // Continue anyway — let the bot try
                }
            }

            let cfg = self.config_rx.borrow().clone();
            tracing::info!(
                "WhatsApp agent running (STT={}, TTS={})",
                cfg.voice.stt_enabled,
                cfg.voice.tts_enabled,
            );

            // Derive owner JID from first allowed phone (for proactive messaging)
            let owner_jid = cfg
                .channels
                .whatsapp
                .allowed_phones
                .first()
                .map(|p| format!("{}@s.whatsapp.net", p.trim_start_matches('+')));

            let agent = self.agent_service.clone();
            let session_svc = self.session_service.clone();
            let shared_session = self.shared_session_id.clone();
            let wa_state = self.whatsapp_state.clone();
            let config_rx = self.config_rx.clone();
            let channel_msg_repo = self.channel_msg_repo.clone();
            let extra_sessions: Arc<
                Mutex<std::collections::HashMap<String, (Uuid, std::time::Instant)>>,
            > = Arc::new(Mutex::new(std::collections::HashMap::new()));

            let owner_jid_clone = owner_jid.clone();

            let bot_result = Bot::builder()
                .with_backend(backend)
                .with_transport_factory(TokioWebSocketTransportFactory::new())
                .with_http_client(UreqHttpClient::new())
                .on_event(move |event, client| {
                    let agent = agent.clone();
                    let session_svc = session_svc.clone();
                    let extra_sessions = extra_sessions.clone();
                    let shared_session = shared_session.clone();
                    let wa_state = wa_state.clone();
                    let owner_jid = owner_jid_clone.clone();
                    let config_rx = config_rx.clone();
                    let channel_msg_repo = channel_msg_repo.clone();
                    async move {
                        match event {
                            Event::PairingQrCode { ref code, .. } => {
                                tracing::info!(
                                    "WhatsApp: QR code available (scan with your phone)"
                                );
                                // In static mode, just log — QR display is handled by the connect tool
                                tracing::debug!("WhatsApp QR: {}", code);
                            }
                            Event::Connected(_) => {
                                tracing::info!("WhatsApp: connected successfully");
                                wa_state
                                    .set_connected(client.clone(), owner_jid.clone())
                                    .await;
                            }
                            Event::PairSuccess(_) => {
                                tracing::info!("WhatsApp: pairing successful");
                            }
                            Event::Message(msg, info) => {
                                tracing::debug!("WhatsApp: Event::Message received");
                                handler::handle_message(
                                    *msg,
                                    info,
                                    client,
                                    agent,
                                    session_svc,
                                    extra_sessions,
                                    shared_session,
                                    wa_state.clone(),
                                    config_rx,
                                    channel_msg_repo,
                                )
                                .await;
                            }
                            Event::LoggedOut(_) => {
                                tracing::warn!("WhatsApp: logged out");
                            }
                            Event::Disconnected(_) => {
                                tracing::warn!("WhatsApp: disconnected");
                            }
                            other => {
                                tracing::debug!("WhatsApp: unhandled event: {:?}", other);
                            }
                        }
                    }
                })
                .build()
                .await;

            let mut bot = match bot_result {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("WhatsApp: failed to build bot: {}", e);
                    return;
                }
            };

            match bot.run().await {
                Ok(handle) => {
                    if let Err(e) = handle.await {
                        tracing::error!("WhatsApp agent task error: {:?}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("WhatsApp agent error: {}", e);
                }
            }
        })
    }
}
