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
use whatsapp_rust::TokioRuntime;
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
            // Spawn the outbound rate-limit drainer exactly once per
            // process (#1407). The drainer snapshots the rate_limit
            // config here; later config changes reach the inline gate
            // paths immediately (they re-read config per turn) but the
            // drainer keeps this snapshot until restart (v1 tradeoff,
            // documented in rate_limit.rs).
            let rl_cfg = self.config_rx.borrow().channels.whatsapp.rate_limit.clone();
            super::rate_limit::spawn_drainer(self.whatsapp_state.clone(), rl_cfg);

            let db_path = crate::config::opencrabs_home()
                .join("whatsapp")
                .join("session.db");

            // Ensure parent directory exists
            if let Some(parent) = db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            // `with_backend` takes the backend by value now (it wraps it
            // internally); the old blanket `Backend for Arc<T>` impl is gone.
            let backend = match Store::new(db_path.to_string_lossy().as_ref()).await {
                Ok(store) => store,
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
            let owner_jid_clone = owner_jid.clone();

            let builder = Bot::builder();
            #[cfg(crates_publish)]
            let builder = builder.with_backend(std::sync::Arc::new(backend));
            #[cfg(not(crates_publish))]
            let builder = builder.with_backend(backend);
            let bot_result = builder
                .with_transport_factory(TokioWebSocketTransportFactory::new())
                .with_http_client(UreqHttpClient::new())
                .with_runtime(TokioRuntime)
                .on_event(move |event, client| {
                    let agent = agent.clone();
                    let session_svc = session_svc.clone();
                    let shared_session = shared_session.clone();
                    let wa_state = wa_state.clone();
                    let owner_jid = owner_jid_clone.clone();
                    let config_rx = config_rx.clone();
                    let channel_msg_repo = channel_msg_repo.clone();
                    async move {
                        match &*event {
                            Event::PairingQrCode { code, .. } => {
                                tracing::info!(
                                    "WhatsApp: QR code available (scan with your phone)"
                                );
                                wa_state.broadcast_qr(code);
                            }
                            Event::PairSuccess(s) => {
                                // The paired account's JID IS the owner (the
                                // person who scanned). Pin them as the
                                // authoritative owner so a config mismatch can
                                // never lock them out, and ensure they are in
                                // the allow list. Numbers are stored WITHOUT a
                                // leading '+' (matching sender_phone); the
                                // handler's wa_should_respond normalises both
                                // sides, so '+'-prefixed legacy entries still
                                // match.
                                let full = s.id.to_string();
                                let num = full
                                    .split('@')
                                    .next()
                                    .unwrap_or(&full)
                                    .split(':')
                                    .next()
                                    .unwrap_or(&full)
                                    .trim_start_matches('+')
                                    .to_string();
                                if num.is_empty() {
                                    tracing::warn!(
                                        "WhatsApp: pairing successful but could not extract \
                                         owner number from JID '{full}'"
                                    );
                                } else {
                                    tracing::info!("WhatsApp: pairing successful — owner is {num}");
                                    if let Err(e) = Config::write_key(
                                        "channels.whatsapp",
                                        "bot_owner",
                                        &format!("[\"{num}\"]"),
                                    ) {
                                        tracing::warn!(
                                            "WhatsApp: failed to persist bot_owner: {e}"
                                        );
                                    }
                                    // Append to allowed_phones if not already present.
                                    let mut allowed: Vec<String> =
                                        config_rx.borrow().channels.whatsapp.allowed_phones.clone();
                                    let present =
                                        allowed.iter().any(|a| a.trim_start_matches('+') == num);
                                    if !present {
                                        allowed.push(num.clone());
                                        let json = format!(
                                            "[{}]",
                                            allowed
                                                .iter()
                                                .map(|a| format!("\"{a}\""))
                                                .collect::<Vec<_>>()
                                                .join(",")
                                        );
                                        if let Err(e) = Config::write_key(
                                            "channels.whatsapp",
                                            "allowed_phones",
                                            &json,
                                        ) {
                                            tracing::warn!(
                                                "WhatsApp: failed to persist allowed_phones: {e}"
                                            );
                                        }
                                    }
                                    // Make the freshly-paired owner available to
                                    // the Connected handler (which sends the
                                    // confirmation greeting once the socket is
                                    // ready) and to the whatsapp_send tool.
                                    wa_state
                                        .set_owner_jid(format!("{num}@s.whatsapp.net"))
                                        .await;
                                    // Flag this as a fresh pairing so the
                                    // Connected handler knows to fire the
                                    // one-time greeting (not suppress it as a
                                    // routine restart).
                                    wa_state.set_first_pair_pending();
                                }
                            }
                            Event::Connected(_) => {
                                tracing::info!("WhatsApp: connected successfully");
                                // Prefer the freshly-paired owner (set on
                                // PairSuccess) over the startup-derived one,
                                // which is None on a first-time pairing.
                                let owner = match wa_state.owner_jid().await {
                                    Some(j) => Some(j),
                                    None => owner_jid.clone(),
                                };
                                // Greet only on a fresh pairing (first-time or
                                // #1488: announce availability once per
                                // connection. Without it the paired account
                                // reads as permanently offline to everyone it
                                // talks to, and WhatsApp withholds presence
                                // updates from a client that never publishes
                                // its own. Non-fatal: a failure costs presence,
                                // not messaging.
                                if let Err(e) = client.presence().set_available().await {
                                    tracing::warn!(
                                        target: "whatsapp",
                                        error = %e,
                                        "could not publish availability; the account will \
                                         appear offline and contact presence may not arrive"
                                    );
                                }
                                // re-pair after reset), not on every app restart
                                // or reconnect. The `first_pair_pending` flag is
                                // set by PairSuccess and consumed here: it is
                                // `true` exactly once per pairing, so a plain
                                // restart (where no PairSuccess fires) never
                                // triggers the greeting. The `was_connected`
                                // guard still suppresses keepalive reconnects
                                // within the same session.
                                let was_connected = wa_state.is_connected().await;
                                wa_state.set_connected(client.clone(), owner.clone()).await;
                                if was_connected {
                                    tracing::debug!(
                                        "WhatsApp: reconnected — suppressing duplicate \
                                         confirmation greeting"
                                    );
                                } else if wa_state.take_first_pair_pending() {
                                    // Fresh pairing: a real agent turn into the
                                    // owner's self-chat. Spawned so the event loop
                                    // is never blocked by a full agent turn.
                                    if let Some(jid) = owner {
                                        let num = jid.split('@').next().unwrap_or(&jid).to_string();
                                        tokio::spawn(handler::send_connection_greeting(
                                            client.clone(),
                                            agent.clone(),
                                            session_svc.clone(),
                                            wa_state.clone(),
                                            num,
                                        ));
                                    } else {
                                        tracing::warn!(
                                            "WhatsApp: connected but no owner number known — \
                                             skipping confirmation greeting"
                                        );
                                    }
                                } else {
                                    tracing::debug!(
                                        "WhatsApp: connected (app restart) — no fresh pair, \
                                         staying silent"
                                    );
                                }
                            }
                            Event::Message(msg, info) => {
                                tracing::debug!("WhatsApp: Event::Message received");
                                // Spawned onto its own task: the agent turn is a
                                // very large async state machine, and polling it
                                // inline inside the event-loop future overflows
                                // the worker stack (and would block the loop).
                                tokio::spawn(handler::handle_message(
                                    (**msg).clone(),
                                    (**info).clone(),
                                    client,
                                    agent,
                                    session_svc,
                                    shared_session,
                                    wa_state.clone(),
                                    config_rx,
                                    channel_msg_repo,
                                ));
                            }
                            Event::LoggedOut(_) => {
                                tracing::warn!("WhatsApp: logged out");
                            }
                            Event::Disconnected(_) => {
                                tracing::warn!("WhatsApp: disconnected");
                            }
                            Event::Receipt(receipt) => {
                                // A message we sent was accepted/delivered.
                                // `Delivered` is the normal recipient receipt;
                                // `Sender` is what a self-chat send gets back —
                                // the bot is paired AS the owner, so its replies
                                // go to the owner's own devices, which ack with
                                // `sender`, not `delivered`. Either one means the
                                // message landed, so surface the id for the
                                // onboarding connection test.
                                if matches!(
                                    receipt.r#type,
                                    wacore::types::presence::ReceiptType::Delivered
                                        | wacore::types::presence::ReceiptType::Sender
                                ) {
                                    for id in &receipt.message_ids {
                                        wa_state.broadcast_delivered(id);
                                    }
                                }
                            }
                            other => {
                                tracing::debug!("WhatsApp: unhandled event: {:?}", other);
                            }
                        }
                    }
                })
                .build()
                .await;

            #[cfg(not(crates_publish))]
            let bot = match bot_result {
                Ok(b) => b,
                Err(e) => {
                    let msg = format!("Failed to build WhatsApp bot: {}", e);
                    tracing::error!("WhatsApp: {}", msg);
                    self.whatsapp_state.broadcast_error(&msg);
                    return;
                }
            };
            #[cfg(crates_publish)]
            let mut bot = match bot_result {
                Ok(b) => b,
                Err(e) => {
                    let msg = format!("Failed to build WhatsApp bot: {}", e);
                    tracing::error!("WhatsApp: {}", msg);
                    self.whatsapp_state.broadcast_error(&msg);
                    return;
                }
            };

            // `run()` drives the bot until the connection ends. The newer
            // upstream API returns `()` (no Result, no separate handle —
            // build/credential failures surface before this point). The live
            // client is published to WhatsAppState from the Connected event
            // callback, so we don't need a handle here.
            #[cfg(not(crates_publish))]
            bot.run().await;
            #[cfg(crates_publish)]
            if let Err(e) = bot.run().await {
                tracing::warn!(error = %e, "WhatsApp bot run failed");
            }
            tracing::info!("WhatsApp: bot run loop exited");
        })
    }
}
