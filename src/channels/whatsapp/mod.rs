//! WhatsApp Integration
//!
//! Runs a WhatsApp Web client alongside the TUI, forwarding messages from
//! allowlisted phone numbers to the AgentService and replying with responses.

mod agent;
pub(crate) mod handler;
pub(crate) mod store;

pub use agent::WhatsAppAgent;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use whatsapp_rust::client::Client;

/// Approval choices mirroring the TUI's Yes / Always (session) / YOLO (permanent) / No.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaApproval {
    /// Approve this tool call once.
    Yes,
    /// Approve this and all future tool calls for the rest of the session.
    Always,
    /// Approve permanently (survives restarts).
    Yolo,
    /// Deny this tool call.
    No,
}

/// Shared WhatsApp client state for proactive messaging.
///
/// Set when the bot connects (either via static agent or whatsapp_connect tool).
/// Read by the `whatsapp_send` tool to send messages on demand.
pub struct WhatsAppState {
    client: Mutex<Option<Arc<Client>>>,
    /// Owner's JID (phone@s.whatsapp.net) — first in allowed_phones list
    owner_jid: Mutex<Option<String>>,
    /// Pending tool approvals: phone → oneshot sender of WaApproval.
    /// When a tool approval is in flight, the next message from that phone
    /// (text or button tap) is interpreted as Yes/Always/No instead of
    /// being routed to the agent.
    pub pending_approvals: Mutex<HashMap<String, tokio::sync::oneshot::Sender<WaApproval>>>,
    /// Per-session cancel tokens for aborting in-flight agent tasks via /stop
    cancel_tokens: Mutex<HashMap<Uuid, CancellationToken>>,
    /// Broadcast channel for QR codes — onboarding subscribes to this.
    qr_tx: tokio::sync::broadcast::Sender<String>,
    /// Broadcast channel for connection events — onboarding subscribes to this.
    connected_tx: tokio::sync::broadcast::Sender<()>,
    /// Broadcast channel for error events — onboarding subscribes to this.
    error_tx: tokio::sync::broadcast::Sender<String>,
}

impl Default for WhatsAppState {
    fn default() -> Self {
        Self::new()
    }
}

impl WhatsAppState {
    pub fn new() -> Self {
        let (qr_tx, _) = tokio::sync::broadcast::channel(8);
        let (connected_tx, _) = tokio::sync::broadcast::channel(4);
        let (error_tx, _) = tokio::sync::broadcast::channel(4);
        Self {
            client: Mutex::new(None),
            owner_jid: Mutex::new(None),
            pending_approvals: Mutex::new(HashMap::new()),
            cancel_tokens: Mutex::new(HashMap::new()),
            qr_tx,
            connected_tx,
            error_tx,
        }
    }

    /// Register a pending approval for a phone number.
    pub async fn register_pending_approval(
        &self,
        phone: String,
        tx: tokio::sync::oneshot::Sender<WaApproval>,
    ) {
        self.pending_approvals.lock().await.insert(phone, tx);
    }

    /// Resolve a pending approval (called when user replies or taps a button).
    /// Returns `Some(choice)` if there was a pending approval, `None` otherwise.
    pub async fn resolve_pending_approval(
        &self,
        phone: &str,
        choice: WaApproval,
    ) -> Option<WaApproval> {
        if let Some(tx) = self.pending_approvals.lock().await.remove(phone) {
            let _ = tx.send(choice);
            Some(choice)
        } else {
            None
        }
    }

    /// Broadcast a QR code to any subscribed onboarding UI.
    pub fn broadcast_qr(&self, code: &str) {
        let _ = self.qr_tx.send(code.to_string());
    }

    /// Broadcast a connected event to any subscribed onboarding UI.
    pub fn broadcast_connected(&self) {
        let _ = self.connected_tx.send(());
    }

    /// Subscribe to QR code events (used by onboarding).
    pub fn subscribe_qr(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.qr_tx.subscribe()
    }

    /// Subscribe to connection events (used by onboarding).
    pub fn subscribe_connected(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.connected_tx.subscribe()
    }

    /// Broadcast an error to any subscribed onboarding UI.
    pub fn broadcast_error(&self, msg: &str) {
        let _ = self.error_tx.send(msg.to_string());
    }

    /// Subscribe to error events (used by onboarding).
    pub fn subscribe_error(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.error_tx.subscribe()
    }

    /// Store the connected client and owner JID.
    pub async fn set_connected(&self, client: Arc<Client>, owner_jid: Option<String>) {
        *self.client.lock().await = Some(client);
        if let Some(jid) = owner_jid {
            *self.owner_jid.lock().await = Some(jid);
        }
        self.broadcast_connected();
    }

    /// Get a clone of the connected client, if any.
    pub async fn client(&self) -> Option<Arc<Client>> {
        self.client.lock().await.clone()
    }

    /// Get the owner's JID for proactive messaging.
    pub async fn owner_jid(&self) -> Option<String> {
        self.owner_jid.lock().await.clone()
    }

    /// Check if WhatsApp is currently connected.
    pub async fn is_connected(&self) -> bool {
        self.client.lock().await.is_some()
    }

    /// Store a cancel token for a session (before starting agent call).
    pub async fn store_cancel_token(&self, session_id: Uuid, token: CancellationToken) {
        self.cancel_tokens.lock().await.insert(session_id, token);
    }

    /// Cancel and remove the token for a session. Returns true if a token existed.
    pub async fn cancel_session(&self, session_id: Uuid) -> bool {
        if let Some(token) = self.cancel_tokens.lock().await.remove(&session_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Remove the cancel token after the agent call completes (cleanup).
    pub async fn remove_cancel_token(&self, session_id: Uuid) {
        self.cancel_tokens.lock().await.remove(&session_id);
    }
}
