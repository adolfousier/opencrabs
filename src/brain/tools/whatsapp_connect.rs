//! WhatsApp Connect Tool
//!
//! Agent-callable tool that initiates WhatsApp QR code pairing.
//! Subscribes to QR/connected events from the single WhatsApp agent bot
//! managed by ChannelManager. No separate bot instance is ever created.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::brain::agent::{ProgressCallback, ProgressEvent};
use crate::config::opencrabs_home;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use qrcode::QrCode;

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
            // Inverted mapping: light modules = bright block, dark modules = space.
            // This is the qrencode -t UTF8 convention — white blocks on dark terminal
            // background — which phone cameras read reliably without needing a white bg.
            let ch = match (top, bot) {
                (qrcode::Color::Light, qrcode::Color::Light) => '\u{2588}', // full bright
                (qrcode::Color::Dark, qrcode::Color::Dark) => ' ',          // transparent dark
                (qrcode::Color::Light, qrcode::Color::Dark) => '\u{2580}',  // upper bright
                (qrcode::Color::Dark, qrcode::Color::Light) => '\u{2584}',  // lower bright
            };
            out.push(ch);
        }
        out.push('\n');
        y += 2;
    }
    Some(out)
}

/// Handle returned by `subscribe_whatsapp_pairing` for QR / connection events.
/// No bot is created — subscribes to the single agent bot via WhatsAppState.
pub struct WhatsAppConnectHandle {
    /// Receives QR code data strings from the agent bot.
    pub qr_rx: tokio::sync::broadcast::Receiver<String>,
    /// Fires once when WhatsApp connects.
    pub connected_rx: tokio::sync::broadcast::Receiver<()>,
    /// Receives error messages from the agent bot.
    pub error_rx: tokio::sync::broadcast::Receiver<String>,
    /// Shared WhatsApp state — use `client()` after connected for test messages.
    pub wa_state: Arc<crate::channels::whatsapp::WhatsAppState>,
}

/// Subscribe to QR / connected events from the running WhatsApp agent bot.
/// Does NOT create a new bot — the ChannelManager's agent is the only instance.
/// If `wipe_session` is true, deletes session.db first so the agent shows a fresh QR.
pub fn subscribe_whatsapp_pairing(
    wa_state: &Arc<crate::channels::whatsapp::WhatsAppState>,
    wipe_session: bool,
) -> WhatsAppConnectHandle {
    if wipe_session {
        let wa_dir = opencrabs_home().join("whatsapp");
        let _ = std::fs::remove_file(wa_dir.join("session.db"));
        let _ = std::fs::remove_file(wa_dir.join("session.db-wal"));
        let _ = std::fs::remove_file(wa_dir.join("session.db-shm"));
    }

    WhatsAppConnectHandle {
        qr_rx: wa_state.subscribe_qr(),
        connected_rx: wa_state.subscribe_connected(),
        error_rx: wa_state.subscribe_error(),
        wa_state: wa_state.clone(),
    }
}

/// Tool that connects WhatsApp by generating a QR code for the user to scan.
pub struct WhatsAppConnectTool {
    progress: Option<ProgressCallback>,
    whatsapp_state: Arc<crate::channels::whatsapp::WhatsAppState>,
}

impl WhatsAppConnectTool {
    pub fn new(
        progress: Option<ProgressCallback>,
        whatsapp_state: Arc<crate::channels::whatsapp::WhatsAppState>,
    ) -> Self {
        Self {
            progress,
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

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        // Use tool-provided phones if given, otherwise fall back to config.
        let tool_phones: Vec<String> = input
            .get("allowed_phones")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Save allowed phones to config if provided
        if !tool_phones.is_empty() {
            let _ = crate::config::Config::write_array(
                "channels.whatsapp",
                "allowed_phones",
                &tool_phones,
            );
        }

        // Wipe session and enable WhatsApp — ChannelManager will (re)start the agent
        let wa_dir = opencrabs_home().join("whatsapp");
        let _ = std::fs::create_dir_all(&wa_dir);
        let _ = std::fs::remove_file(wa_dir.join("session.db"));
        let _ = std::fs::remove_file(wa_dir.join("session.db-wal"));
        let _ = std::fs::remove_file(wa_dir.join("session.db-shm"));
        let _ = crate::config::Config::write_key("channels.whatsapp", "enabled", "true");

        // Subscribe to QR/connected events from the agent bot
        let mut qr_rx = self.whatsapp_state.subscribe_qr();
        let mut connected_rx = self.whatsapp_state.subscribe_connected();

        // Wait for QR code from the agent
        let sid = context.session_id;
        let qr_displayed = tokio::time::timeout(Duration::from_secs(30), qr_rx.recv()).await;

        match qr_displayed {
            Ok(Ok(qr_code)) => {
                if let Some(qr_text) = render_qr_unicode(&qr_code)
                    && let Some(ref cb) = self.progress
                {
                    cb(
                        sid,
                        ProgressEvent::IntermediateText {
                            text: format!(
                                "Scan this QR code with WhatsApp on your phone:\n\n{}",
                                qr_text
                            ),
                            reasoning: None,
                        },
                    );
                }
            }
            Ok(Err(_)) => {
                return Ok(ToolResult::error(
                    "WhatsApp agent not running. Enable WhatsApp in config and restart."
                        .to_string(),
                ));
            }
            Err(_) => {
                // Timeout — check if already connected
                if self.whatsapp_state.is_connected().await {
                    // Already connected, skip QR
                } else {
                    return Ok(ToolResult::error(
                        "Timed out waiting for QR code. Is the WhatsApp agent running?".to_string(),
                    ));
                }
            }
        }

        // Wait for connection (2 minute timeout)
        match tokio::time::timeout(Duration::from_secs(120), connected_rx.recv()).await {
            Ok(Ok(())) => Ok(ToolResult::success(
                "WhatsApp connected successfully! Now listening for messages. \
                 Connection persists across restarts.\n\n\
                 IMPORTANT: Send a welcome message to the user RIGHT NOW \
                 using the `whatsapp_send` tool. Be wildly fun — talk like their \
                 best buddy from childhood. You can curse, be unhinged, hilarious, \
                 and totally informal. Use your personality, SOUL.md, and any context \
                 you have about the user. Make it memorable and uniquely yours. \
                 No generic corporate greetings — surprise them!"
                    .to_string(),
            )),
            _ => Ok(ToolResult::error(
                "QR code expired or connection timed out (2 minutes). \
                 Run the tool again to get a new QR code."
                    .to_string(),
            )),
        }
    }
}
