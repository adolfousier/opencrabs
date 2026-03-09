//! Channel Factory
//!
//! Shared factory for creating channel agent services at runtime.
//! Used by both static startup (ui.rs) and dynamic connection (whatsapp_connect tool).

use crate::brain::agent::AgentService;
use crate::brain::provider::Provider;
use crate::brain::tools::ToolRegistry;
use crate::config::{Config, VoiceConfig};
use crate::services::ServiceContext;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Factory for creating channel-specific AgentService instances.
///
/// Holds all shared state needed to spin up channel agents (Telegram, WhatsApp, etc.)
/// both at startup and dynamically at runtime via tools.
///
/// The `tool_registry` is set lazily via [`set_tool_registry`] to break the circular
/// dependency between tool registration and factory creation.
pub struct ChannelFactory {
    provider: Arc<dyn Provider>,
    service_context: ServiceContext,
    shared_brain: String,
    tool_registry: OnceLock<Arc<ToolRegistry>>,
    working_directory: PathBuf,
    brain_path: PathBuf,
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    config_rx: tokio::sync::watch::Receiver<Config>,
    session_updated_tx: OnceLock<tokio::sync::mpsc::UnboundedSender<Uuid>>,
}

impl ChannelFactory {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Arc<dyn Provider>,
        service_context: ServiceContext,
        shared_brain: String,
        working_directory: PathBuf,
        brain_path: PathBuf,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        config_rx: tokio::sync::watch::Receiver<Config>,
    ) -> Self {
        Self {
            provider,
            service_context,
            shared_brain,
            tool_registry: OnceLock::new(),
            working_directory,
            brain_path,
            shared_session_id,
            config_rx,
            session_updated_tx: OnceLock::new(),
        }
    }

    /// Wire in the TUI session-updated sender so channel agents trigger live TUI refresh.
    pub fn set_session_updated_tx(&self, tx: tokio::sync::mpsc::UnboundedSender<Uuid>) {
        let _ = self.session_updated_tx.set(tx);
    }

    /// Set the tool registry (call once, after Arc<ToolRegistry> is created).
    pub fn set_tool_registry(&self, registry: Arc<ToolRegistry>) {
        let _ = self.tool_registry.set(registry);
    }

    /// Create a new AgentService configured for channel use.
    ///
    /// Channels that implement their own approval flow (WhatsApp, Telegram, Discord, Slack)
    /// pass an `override_approval_callback` per call — they must NOT have `auto_approve_tools`
    /// set, otherwise the override is ignored. A2A and headless tools that have no interactive
    /// user can set their own auto-approval via session context.
    pub fn create_agent_service(&self) -> Arc<AgentService> {
        let mut builder = AgentService::new(self.provider.clone(), self.service_context.clone())
            .with_system_brain(self.shared_brain.clone())
            .with_working_directory(self.working_directory.clone())
            .with_brain_path(self.brain_path.clone());

        if let Some(registry) = self.tool_registry.get() {
            builder = builder.with_tool_registry(registry.clone());
        }

        if let Some(tx) = self.session_updated_tx.get() {
            builder = builder.with_session_updated_tx(tx.clone());
        }

        Arc::new(builder)
    }

    pub fn shared_session_id(&self) -> Arc<Mutex<Option<Uuid>>> {
        self.shared_session_id.clone()
    }

    pub fn service_context(&self) -> ServiceContext {
        self.service_context.clone()
    }

    /// Get a clone of the config watch receiver for channels to subscribe to.
    pub fn config_rx(&self) -> tokio::sync::watch::Receiver<Config> {
        self.config_rx.clone()
    }

    /// Read the current voice config from the watch channel (always latest).
    pub fn voice_config(&self) -> VoiceConfig {
        self.config_rx.borrow().voice_config()
    }

    /// Read the current OpenAI TTS key from the watch channel (always latest).
    pub fn openai_tts_key(&self) -> Option<String> {
        let cfg = self.config_rx.borrow();
        cfg.providers
            .tts
            .as_ref()
            .and_then(|t| t.openai.as_ref())
            .and_then(|p| p.api_key.clone())
    }
}
