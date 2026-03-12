//! WhatsApp Agent
//!
//! Single bot instance — handles pairing, reconnection, and message processing.
//! Onboarding subscribes to QR/connected events via WhatsAppState.

use super::WhatsAppState;
use super::handler;
use crate::brain::agent::AgentService;
use crate::config::Config;
use crate::db::ChannelMessageRepository;
use crate::services::{ServiceContext, SessionService};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::store::Store;
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
    /// Always starts — if no session exists, emits QR events for onboarding.
    /// If already paired, reconnects and handles messages.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let db_path = crate::config::opencrabs_home()
                .join("whatsapp")
                .join("session.db");

            // Ensure parent directory exists
            if let Some(parent) = db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let backend = match Store::new(db_path.to_string_lossy().as_ref()).await {
                Ok(store) => Arc::new(store),
                Err(e) => {
                    let msg = format!(
                        "Failed to open session store at {}: {}",
                        db_path.display(),
                        e
                    );
                    tracing::error!("WhatsApp: {}", msg);
                    self.whatsapp_state.broadcast_error(&msg);
                    return;
                }
            };

            let cfg = self.config_rx.borrow().clone();
            tracing::info!(
                "WhatsApp agent running (STT={}, TTS={})",
                cfg.voice_config().stt_enabled,
                cfg.voice_config().tts_enabled,
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
                                wa_state.broadcast_qr(code);
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
                    let msg = format!("Failed to build WhatsApp bot: {}", e);
                    tracing::error!("WhatsApp: {}", msg);
                    self.whatsapp_state.broadcast_error(&msg);
                    return;
                }
            };

            match bot.run().await {
                Ok(handle) => {
                    if let Err(e) = handle.await {
                        let msg = format!("WhatsApp agent crashed: {:?}", e);
                        tracing::error!("{}", msg);
                        self.whatsapp_state.broadcast_error(&msg);
                    }
                }
                Err(e) => {
                    let msg = format!("WhatsApp agent failed to start: {}", e);
                    tracing::error!("{}", msg);
                    self.whatsapp_state.broadcast_error(&msg);
                }
            }
        })
    }
}
