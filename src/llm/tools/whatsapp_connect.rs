//! WhatsApp Connect Tool
//!
//! Agent-callable tool that initiates WhatsApp QR code pairing.
//! Shows a QR code in the terminal, waits for scan, then the same bot
//! continues running as the persistent message listener (no abort/respawn).

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::channels::ChannelFactory;
use crate::config::opencrabs_home;
use crate::llm::agent::{ProgressCallback, ProgressEvent};
use crate::whatsapp::handler;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use qrcode::QrCode;
use wacore::types::events::Event;
use waproto;
use whatsapp_rust::bot::Bot;
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
use whatsapp_rust_ureq_http_client::UreqHttpClient;

/// Render a QR code as pure Unicode block characters (no ANSI escapes).
/// Uses upper/lower half blocks to pack two rows per line.
/// Includes a 4-module quiet zone (white border) required for scanning.
fn render_qr_unicode(data: &str) -> Option<String> {
    let code = QrCode::new(data.as_bytes()).ok()?;
    let matrix = code.to_colors();
    let w = code.width();
    let quiet = 4;
    let total = w + quiet * 2;
    let mut out = String::new();

    let color_at = |x: usize, y: usize| -> qrcode::Color {
        if x < quiet || x >= quiet + w || y < quiet || y >= quiet + w {
            qrcode::Color::Light
        } else {
            matrix[(y - quiet) * w + (x - quiet)]
        }
    };

    let mut y = 0;
    while y < total {
        for x in 0..total {
            let top = color_at(x, y);
            let bot = if y + 1 < total {
                color_at(x, y + 1)
            } else {
                qrcode::Color::Light
            };
            let ch = match (top, bot) {
                (qrcode::Color::Light, qrcode::Color::Light) => ' ',
                (qrcode::Color::Dark, qrcode::Color::Dark) => '\u{2588}',
                (qrcode::Color::Dark, qrcode::Color::Light) => '\u{2580}',
                (qrcode::Color::Light, qrcode::Color::Dark) => '\u{2584}',
            };
            out.push(ch);
        }
        out.push('\n');
        y += 2;
    }
    Some(out)
}

/// Tool that connects WhatsApp by generating a QR code for the user to scan.
pub struct WhatsAppConnectTool {
    progress: Option<ProgressCallback>,
    channel_factory: Arc<ChannelFactory>,
    whatsapp_state: Arc<crate::whatsapp::WhatsAppState>,
}

impl WhatsAppConnectTool {
    pub fn new(
        progress: Option<ProgressCallback>,
        channel_factory: Arc<ChannelFactory>,
        whatsapp_state: Arc<crate::whatsapp::WhatsAppState>,
    ) -> Self {
        Self {
            progress,
            channel_factory,
            whatsapp_state,
        }
    }
}

#[async_trait]
impl Tool for WhatsAppConnectTool {
    fn name(&self) -> &str {
        "whatsapp_connect"
    }

