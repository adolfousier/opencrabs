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
    session_updated_tx:
        OnceLock<tokio::sync::mpsc::UnboundedSender<crate::brain::agent::ChannelSessionEvent>>,
    /// Runtime info (model/provider/working dir) used to faithfully rebuild
    /// the channel system brain when a brain file changes (#213). Set once at
    /// startup via [`set_runtime_info`]; absent on the cron daemon path, where
    /// the brain is built with no runtime info anyway.
    runtime_info: OnceLock<crate::brain::prompt_builder::RuntimeInfo>,
    /// Sub-agent manager for compaction persistence (#936). Set once at startup
    /// via [`set_subagent_manager`]; wired into every agent service the factory
    /// creates so compaction can inject running sub-agent IDs into the summary.
    subagent_manager: OnceLock<Arc<crate::brain::tools::subagent::SubAgentManager>>,
    /// Headless factory (#129): agents built by the daemon's cron scheduler
    /// have no live user surface. Stamped into every agent service so
    /// interactive-only tools hard-error (belt-and-braces backstop behind the
    /// registry-level exclusion). Default `false` (interactive channel factory).
    headless: std::sync::atomic::AtomicBool,
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
            runtime_info: OnceLock::new(),
            subagent_manager: OnceLock::new(),
            headless: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Mark this factory headless (#129): every agent service it builds gets
    /// `with_headless(true)` so interactive-only tools hard-error. Used by the
    /// daemon's cron-scheduler factory; the interactive channel factory never
    /// calls this.
    pub fn set_headless(&self, headless: bool) {
        self.headless
            .store(headless, std::sync::atomic::Ordering::Relaxed);
    }

    /// Wire in the runtime info so channel agents rebuild their system brain
    /// faithfully (model/provider/working-dir lines preserved) when a brain
    /// file changes at runtime (#213). Call once at startup.
    pub fn set_runtime_info(&self, info: crate::brain::prompt_builder::RuntimeInfo) {
        let _ = self.runtime_info.set(info);
    }

    /// Wire in the TUI session-updated sender so channel agents trigger live TUI refresh.
    pub fn set_session_updated_tx(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<crate::brain::agent::ChannelSessionEvent>,
    ) {
        let _ = self.session_updated_tx.set(tx);
    }

    /// Set the tool registry (call once, after Arc<ToolRegistry> is created).
    pub fn set_tool_registry(&self, registry: Arc<ToolRegistry>) {
        let _ = self.tool_registry.set(registry);
    }

    /// Set the sub-agent manager for compaction persistence (#936).
    pub fn set_subagent_manager(
        &self,
        manager: Arc<crate::brain::tools::subagent::SubAgentManager>,
    ) {
        let _ = self.subagent_manager.set(manager);
    }

    /// Create a new AgentService configured for channel use.
    ///
    /// Channels that implement their own approval flow (WhatsApp, Telegram, Discord, Slack)
    /// pass an `override_approval_callback` per call — they must NOT have `auto_approve_tools`
    /// set, otherwise the override is ignored. A2A and headless tools that have no interactive
    /// user can set their own auto-approval via session context.
    pub async fn create_agent_service(&self) -> Arc<AgentService> {
        self.create_agent_service_with_queue(None).await
    }

    /// Like [`create_agent_service`](Self::create_agent_service) but wires a
    /// per-session message-queue callback so the tool loop can inject a queued
    /// message between rounds. Telegram uses this to surface a mid-turn reaction
    /// into the running loop (#302 Stage 2); channels without one pass `None`.
    pub async fn create_agent_service_with_queue(
        &self,
        message_queue_callback: Option<crate::brain::agent::MessageQueueCallback>,
    ) -> Arc<AgentService> {
        self.create_agent_service_full(message_queue_callback, None)
            .await
    }

    /// Like [`create_agent_service_with_queue`](Self::create_agent_service_with_queue)
    /// but also wires the background-task enqueue producer (#722) so a channel can
    /// resume a session when a detached long command finishes.
    pub async fn create_agent_service_full(
        &self,
        message_queue_callback: Option<crate::brain::agent::MessageQueueCallback>,
        message_enqueue_callback: Option<crate::brain::agent::service::MessageEnqueueCallback>,
    ) -> Arc<AgentService> {
        let config = self.config_rx.borrow().clone();
        let mut builder =
            AgentService::new(self.provider.clone(), self.service_context.clone(), &config)
                .await
                .with_system_brain(self.shared_brain.clone())
                // Live brain rebuild (#213): channel agents pick up brain-file
                // edits on their next turn. Channels use the core brain; the
                // lazy-tools suffix mirrors the startup assembly. Seeded from
                // `shared_brain` so nothing is re-read until a file changes.
                .with_brain_rebuild(
                    crate::brain::prompt_builder::BrainLoader::new(self.brain_path.clone()),
                    self.runtime_info.get().cloned(),
                    true,
                    config.agent.lazy_tools,
                )
                .with_working_directory(self.working_directory.clone())
                .with_brain_path(self.brain_path.clone());

        if let Some(registry) = self.tool_registry.get() {
            builder = builder.with_tool_registry(registry.clone());
        }

        if self.headless.load(std::sync::atomic::Ordering::Relaxed) {
            builder = builder.with_headless(true);
        }

        if let Some(tx) = self.session_updated_tx.get() {
            builder = builder.with_session_updated_tx(tx.clone());
        }

        if let Some(mgr) = self.subagent_manager.get() {
            builder = builder.with_subagent_manager(mgr.clone());
        }

        if message_queue_callback.is_some() {
            builder = builder.with_message_queue_callback(message_queue_callback);
        }

        if message_enqueue_callback.is_some() {
            builder = builder.with_message_enqueue_callback(message_enqueue_callback);
        }

        Arc::new(builder)
    }

    pub fn shared_session_id(&self) -> Arc<Mutex<Option<Uuid>>> {
        self.shared_session_id.clone()
    }

    /// The shared tool registry, if it has been wired in yet. Used by the cron
    /// scheduler to give a foreign-profile agent the same tools (the registry is
    /// profile-agnostic dispatch; per-call config drives profile behaviour).
    pub fn tool_registry(&self) -> Option<Arc<ToolRegistry>> {
        self.tool_registry.get().cloned()
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
