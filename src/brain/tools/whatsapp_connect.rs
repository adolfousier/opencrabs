//! WhatsApp Connect Tool
//!
//! Agent-callable tool that initiates WhatsApp QR code pairing.
//! Shows a QR code in the terminal, waits for scan, then the same bot
//! continues running as the persistent message listener (no abort/respawn).

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::channels::ChannelFactory;
use crate::config::opencrabs_home;
use crate::brain::agent::{ProgressCallback, ProgressEvent};
use crate::channels::whatsapp::handler;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use qrcode::QrCode;
use wacore::types::events::Event;

use whatsapp_rust::bot::Bot;
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
use whatsapp_rust_ureq_http_client::UreqHttpClient;

/// Render a QR code as pure Unicode block characters (no ANSI escapes).
/// Uses upper/lower half blocks to pack two rows per line.
/// Includes a 4-module quiet zone (white border) required for scanning.
pub fn render_qr_unicode(data: &str) -> Option<String> {
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

/// Handle returned by `start_whatsapp_pairing` for async QR code / connection flow.
pub struct WhatsAppConnectHandle {
    /// Receives QR code data strings (may receive multiple as codes refresh).
    pub qr_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    /// Fires once when WhatsApp is successfully paired.
    pub connected_rx: tokio::sync::oneshot::Receiver<()>,
}

/// Start WhatsApp pairing — spawns a lightweight bot, returns channels for QR codes
/// and a connection signal.  Used by onboarding to show the QR popup without the
/// full message-handling pipeline that `WhatsAppConnectTool` wires up.
pub async fn start_whatsapp_pairing() -> Result<WhatsAppConnectHandle> {
    // 1. Create session storage
    let db_dir = opencrabs_home().join("whatsapp");
    std::fs::create_dir_all(&db_dir)
        .map_err(|e| super::error::ToolError::Internal(format!(
            "Failed to create WhatsApp data directory: {}", e
        )))?;
    let db_path = db_dir.join("session.db");

    let backend = Arc::new(
        crate::channels::whatsapp::sqlx_store::SqlxStore::new(
            db_path.to_string_lossy().as_ref(),
        )
        .await
        .map_err(|e| super::error::ToolError::Internal(format!(
            "Failed to open WhatsApp session store: {}", e
        )))?,
    );

    // 2. Set up signaling channels
    let (qr_tx, qr_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (conn_tx, conn_rx) = tokio::sync::oneshot::channel::<()>();
    let conn_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
        Arc::new(Mutex::new(Some(conn_tx)));

    // 3. Build bot with minimal event handler (QR + Connected only)
    let mut bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(TokioWebSocketTransportFactory::new())
        .with_http_client(UreqHttpClient::new())
        .on_event(move |event, _client| {
            let qr_tx = qr_tx.clone();
            let conn_tx = conn_tx.clone();
            async move {
                match event {
                    Event::PairingQrCode { ref code, .. } => {
                        let _ = qr_tx.send(code.clone());
                    }
                    Event::Connected(_) | Event::PairSuccess(_) => {
                        let mut tx = conn_tx.lock().await;
                        if let Some(sender) = tx.take() {
                            let _ = sender.send(());
                        }
                        tracing::info!("WhatsApp: paired successfully via onboarding");
                    }
                    _ => {}
                }
            }
        })
        .build()
        .await
        .map_err(|e| super::error::ToolError::Internal(format!(
            "Failed to create WhatsApp client: {}", e
        )))?;

    // 4. Spawn bot.run() in background
    tokio::spawn(async move {
        match bot.run().await {
            Ok(handle) => {
                if let Err(e) = handle.await {
                    tracing::error!("WhatsApp pairing bot task error: {:?}", e);
                }
            }
            Err(e) => {
                tracing::error!("WhatsApp pairing bot run error: {}", e);
            }
        }
    });

    Ok(WhatsAppConnectHandle {
        qr_rx,
        connected_rx: conn_rx,
    })
}

/// Tool that connects WhatsApp by generating a QR code for the user to scan.
pub struct WhatsAppConnectTool {
    progress: Option<ProgressCallback>,
    channel_factory: Arc<ChannelFactory>,
    whatsapp_state: Arc<crate::channels::whatsapp::WhatsAppState>,
}

impl WhatsAppConnectTool {
    pub fn new(
        progress: Option<ProgressCallback>,
        channel_factory: Arc<ChannelFactory>,
        whatsapp_state: Arc<crate::channels::whatsapp::WhatsAppState>,
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

        let backend = match crate::channels::whatsapp::sqlx_store::SqlxStore::new(
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

                Ok(ToolResult::success(
                    "WhatsApp connected successfully! Now listening for messages. \
                     Connection persists across restarts.\n\n\
                     IMPORTANT: Send a welcome message to the user RIGHT NOW \
                     using the `whatsapp_send` tool. Be wildly fun — talk like their \
                     best buddy from childhood. You can curse, be unhinged, hilarious, \
                     and totally informal. Use your personality, SOUL.md, and any context \
                     you have about the user. Make it memorable and uniquely yours. \
                     No generic corporate greetings — surprise them!"
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