    fn description(&self) -> &str {
        "Connect WhatsApp to OpenCrabs. Generates a QR code that the user scans with their \
         WhatsApp mobile app. Once scanned, WhatsApp messages from allowed phone numbers \
         will be routed to the agent. Call this when the user asks to connect or set up WhatsApp."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "allowed_phones": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Phone numbers to allow (E.164 format, e.g. '+15551234567'). If empty, all messages accepted."
                }
            }
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network, ToolCapability::SystemModification]
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let allowed_phones: Vec<String> = input
            .get("allowed_phones")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // 1. Create session storage
        let db_dir = opencrabs_home().join("whatsapp");
        if let Err(e) = std::fs::create_dir_all(&db_dir) {
            return Ok(ToolResult::error(format!(
                "Failed to create WhatsApp data directory: {}",
                e
            )));
        }
        let db_path = db_dir.join("session.db");

        let backend = match crate::whatsapp::sqlx_store::SqlxStore::new(
            db_path.to_string_lossy().as_ref(),
        )
        .await
        {
            Ok(store) => Arc::new(store),
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to open WhatsApp session store: {}",
                    e
                )));
            }
        };

        // 2. Set up signaling channels for QR code and connection events
        let (qr_tx, mut qr_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let connected_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
            Arc::new(Mutex::new(None));
        let (conn_sender, conn_receiver) = tokio::sync::oneshot::channel::<()>();
        *connected_tx.lock().await = Some(conn_sender);

        // 3. Prepare the FULL message handler state upfront so the bot handles
        //    messages immediately after pairing — no abort/respawn needed.
        let factory = self.channel_factory.clone();
        let agent = factory.create_agent_service();
        let session_svc =
            crate::services::SessionService::new(factory.service_context());
        let allowed: Arc<HashSet<String>> =
            Arc::new(allowed_phones.iter().cloned().collect());
        let voice_config = Arc::new(factory.voice_config().clone());
        let shared_session = factory.shared_session_id();
        let extra_sessions: Arc<Mutex<HashMap<String, uuid::Uuid>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // 4. Build bot with combined event handler (QR + Connected + Messages)
        let qr_tx_clone = qr_tx.clone();
        let connected_tx_clone = connected_tx.clone();
        let wa_state = self.whatsapp_state.clone();
        let owner_jid: Option<String> = allowed_phones
            .first()
            .map(|p| format!("{}@s.whatsapp.net", p.trim_start_matches('+')));

        let bot_result = Bot::builder()
            .with_backend(backend)
            .with_transport_factory(TokioWebSocketTransportFactory::new())
            .with_http_client(UreqHttpClient::new())
            .on_event(move |event, client| {
                let qr_tx = qr_tx_clone.clone();
                let connected_tx = connected_tx_clone.clone();
                let agent = agent.clone();
                let session_svc = session_svc.clone();
                let allowed = allowed.clone();
                let extra_sessions = extra_sessions.clone();
                let voice_config = voice_config.clone();
                let shared_session = shared_session.clone();
                let wa_state = wa_state.clone();
                let owner_jid = owner_jid.clone();
                async move {
                    match event {
                        Event::PairingQrCode { ref code, .. } => {
                            let _ = qr_tx.send(code.clone());
                        }
                        Event::Connected(_) | Event::PairSuccess(_) => {
                            // Store client for proactive messaging
                            wa_state.set_connected(client.clone(), owner_jid.clone()).await;

                            let mut tx = connected_tx.lock().await;
                            if let Some(sender) = tx.take() {
                                let _ = sender.send(());
                            }
                            tracing::info!("WhatsApp: connected and ready for messages");
                        }
                        Event::Message(msg, info) => {
                            handler::handle_message(
                                *msg,
                                info,
                                client,
                                agent,
                                session_svc,
                                allowed,
                                extra_sessions,
                                voice_config,
                                shared_session,
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
                return Ok(ToolResult::error(format!(
                    "Failed to create WhatsApp client: {}",
                    e
                )));
            }
        };

        // 5. Spawn bot.run() — this bot stays alive forever (handles messages after pairing)
        let _bot_handle = tokio::spawn(async move {
            match bot.run().await {
                Ok(handle) => {
                    if let Err(e) = handle.await {
                        tracing::error!("WhatsApp bot task error: {:?}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("WhatsApp bot run error: {}", e);
                }
            }
        });

        // 6. Wait for QR code, render with Unicode block characters
        let qr_displayed = tokio::time::timeout(Duration::from_secs(30), qr_rx.recv()).await;

        match qr_displayed {
            Ok(Some(qr_code)) => {
                match render_qr_unicode(&qr_code) {
                    Some(qr_text) => {
                        if let Some(ref cb) = self.progress {
                            cb(ProgressEvent::IntermediateText {
                                text: format!(
                                    "Scan this QR code with WhatsApp on your phone:\n\n{}",
                                    qr_text
                                ),
                            });
                        }
                    }
                    None => {
                        if let Some(ref cb) = self.progress {
                            cb(ProgressEvent::IntermediateText {
                                text: format!(
                                    "QR code generated but couldn't render. Raw code: {}",
                                    qr_code
                                ),
                            });
                        }
                    }
                }
            }
            Ok(None) => {
                return Ok(ToolResult::error(
                    "WhatsApp client closed before generating QR code. \
                     The session may already be paired — try restarting."
                        .to_string(),
                ));
            }
            Err(_) => {
                // Timeout waiting for QR — session might already be paired
                if conn_receiver.is_terminated() {
                    // Already connected, skip QR
                } else {
                    return Ok(ToolResult::error(
                        "Timed out waiting for QR code. Try again.".to_string(),
                    ));
                }
            }
        }

        // 7. Wait for connection (2 minute timeout)
        match tokio::time::timeout(Duration::from_secs(120), conn_receiver).await {
            Ok(Ok(())) => {
                // Update config to persist for auto-reconnect on restart
                let _ = crate::config::Config::write_key("channels.whatsapp", "enabled", "true");

                // Send a surprise welcome message to the owner
                let wa_state = self.whatsapp_state.clone();
                tokio::spawn(async move {
                    // Small delay to let the connection fully stabilize
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if let (Some(client), Some(owner_jid)) =
                        (wa_state.client().await, wa_state.owner_jid().await)
                    {
                        let jid: wacore_binary::jid::Jid = match owner_jid.parse() {
                            Ok(j) => j,
                            Err(_) => return,
                        };
                        let welcome = waproto::whatsapp::Message {
                            conversation: Some(format!(
                                "{}\n\n\
                                 WhatsApp link is live! \
                                 I can now read your messages and reach out to you anytime. \
                                 Just say the word!",
                                crate::whatsapp::handler::MSG_HEADER,
                            )),
                            ..Default::default()
                        };
                        if let Err(e) = client.send_message(jid, welcome).await {
                            tracing::warn!("WhatsApp: failed to send welcome message: {}", e);
                        }
                    }
                });

                Ok(ToolResult::success(
                    "WhatsApp connected successfully! The bot is now listening for messages. \
                     Connection persists across restarts."
                        .to_string(),
                ))
            }
            Ok(Err(_)) => Ok(ToolResult::error(
                "Connection channel closed unexpectedly. Try again.".to_string(),
            )),
            Err(_) => Ok(ToolResult::error(
                "QR code expired or connection timed out (2 minutes). \
                 Run the tool again to get a new QR code."
                    .to_string(),
            )),
        }
    }
}
