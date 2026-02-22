//! Channel Factory
//!
//! Shared factory for creating channel agent services at runtime.
//! Used by both static startup (ui.rs) and dynamic connection (whatsapp_connect tool).

use crate::brain::agent::AgentService;
use crate::brain::provider::Provider;
use crate::brain::tools::ToolRegistry;
use crate::config::VoiceConfig;
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
    voice_config: VoiceConfig,
    openai_tts_key: Option<String>,
}

impl ChannelFactory {
    pub fn new(
        provider: Arc<dyn Provider>,
        service_context: ServiceContext,
        shared_brain: String,
        working_directory: PathBuf,
        brain_path: PathBuf,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
        voice_config: VoiceConfig,
        openai_tts_key: Option<String>,
    ) -> Self {
        Self {
            provider,
            service_context,
            shared_brain,
            tool_registry: OnceLock::new(),
            working_directory,
            brain_path,
            shared_session_id,
            voice_config,
            openai_tts_key,
        }
    }

    /// Set the tool registry (call once, after Arc<ToolRegistry> is created).
    pub fn set_tool_registry(&self, registry: Arc<ToolRegistry>) {
        let _ = self.tool_registry.set(registry);
    }

    /// Create a new AgentService configured for channel use (auto-approve, no TUI callbacks).
    pub fn create_agent_service(&self) -> Arc<AgentService> {
        let mut builder = AgentService::new(self.provider.clone(), self.service_context.clone())
            .with_system_brain(self.shared_brain.clone())
            .with_auto_approve_tools(true)
            .with_working_directory(self.working_directory.clone())
            .with_brain_path(self.brain_path.clone());

        if let Some(registry) = self.tool_registry.get() {
            builder = builder.with_tool_registry(registry.clone());
        }

        Arc::new(builder)
    }

    pub fn shared_session_id(&self) -> Arc<Mutex<Option<Uuid>>> {
        self.shared_session_id.clone()
    }

    pub fn service_context(&self) -> ServiceContext {
        self.service_context.clone()
    }

    pub fn voice_config(&self) -> &VoiceConfig {
        &self.voice_config
    }

    pub fn openai_tts_key(&self) -> Option<String> {
        self.openai_tts_key.clone()
    }
}
