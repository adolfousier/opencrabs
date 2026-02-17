//! WhatsApp Send Tool
//!
//! Agent-callable tool for proactively sending WhatsApp messages.
//! Uses the shared `WhatsAppState` to access the connected client.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::whatsapp::WhatsAppState;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool that sends a WhatsApp message to the owner or a specific phone number.
pub struct WhatsAppSendTool {
    whatsapp_state: Arc<WhatsAppState>,
}

impl WhatsAppSendTool {
    pub fn new(whatsapp_state: Arc<WhatsAppState>) -> Self {
        Self { whatsapp_state }
    }
}

#[async_trait]
impl Tool for WhatsAppSendTool {
    fn name(&self) -> &str {
        "whatsapp_send"
    }

    fn description(&self) -> &str {
        "Send a WhatsApp message to the user. Use this to proactively reach out, share updates, \
         or notify the user about completed tasks. If no phone number is specified, the message \
         is sent to the owner (primary user). Requires WhatsApp to be connected first."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message text to send"
                },
                "phone": {
                    "type": "string",
                    "description": "Phone number to send to (E.164 format, e.g. '+15551234567'). Omit to message the owner."
                }
            },
            "required": ["message"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let message = match input.get("message").and_then(|v| v.as_str()) {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing or empty 'message' parameter.".to_string(),
                ));
            }
        };

        let client = match self.whatsapp_state.client().await {
            Some(c) => c,
            None => {
                return Ok(ToolResult::error(
                    "WhatsApp is not connected. Ask the user to connect WhatsApp first \
                     (use the whatsapp_connect tool)."
                        .to_string(),
                ));
            }
        };

        // Resolve target JID: explicit phone or owner
        let jid_str = if let Some(phone) = input.get("phone").and_then(|v| v.as_str()) {
            let digits = phone.trim_start_matches('+');
            format!("{}@s.whatsapp.net", digits)
        } else {
            match self.whatsapp_state.owner_jid().await {
                Some(jid) => jid,
                None => {
                    return Ok(ToolResult::error(
                        "No owner phone number configured and no 'phone' parameter provided. \
                         Specify a phone number to send to."
                            .to_string(),
                    ));
                }
            }
        };

        let jid: wacore_binary::jid::Jid = match jid_str.parse() {
            Ok(j) => j,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Invalid phone number format: {}",
                    e
                )));
            }
        };

        // Prepend agent header and split long messages
        let tagged = format!("{}\n\n{}", crate::whatsapp::handler::MSG_HEADER, message);
        let chunks = crate::whatsapp::handler::split_message(&tagged, 4000);
        for chunk in chunks {
            let wa_msg = waproto::whatsapp::Message {
                conversation: Some(chunk.to_string()),
                ..Default::default()
            };
            if let Err(e) = client.send_message(jid.clone(), wa_msg).await {
                return Ok(ToolResult::error(format!(
                    "Failed to send WhatsApp message: {}",
                    e
                )));
            }
        }

        Ok(ToolResult::success(format!(
            "Message sent to {} via WhatsApp.",
            jid_str
        )))
    }
}
