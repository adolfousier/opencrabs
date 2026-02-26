//! TUI Application State
//!
//! Core state management for the terminal user interface.

use super::events::{
    AppMode, EventHandler, SudoPasswordRequest, SudoPasswordResponse, ToolApprovalRequest,
    ToolApprovalResponse, TuiEvent,
};
use super::onboarding::OnboardingWizard;
use super::plan::PlanDocument;
use super::prompt_analyzer::PromptAnalyzer;
use crate::brain::agent::AgentService;
use crate::brain::{BrainLoader, CommandLoader, SelfUpdater, UserCommand};
use crate::db::models::{Message, Session};
use crate::services::{MessageService, PlanService, ServiceContext, SessionService};
use anyhow::Result;
use ratatui::text::Line;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Slash command definition
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
}

/// Available slash commands for autocomplete
pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/help",
        description: "Show available commands",
    },
    SlashCommand {
        name: "/models",
        description: "Switch model",
    },
    SlashCommand {
        name: "/usage",
        description: "Session usage stats",
    },
    SlashCommand {
        name: "/onboard",
        description: "Run setup wizard",
    },
    SlashCommand {
        name: "/sessions",
        description: "List all sessions",
    },
    SlashCommand {
        name: "/approve",
        description: "Tool approval policy",
    },
    SlashCommand {
        name: "/compact",
        description: "Compact context now",
    },
    SlashCommand {
        name: "/rebuild",
        description: "Build & restart from source",
    },
    SlashCommand {
        name: "/whisper",
        description: "Speak anywhere, paste to clipboard",
    },
    SlashCommand {
        name: "/cd",
        description: "Change working directory",
    },
];

/// Approval option selected by the user
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalOption {
    AllowOnce,
    AllowForSession,
    AllowAlways,
}

/// State of an inline approval request
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalState {
    Pending,
    Approved(ApprovalOption),
    Denied(String),
}

/// Data for an inline tool approval request embedded in a DisplayMessage
#[derive(Debug, Clone)]
pub struct ApprovalData {
    pub tool_name: String,
    pub tool_description: String,
    pub tool_input: Value,
    pub capabilities: Vec<String>,
    pub request_id: Uuid,
    pub response_tx: mpsc::UnboundedSender<ToolApprovalResponse>,
    pub requested_at: std::time::Instant,
    pub state: ApprovalState,
    /// 0-2, arrow key navigation
    pub selected_option: usize,
    /// V key toggle
    pub show_details: bool,
}

/// State for the /approve policy selector menu
#[derive(Debug, Clone, PartialEq)]
pub enum ApproveMenuState {
    Pending,
    Selected(usize),
}

/// Data for the /approve inline menu
#[derive(Debug, Clone)]
pub struct ApproveMenu {
    /// 0-2
    pub selected_option: usize,
    pub state: ApproveMenuState,
}

/// State of an inline plan approval
#[derive(Debug, Clone, PartialEq)]
pub enum PlanApprovalState {
    Pending,
    Approved,
    Rejected,
    RevisionRequested,
}

/// Data for an inline plan approval selector embedded in a DisplayMessage
#[derive(Debug, Clone)]
pub struct PlanApprovalData {
    pub plan_title: String,
    pub task_count: usize,
    pub task_summaries: Vec<String>,
    pub state: PlanApprovalState,
    /// 0=Approve, 1=Reject, 2=Request Changes, 3=View Plan
    pub selected_option: usize,
    /// Toggle task list
    pub show_details: bool,
}

/// An image file attached to the input (detected from pasted paths)
#[derive(Debug, Clone)]
pub struct ImageAttachment {
    /// Display name (file name)
    pub name: String,
    /// Full path to the image
    pub path: String,
}

/// Image file extensions for auto-detection
pub(crate) const IMAGE_EXTENSIONS: &[&str] =
    &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"];

/// A single tool call entry within a grouped display
#[derive(Debug, Clone)]
pub struct ToolCallEntry {
    pub description: String,
    pub success: bool,
    pub details: Option<String>,
}

/// A group of tool calls displayed as a collapsible bullet
#[derive(Debug, Clone)]
pub struct ToolCallGroup {
    pub calls: Vec<ToolCallEntry>,
    pub expanded: bool,
}

/// Display message for UI rendering
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub token_count: Option<i32>,
    pub cost: Option<f64>,
    pub approval: Option<ApprovalData>,
    pub approve_menu: Option<ApproveMenu>,
    /// Collapsible details (tool output, etc.) — shown when expanded
    pub details: Option<String>,
    /// Whether details are currently expanded
    pub expanded: bool,
    /// Grouped tool calls (for role == "tool_group")
    pub tool_group: Option<ToolCallGroup>,
    /// Inline plan approval selector
    pub plan_approval: Option<PlanApprovalData>,
}

impl From<Message> for DisplayMessage {
    fn from(msg: Message) -> Self {
        Self {
            id: msg.id,
            role: msg.role,
            content: msg.content,
            timestamp: msg.created_at,
            token_count: msg.token_count,
            cost: msg.cost,
            approval: None,
            approve_menu: None,
            details: None,
            expanded: false,
            tool_group: None,
            plan_approval: None,
        }
    }
}

/// Main application state
pub struct App {
    /// Core state
    pub current_session: Option<Session>,
    pub messages: Vec<DisplayMessage>,
    pub sessions: Vec<Session>,

    /// UI state
    pub mode: AppMode,
    pub input_buffer: String,
    /// Cursor position within input_buffer (byte offset, always on a char boundary)
    pub cursor_position: usize,
    /// Images attached to the current input (auto-detected from pasted paths)
    pub attachments: Vec<ImageAttachment>,
    pub scroll_offset: usize,
    /// When true, new streaming content auto-scrolls to bottom.
    /// Set to false when user scrolls up; re-enabled when they scroll back to bottom or send a message.
    pub auto_scroll: bool,
    pub selected_session_index: usize,
    pub should_quit: bool,

    /// Streaming state
    pub is_processing: bool,
    pub processing_started_at: Option<std::time::Instant>,
    pub streaming_response: Option<String>,
    /// Reasoning/thinking content from providers like MiniMax (display-only, cleared on complete)
    pub streaming_reasoning: Option<String>,
    pub error_message: Option<String>,
    /// Set to true when IntermediateText arrives during the current response cycle.
    /// Reset to false at the start of each new send_message call.
    /// Used in complete_response to avoid double-adding the assistant message.
    pub(crate) intermediate_text_received: bool,

    /// Animation state
    pub animation_frame: usize,

    /// Splash screen state
    pub(crate) splash_shown_at: Option<std::time::Instant>,

    /// Escape confirmation state (double-press to clear)
    pub(crate) escape_pending_at: Option<std::time::Instant>,

    /// Ctrl+C confirmation state (first clears input, second quits)
    pub(crate) ctrl_c_pending_at: Option<std::time::Instant>,

    /// Help/Settings scroll offset
    pub help_scroll_offset: usize,

    /// Model name for display (from provider default)
    pub default_model_name: String,

    /// Approval policy state
    pub approval_auto_session: bool,
    pub approval_auto_always: bool,

    /// Plan mode state
    pub current_plan: Option<PlanDocument>,
    pub plan_scroll_offset: usize,
    pub selected_task_index: Option<usize>,
    pub executing_plan: bool,

    /// File picker state
    pub file_picker_files: Vec<std::path::PathBuf>,
    pub file_picker_selected: usize,
    pub file_picker_scroll_offset: usize,
    pub file_picker_current_dir: std::path::PathBuf,

    /// Slash autocomplete state
    pub slash_suggestions_active: bool,
    /// Indices into SLASH_COMMANDS
    pub slash_filtered: Vec<usize>,
    pub slash_selected_index: usize,

    /// Session rename state
    pub session_renaming: bool,
    pub session_rename_buffer: String,

    /// Model selector state (mirrors onboarding ProviderAuth)
    pub model_selector_models: Vec<String>,
    pub model_selector_selected: usize,
    pub model_selector_showing_providers: bool,
    pub model_selector_provider_selected: usize,
    pub model_selector_api_key: String,
    pub model_selector_base_url: String,
    pub model_selector_custom_model: String,
    /// Focused field: 0=provider, 1=api_key, 2=model
    pub model_selector_focused_field: usize,
    pub model_selector_filter: String,

    /// Input history (arrow up/down to cycle through past messages)
    pub(crate) input_history: Vec<String>,
    /// None = not browsing, Some(i) = viewing history[i]
    pub(crate) input_history_index: Option<usize>,
    /// Saves current input when entering history
    pub(crate) input_history_stash: String,

    /// Working directory
    pub working_directory: std::path::PathBuf,

    /// Context hints queued by UI actions (e.g. /cd, @ file picker).
    /// Drained and prepended to the next user message so the LLM knows
    /// what just happened without the user having to explain.
    pub pending_context: Vec<String>,

    /// Brain state
    pub brain_path: PathBuf,
    pub user_commands: Vec<UserCommand>,

    /// Onboarding wizard state
    pub onboarding: Option<OnboardingWizard>,
    pub force_onboard: bool,

    /// Cancellation token for aborting in-progress requests
    pub(crate) cancel_token: Option<CancellationToken>,

    /// Queued message — shared with agent so it can be injected between tool calls
    pub(crate) message_queue: Arc<tokio::sync::Mutex<Option<String>>>,

    /// Shared session ID — channels (Telegram, WhatsApp) read this to use the same session
    pub(crate) shared_session_id: Arc<tokio::sync::Mutex<Option<Uuid>>>,

    /// Context window tracking
    pub context_max_tokens: u32,
    pub last_input_tokens: Option<u32>,

    /// Active tool call group (during processing)
    pub active_tool_group: Option<ToolCallGroup>,

    /// Self-update state
    pub rebuild_status: Option<String>,

    /// Session to resume after restart (set via --session CLI arg)
    pub resume_session_id: Option<Uuid>,

    /// Cache of rendered lines per message to avoid re-parsing markdown every frame.
    /// Key: (message_id, content_width). Invalidated on terminal resize.
    pub render_cache: HashMap<(Uuid, u16), Vec<Line<'static>>>,

    /// History paging — how many DB messages are hidden above the current view
    pub hidden_older_messages: usize,
    pub oldest_displayed_sequence: i32,
    pub display_token_count: usize,

    /// Pending sudo password request (shown as inline dialog)
    pub sudo_pending: Option<SudoPasswordRequest>,
    /// Raw password text being typed (never displayed, only dots)
    pub sudo_input: String,

    /// Services
    pub(crate) agent_service: Arc<AgentService>,
    pub(crate) session_service: SessionService,
    pub(crate) message_service: MessageService,
    pub(crate) plan_service: PlanService,

    /// Events
    pub(crate) event_handler: EventHandler,

    /// Prompt analyzer
    pub(crate) prompt_analyzer: PromptAnalyzer,
}

impl App {
    /// Create a new app instance
    pub fn new(agent_service: Arc<AgentService>, context: ServiceContext) -> Self {
        let brain_path = BrainLoader::resolve_path();
        let command_loader = CommandLoader::from_brain_path(&brain_path);
        let user_commands = command_loader.load();

        // Load persisted approval policy from config.toml
        let (approval_auto_session, approval_auto_always) = match crate::config::Config::load() {
            Ok(cfg) => match cfg.agent.approval_policy.as_str() {
                "auto-session" => (true, false),
                "auto-always" => (false, true),
                _ => (false, false),
            },
            Err(_) => (false, false),
        };

        Self {
            current_session: None,
            messages: Vec::new(),
            sessions: Vec::new(),
            mode: AppMode::Splash,
            input_buffer: String::new(),
            cursor_position: 0,
            attachments: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
            selected_session_index: 0,
            should_quit: false,
            is_processing: false,
            processing_started_at: None,
            streaming_response: None,
            streaming_reasoning: None,
            error_message: None,
            intermediate_text_received: false,
            animation_frame: 0,
            splash_shown_at: Some(std::time::Instant::now()),
            escape_pending_at: None,
            ctrl_c_pending_at: None,
            help_scroll_offset: 0,
            approval_auto_session,
            approval_auto_always,
            current_plan: None,
            plan_scroll_offset: 0,
            selected_task_index: None,
            executing_plan: false,
            file_picker_files: Vec::new(),
            file_picker_selected: 0,
            file_picker_scroll_offset: 0,
            file_picker_current_dir: std::env::current_dir().unwrap_or_default(),
            slash_suggestions_active: false,
            slash_filtered: Vec::new(),
            slash_selected_index: 0,
            session_renaming: false,
            session_rename_buffer: String::new(),
            model_selector_models: Vec::new(),
            model_selector_selected: 0,
            model_selector_showing_providers: false,
            model_selector_provider_selected: 0,
            model_selector_api_key: String::new(),
            model_selector_base_url: String::new(),
            model_selector_custom_model: String::new(),
            model_selector_focused_field: 0,
            model_selector_filter: String::new(),
            input_history: Self::load_history(),
            input_history_index: None,
            input_history_stash: String::new(),
            working_directory: std::env::current_dir().unwrap_or_default(),
            pending_context: Vec::new(),
            brain_path,
            user_commands,
            onboarding: None,
            force_onboard: false,
            cancel_token: None,
            message_queue: Arc::new(tokio::sync::Mutex::new(None)),
            shared_session_id: Arc::new(tokio::sync::Mutex::new(None)),
            default_model_name: agent_service.provider_model().to_string(),
            context_max_tokens: agent_service
                .context_window_for_model(agent_service.provider_model()),
            last_input_tokens: None,
            active_tool_group: None,
            rebuild_status: None,
            resume_session_id: None,
            render_cache: HashMap::new(),
            hidden_older_messages: 0,
            oldest_displayed_sequence: 0,
            display_token_count: 0,
            sudo_pending: None,
            sudo_input: String::new(),
            session_service: SessionService::new(context.clone()),
            message_service: MessageService::new(context.clone()),
            plan_service: PlanService::new(context),
            agent_service,
            event_handler: EventHandler::new(),
            prompt_analyzer: PromptAnalyzer::new(),
        }
    }

    /// Get the provider name
    pub fn provider_name(&self) -> &str {
        self.agent_service.provider_name()
    }

    /// Get the provider model
    pub fn provider_model(&self) -> &str {
        self.agent_service.provider_model()
    }

    /// Get the shared session ID handle (for channels like Telegram/WhatsApp)
    pub fn shared_session_id(&self) -> Arc<tokio::sync::Mutex<Option<Uuid>>> {
        self.shared_session_id.clone()
    }

    /// Initialize the app by loading or creating a session
    pub async fn initialize(&mut self) -> Result<()> {
        // Resume a specific session (e.g. after /rebuild restart) or load the most recent
        if let Some(session_id) = self.resume_session_id.take() {
            self.load_session(session_id).await?;
            // Skip splash — go straight to chat
            self.mode = AppMode::Chat;
            self.splash_shown_at = None;
            // Send a hidden wake-up message to the agent (not shown in UI)
            self.is_processing = true;
            self.processing_started_at = Some(std::time::Instant::now());
            let agent_service = self.agent_service.clone();
            let event_sender = self.event_sender();
            let token = CancellationToken::new();
            self.cancel_token = Some(token.clone());
            tokio::spawn(async move {
                let wake_up = "[SYSTEM: You just rebuilt yourself from source and restarted \
                    via exec(). Greet the user, confirm the restart succeeded, and continue \
                    where you left off.]";
                match agent_service
                    .send_message_with_tools_and_mode(
                        session_id,
                        wake_up.to_string(),
                        None,
                        false,
                        Some(token),
                    )
                    .await
                {
                    Ok(response) => {
                        let _ = event_sender.send(TuiEvent::ResponseComplete(response));
                    }
                    Err(e) => {
                        let _ = event_sender.send(TuiEvent::Error(e.to_string()));
                    }
                }
            });
        } else if let Some(session) = self.session_service.get_most_recent_session().await? {
            self.load_session(session.id).await?;
        } else {
            // Create a new session if none exists
            self.create_new_session().await?;
        }

        // Load sessions list
        self.load_sessions().await?;

        Ok(())
    }

    /// Get event handler
    pub fn event_handler(&self) -> &EventHandler {
        &self.event_handler
    }

    /// Get mutable event handler
    pub fn event_handler_mut(&mut self) -> &mut EventHandler {
        &mut self.event_handler
    }

    /// Get event sender
    pub fn event_sender(&self) -> tokio::sync::mpsc::UnboundedSender<TuiEvent> {
        self.event_handler.sender()
    }

    /// Set agent service (used to inject configured agent after app creation)
    pub fn set_agent_service(&mut self, agent_service: Arc<AgentService>) {
        self.default_model_name = agent_service.provider_model().to_string();
        self.agent_service = agent_service;
    }

    /// Rebuild agent service with a new provider
    pub(crate) async fn rebuild_agent_service(&mut self) -> Result<()> {
        use crate::brain::provider::create_provider;

        // Load config - API keys are stored in keys.toml and merged with config
        let config = crate::config::Config::load()
            .map_err(|e| anyhow::anyhow!("Failed to load config: {}", e))?;

        // Check all providers dynamically - log enabled providers for debugging
        let enabled_providers: Vec<&str> = vec![
            config
                .providers
                .anthropic
                .as_ref()
                .filter(|p| p.enabled)
                .map(|_| "anthropic"),
            config
                .providers
                .openai
                .as_ref()
                .filter(|p| p.enabled)
                .map(|_| "openai"),
            config
                .providers
                .gemini
                .as_ref()
                .filter(|p| p.enabled)
                .map(|_| "gemini"),
            config
                .providers
                .openrouter
                .as_ref()
                .filter(|p| p.enabled)
                .map(|_| "openrouter"),
            config
                .providers
                .minimax
                .as_ref()
                .filter(|p| p.enabled)
                .map(|_| "minimax"),
            config.providers.active_custom().map(|_| "custom"),
        ]
        .into_iter()
        .flatten()
        .collect();

        tracing::debug!(
            "rebuild_agent_service: enabled_providers = {:?}",
            enabled_providers
        );

        // Create new provider from config
        let provider = create_provider(&config)
            .map_err(|e| anyhow::anyhow!("Failed to create provider: {}", e))?;

        // Get existing context from current agent service
        let context = self.agent_service.context().clone();

        // Get existing tool registry from current agent service
        let tool_registry = self.agent_service.tool_registry().clone();

        // Get existing system brain from current agent service
        let system_brain = self.agent_service.system_brain().cloned();

        // Get event sender for approval callback
        let event_sender = self.event_sender();

        // Create approval callback that sends requests to TUI
        let approval_callback: crate::brain::agent::ApprovalCallback = Arc::new(move |tool_info| {
            let sender = event_sender.clone();
            Box::pin(async move {
                use crate::tui::events::{ToolApprovalRequest, TuiEvent};
                use tokio::sync::mpsc;

                let (response_tx, mut response_rx) = mpsc::unbounded_channel();

                let request = ToolApprovalRequest {
                    request_id: uuid::Uuid::new_v4(),
                    tool_name: tool_info.tool_name,
                    tool_description: tool_info.tool_description,
                    tool_input: tool_info.tool_input,
                    capabilities: tool_info.capabilities,
                    response_tx,
                    requested_at: std::time::Instant::now(),
                };

                sender
                    .send(TuiEvent::ToolApprovalRequested(request))
                    .map_err(|e| {
                        crate::brain::agent::AgentError::Internal(format!(
                            "Failed to send approval request: {}",
                            e
                        ))
                    })?;

                let response =
                    tokio::time::timeout(std::time::Duration::from_secs(120), response_rx.recv())
                        .await
                        .map_err(|_| {
                            crate::brain::agent::AgentError::Internal(
                                "Approval request timed out".to_string(),
                            )
                        })?
                        .ok_or_else(|| {
                            crate::brain::agent::AgentError::Internal(
                                "Approval channel closed".to_string(),
                            )
                        })?;

                Ok(response.approved)
            })
        });

        // Preserve existing callbacks from the current agent service
        let progress_callback = self.agent_service.progress_callback().clone();
        let message_queue_callback = self.agent_service.message_queue_callback().clone();
        let sudo_callback = self.agent_service.sudo_callback().clone();
        let working_dir = self
            .agent_service
            .working_directory()
            .read()
            .expect("working_directory lock poisoned")
            .clone();
        let brain_path = self.agent_service.brain_path().clone();

        // Create new agent service with new provider — preserve ALL callbacks
        let mut new_agent_service = AgentService::new(provider, context)
            .with_tool_registry(tool_registry)
            .with_approval_callback(Some(approval_callback))
            .with_progress_callback(progress_callback)
            .with_message_queue_callback(message_queue_callback)
            .with_sudo_callback(sudo_callback)
            .with_working_directory(working_dir);

        if let Some(bp) = brain_path {
            new_agent_service = new_agent_service.with_brain_path(bp);
        }

        // Add system brain if it exists
        if let Some(brain) = system_brain {
            new_agent_service = new_agent_service.with_system_brain(brain);
        }

        let new_agent_service = Arc::new(new_agent_service);

        // Update app state
        self.default_model_name = new_agent_service.provider_model().to_string();
        self.agent_service = new_agent_service;

        Ok(())
    }

    /// Get the agent service
    pub fn agent_service(&self) -> &Arc<AgentService> {
        &self.agent_service
    }

    /// Receive next event (blocks until available)
    pub async fn next_event(&mut self) -> Option<TuiEvent> {
        self.event_handler.next().await
    }

    /// Try to receive next event without blocking (returns None if queue is empty)
    pub fn try_next_event(&mut self) -> Option<TuiEvent> {
        self.event_handler.try_next()
    }

    /// Handle an event
    pub async fn handle_event(&mut self, event: TuiEvent) -> Result<()> {
        match event {
            TuiEvent::Key(key_event) => {
                self.handle_key_event(key_event).await?;
            }
            TuiEvent::MouseScroll(direction) => {
                if self.mode == AppMode::Chat {
                    if direction > 0 {
                        // Scrolling up — disable auto-scroll
                        self.scroll_offset = self.scroll_offset.saturating_add(3);
                        self.auto_scroll = false;
                    } else {
                        self.scroll_offset = self.scroll_offset.saturating_sub(3);
                        // Re-enable auto-scroll when back at bottom
                        if self.scroll_offset == 0 {
                            self.auto_scroll = true;
                        }
                    }
                }
            }
            TuiEvent::Paste(text) => {
                // Handle paste events in Chat mode or Onboarding mode
                if self.mode == AppMode::Chat {
                    // Check if pasted text contains image paths — extract as attachments
                    let (clean_text, new_attachments) = Self::extract_image_paths(&text);
                    if !new_attachments.is_empty() {
                        self.attachments.extend(new_attachments);
                        if !clean_text.trim().is_empty() {
                            self.input_buffer
                                .insert_str(self.cursor_position, &clean_text);
                            self.cursor_position += clean_text.len();
                        }
                    } else {
                        self.input_buffer.insert_str(self.cursor_position, &text);
                        self.cursor_position += text.len();
                    }
                    self.update_slash_suggestions();
                } else if self.mode == AppMode::Onboarding {
                    // Handle paste in onboarding wizard (for API keys, etc.)
                    if let Some(ref mut wizard) = self.onboarding {
                        wizard.handle_paste(&text);
                        // Trigger model fetch if provider supports it and key was just pasted
                        if wizard.supports_model_fetch() && !wizard.api_key_input.is_empty() {
                            let provider_idx = wizard.selected_provider;
                            let api_key = wizard.api_key_input.clone();
                            wizard.models_fetching = true;
                            let sender = self.event_sender();
                            tokio::spawn(async move {
                                let models = super::onboarding::fetch_provider_models(
                                    provider_idx,
                                    Some(&api_key),
                                )
                                .await;
                                let _ = sender.send(TuiEvent::OnboardingModelsFetched(models));
                            });
                        }
                    }
                } else if self.mode == AppMode::ModelSelector {
                    let is_custom = self.model_selector_provider_selected == 5;
                    match (self.model_selector_focused_field, is_custom) {
                        // Non-custom: field 1 = API key
                        (1, false) => {
                            self.model_selector_api_key.push_str(&text);
                            // Trigger model fetch after pasting key
                            let provider_idx = self.model_selector_provider_selected;
                            let api_key = self.model_selector_api_key.clone();
                            let sender = self.event_sender();
                            tokio::spawn(async move {
                                let models = super::onboarding::fetch_provider_models(
                                    provider_idx,
                                    Some(&api_key),
                                )
                                .await;
                                let _ = sender.send(TuiEvent::ModelSelectorModelsFetched(models));
                            });
                        }
                        // Custom: field 1 = base URL, field 2 = API key
                        (1, true) => {
                            self.model_selector_base_url.push_str(&text);
                        }
                        (2, true) => {
                            self.model_selector_api_key.push_str(&text);
                        }
                        _ => {}
                    }
                }
            }
            TuiEvent::MessageSubmitted(content) => {
                self.send_message(content).await?;
            }
            TuiEvent::ResponseChunk(chunk) => {
                self.append_streaming_chunk(chunk);
            }
            TuiEvent::ReasoningChunk(text) => {
                if let Some(ref mut existing) = self.streaming_reasoning {
                    existing.push_str(&text);
                } else {
                    self.streaming_reasoning = Some(text);
                }
                if self.auto_scroll {
                    self.scroll_offset = 0;
                }
            }
            TuiEvent::ResponseComplete(response) => {
                self.complete_response(response).await?;
            }
            TuiEvent::Error(error) => {
                self.show_error(error);
            }
            TuiEvent::SwitchMode(mode) => {
                self.switch_mode(mode).await?;
            }
            TuiEvent::SelectSession(session_id) => {
                self.load_session(session_id).await?;
            }
            TuiEvent::NewSession => {
                self.create_new_session().await?;
            }
            TuiEvent::Quit => {
                self.should_quit = true;
            }
            TuiEvent::Tick => {
                // Update animation frame for spinner
                self.animation_frame = self.animation_frame.wrapping_add(1);

                // Auto-close splash screen after 3 seconds
                if self.mode == AppMode::Splash
                    && let Some(shown_at) = self.splash_shown_at
                    && shown_at.elapsed() >= std::time::Duration::from_secs(3)
                {
                    self.splash_shown_at = None;
                    let is_first = super::onboarding::is_first_time();
                    if self.force_onboard || is_first {
                        self.force_onboard = false;
                        self.onboarding = Some(OnboardingWizard::new());
                        self.switch_mode(AppMode::Onboarding).await?;
                    } else {
                        self.switch_mode(AppMode::Chat).await?;
                    }
                }
            }
            TuiEvent::ToolApprovalRequested(request) => {
                self.handle_approval_requested(request);
            }
            TuiEvent::ToolApprovalResponse(_response) => {
                // Response is sent via channel, auto-scroll if enabled
                if self.auto_scroll {
                    self.scroll_offset = 0;
                }
            }
            TuiEvent::ToolCallStarted {
                tool_name,
                tool_input,
            } => {
                tracing::info!(
                    "[TUI] ToolCallStarted: {} (active_group={}, msg_count={})",
                    tool_name,
                    self.active_tool_group.is_some(),
                    self.messages.len()
                );
                // Show tool call in progress
                let desc = Self::format_tool_description(&tool_name, &tool_input);
                let entry = ToolCallEntry {
                    description: desc,
                    success: true,
                    details: None,
                };
                if let Some(ref mut group) = self.active_tool_group {
                    group.calls.push(entry);
                } else {
                    self.active_tool_group = Some(ToolCallGroup {
                        calls: vec![entry],
                        expanded: false,
                    });
                }
                if self.auto_scroll {
                    self.scroll_offset = 0;
                }
            }
            TuiEvent::IntermediateText(text, reasoning) => {
                tracing::info!(
                    "[TUI] IntermediateText: len={} active_group={} streaming={}",
                    text.len(),
                    self.active_tool_group.is_some(),
                    self.streaming_response.is_some()
                );
                // Reset timer for next thinking phase
                self.processing_started_at = Some(std::time::Instant::now());

                // Capture reasoning from either the event or the streaming accumulator
                let reasoning_details = reasoning.or_else(|| self.streaming_reasoning.take());

                // Clear streaming response - text is now going to be a permanent message
                self.streaming_response = None;
                self.streaming_reasoning = None;
                self.intermediate_text_received = true;

                // Check if there was a queued message that was just processed
                // If so, add it at the VERY END (after all assistant messages and tool calls)
                if let Some(queued_content) = self.message_queue.lock().await.take() {
                    let queued_msg = DisplayMessage {
                        id: Uuid::new_v4(),
                        role: "user".to_string(),
                        content: Self::humanize_image_markers(&queued_content),
                        timestamp: chrono::Utc::now(),
                        token_count: None,
                        cost: None,
                        approval: None,
                        approve_menu: None,
                        details: Some("queued".to_string()), // Mark as queued
                        expanded: false,
                        tool_group: None,
                        plan_approval: None,
                    };
                    // Push at the END so it appears below everything
                    self.messages.push(queued_msg);
                    tracing::info!("[TUI] Added queued message at end of conversation");
                }

                // Flush previous iteration's tool group FIRST, so tools appear
                // before the next iteration's text (matches DB order).
                if let Some(group) = self.active_tool_group.take() {
                    let count = group.calls.len();
                    self.messages.push(DisplayMessage {
                        id: Uuid::new_v4(),
                        role: "tool_group".to_string(),
                        content: format!(
                            "{} tool call{}",
                            count,
                            if count == 1 { "" } else { "s" }
                        ),
                        timestamp: chrono::Utc::now(),
                        token_count: None,
                        cost: None,
                        approval: None,
                        approve_menu: None,
                        details: None,
                        expanded: false,
                        tool_group: Some(group),
                        plan_approval: None,
                    });
                }

                // Then add the new intermediate text as a separate assistant message
                self.messages.push(DisplayMessage {
                    id: Uuid::new_v4(),
                    role: "assistant".to_string(),
                    content: text,
                    timestamp: chrono::Utc::now(),
                    token_count: None,
                    cost: None,
                    approval: None,
                    approve_menu: None,
                    details: reasoning_details,
                    expanded: false,
                    tool_group: None,
                    plan_approval: None,
                });

                if self.auto_scroll {
                    self.scroll_offset = 0;
                }
            }
            TuiEvent::ToolCallCompleted {
                tool_name,
                tool_input,
                success,
                summary,
            } => {
                // Reset timer so "thinking..." counter restarts after each tool call
                self.processing_started_at = Some(std::time::Instant::now());
                let desc = Self::format_tool_description(&tool_name, &tool_input);
                let details = if summary.is_empty() {
                    None
                } else {
                    Some(summary)
                };

                // Update the existing Started entry instead of pushing a duplicate.
                // Match by description — the Started entry has the same desc but no details.
                let updated = if let Some(ref mut group) = self.active_tool_group {
                    if let Some(existing) = group
                        .calls
                        .iter_mut()
                        .rev()
                        .find(|c| c.description == desc && c.details.is_none())
                    {
                        existing.success = success;
                        existing.details = details.clone();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                // Fallback: push as new entry if no matching Started entry found
                if !updated {
                    let entry = ToolCallEntry {
                        description: desc,
                        success,
                        details,
                    };
                    if let Some(ref mut group) = self.active_tool_group {
                        group.calls.push(entry);
                    } else {
                        self.active_tool_group = Some(ToolCallGroup {
                            calls: vec![entry],
                            expanded: false,
                        });
                    }
                }
                if self.auto_scroll {
                    self.scroll_offset = 0;
                }
            }
            TuiEvent::CompactionSummary(summary) => {
                // Agent has summarized history — clear the TUI view for a fresh start.
                self.messages.clear();
                self.render_cache.clear();
                self.hidden_older_messages = 0;
                self.oldest_displayed_sequence = 0;
                self.display_token_count = 0;
                // Reset streaming state so post-compaction tool calls render cleanly
                self.streaming_response = None;
                self.streaming_reasoning = None;
                self.active_tool_group = None;

                // Brief status notice
                self.messages.push(DisplayMessage {
                    id: Uuid::new_v4(),
                    role: "system".to_string(),
                    content: "⚡ Context compacted — summary saved to daily memory log".to_string(),
                    timestamp: chrono::Utc::now(),
                    token_count: None,
                    cost: None,
                    approval: None,
                    approve_menu: None,
                    details: None,
                    expanded: false,
                    tool_group: None,
                    plan_approval: None,
                });

                // Summary rendered as a real assistant message in chat — tool calls follow below
                self.messages.push(DisplayMessage {
                    id: Uuid::new_v4(),
                    role: "assistant".to_string(),
                    content: summary,
                    timestamp: chrono::Utc::now(),
                    token_count: None,
                    cost: None,
                    approval: None,
                    approve_menu: None,
                    details: None,
                    expanded: false,
                    tool_group: None,
                    plan_approval: None,
                });
                // auto_scroll stays true — new messages continue below
            }
            TuiEvent::RestartReady(_status) => {
                self.rebuild_status = None;
                // Auto exec() restart — no prompt, no permission needed
                if let Some(session) = &self.current_session {
                    let session_id = session.id;
                    match SelfUpdater::auto_detect() {
                        Ok(updater) => {
                            if let Err(e) = updater.restart(session_id) {
                                self.show_error(format!("Restart failed: {}", e));
                                self.switch_mode(AppMode::Chat).await?;
                            }
                            // exec() succeeded — this process is replaced, never reached
                        }
                        Err(e) => {
                            self.show_error(format!("Restart failed: {}", e));
                            self.switch_mode(AppMode::Chat).await?;
                        }
                    }
                }
            }
            TuiEvent::ConfigReloaded => {
                // Refresh cached config values and commands
                self.reload_user_commands();
                tracing::info!("Config reloaded — refreshed commands and settings");
            }
            TuiEvent::TokenCountUpdated(count) => {
                self.display_token_count = count;
            }
            TuiEvent::OnboardingModelsFetched(models) => {
                if let Some(ref mut wizard) = self.onboarding {
                    wizard.models_fetching = false;
                    if !models.is_empty() {
                        wizard.fetched_models = models;
                        wizard.selected_model = 0;
                    }
                }
            }
            TuiEvent::ModelSelectorModelsFetched(models) => {
                if self.mode == AppMode::ModelSelector && !models.is_empty() {
                    self.model_selector_models = models;
                    self.model_selector_selected = 0;
                    self.model_selector_filter.clear();
                    // Advance to model selection field
                    let is_custom = self.model_selector_provider_selected == 5;
                    self.model_selector_focused_field = if is_custom { 3 } else { 2 };
                }
            }
            TuiEvent::WhatsAppQrCode(qr_data) => {
                if let Some(ref mut wizard) = self.onboarding {
                    wizard.set_whatsapp_qr(&qr_data);
                }
            }
            TuiEvent::WhatsAppConnected => {
                if let Some(ref mut wizard) = self.onboarding {
                    wizard.set_whatsapp_connected();
                    let _ =
                        crate::config::Config::write_key("channels.whatsapp", "enabled", "true");
                }
            }
            TuiEvent::WhatsAppError(err) => {
                if let Some(ref mut wizard) = self.onboarding {
                    wizard.set_whatsapp_error(err);
                }
            }
            TuiEvent::ChannelTestResult { success, error, .. } => {
                if let Some(ref mut wizard) = self.onboarding {
                    wizard.channel_test_status = if success {
                        super::onboarding::ChannelTestStatus::Success
                    } else {
                        super::onboarding::ChannelTestStatus::Failed(
                            error.unwrap_or_else(|| "Unknown error".to_string()),
                        )
                    };
                }
            }
            TuiEvent::SudoPasswordRequested(request) => {
                self.sudo_pending = Some(request);
                self.sudo_input.clear();
            }
            TuiEvent::SystemMessage(msg) => {
                self.push_system_message(msg);
            }
            TuiEvent::FocusGained | TuiEvent::FocusLost => {
                // Handled by the event loop for tick coalescing
            }
            TuiEvent::Resize(_, _) => {
                // Invalidate render cache on terminal resize (content width changes)
                self.render_cache.clear();
            }
            TuiEvent::AgentProcessing => {
                // Handled by the render loop
            }
        }
        Ok(())
    }

    /// Handle keyboard input
    async fn handle_key_event(&mut self, event: crossterm::event::KeyEvent) -> Result<()> {
        use super::events::keys;
        use crossterm::event::{KeyCode, KeyModifiers};

        // Sudo password dialog intercepts all keys when active
        if self.sudo_pending.is_some() {
            match event.code {
                KeyCode::Enter => {
                    // Submit password
                    if let Some(request) = self.sudo_pending.take() {
                        let password = std::mem::take(&mut self.sudo_input);
                        let _ = request.response_tx.send(SudoPasswordResponse {
                            password: Some(password),
                        });
                    }
                }
                KeyCode::Esc => {
                    // Cancel sudo
                    if let Some(request) = self.sudo_pending.take() {
                        let _ = request
                            .response_tx
                            .send(SudoPasswordResponse { password: None });
                    }
                    self.sudo_input.clear();
                }
                KeyCode::Backspace => {
                    self.sudo_input.pop();
                }
                KeyCode::Char(c) => {
                    self.sudo_input.push(c);
                }
                _ => {}
            }
            return Ok(());
        }

        // DEBUG: Log key events when in Plan mode
        if matches!(self.mode, AppMode::Plan) {
            tracing::debug!(
                "🔑 Plan Mode Key: code={:?}, modifiers={:?}",
                event.code,
                event.modifiers
            );
        }

        // Ctrl+C: first press clears input, second press (within 3s) quits
        if keys::is_quit(&event) {
            if let Some(pending_at) = self.ctrl_c_pending_at
                && pending_at.elapsed() < std::time::Duration::from_secs(3)
            {
                // Second Ctrl+C within window — quit
                // Cancel any running agent task
                if let Some(token) = &self.cancel_token {
                    token.cancel();
                }
                self.should_quit = true;
                // Force exit after 1s in case spawn_blocking tasks are stuck
                tokio::spawn(async {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    std::process::exit(0);
                });
                return Ok(());
            }
            // First Ctrl+C — clear input and show hint
            self.input_buffer.clear();
            self.cursor_position = 0;
            self.slash_suggestions_active = false;
            self.error_message = Some("Press Ctrl+C again to quit".to_string());
            self.ctrl_c_pending_at = Some(std::time::Instant::now());
            return Ok(());
        }

        // Any non-Ctrl+C key resets the quit confirmation
        self.ctrl_c_pending_at = None;

        // Ctrl+Backspace — delete last word
        // Terminals send Ctrl+Backspace as Ctrl+H (KeyCode::Char('h')) so match both
        if (event.code == KeyCode::Backspace || event.code == KeyCode::Char('h'))
            && event.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.delete_last_word();
            return Ok(());
        }

        // Ctrl+Left or Alt+Left — jump to previous word boundary
        if event.code == KeyCode::Left
            && (event.modifiers.contains(KeyModifiers::CONTROL)
                || event.modifiers.contains(KeyModifiers::ALT))
        {
            let before = &self.input_buffer[..self.cursor_position];
            // Skip whitespace, then find start of word
            let trimmed = before.trim_end();
            self.cursor_position = trimmed
                .rfind(char::is_whitespace)
                .map(|pos| pos + 1)
                .unwrap_or(0);
            return Ok(());
        }

        // Ctrl+Right or Alt+Right — jump to next word boundary
        if event.code == KeyCode::Right
            && (event.modifiers.contains(KeyModifiers::CONTROL)
                || event.modifiers.contains(KeyModifiers::ALT))
        {
            let after = &self.input_buffer[self.cursor_position..];
            // Skip current word chars, then skip whitespace
            let word_end = after.find(char::is_whitespace).unwrap_or(after.len());
            let rest = &after[word_end..];
            let space_end = rest
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(rest.len());
            self.cursor_position += word_end + space_end;
            return Ok(());
        }

        if keys::is_new_session(&event) {
            self.create_new_session().await?;
            return Ok(());
        }

        if keys::is_list_sessions(&event) {
            self.switch_mode(AppMode::Sessions).await?;
            return Ok(());
        }

        if keys::is_clear_session(&event) {
            self.clear_session().await?;
            return Ok(());
        }

        if keys::is_toggle_plan(&event) {
            // Toggle between Chat and Plan modes
            match self.mode {
                AppMode::Chat => {
                    // Try to load any plan (not just PendingApproval)
                    self.load_plan_for_viewing().await?;
                    // Only switch if a plan was loaded
                    if self.current_plan.is_some() {
                        self.switch_mode(AppMode::Plan).await?;
                    } else {
                        tracing::info!("No plan available to display");
                        self.error_message =
                            Some("No plan available. Create a plan first.".to_string());
                    }
                }
                AppMode::Plan => self.switch_mode(AppMode::Chat).await?,
                _ => {} // Do nothing in other modes
            }
            return Ok(());
        }

        // Mode-specific handling
        tracing::trace!("Current mode: {:?}", self.mode);
        match self.mode {
            AppMode::Splash => {
                // Check if minimum display time (3 seconds) has elapsed
                if let Some(shown_at) = self.splash_shown_at
                    && shown_at.elapsed() >= std::time::Duration::from_secs(3)
                {
                    self.splash_shown_at = None;
                    // Check if onboarding should be shown
                    let is_first = super::onboarding::is_first_time();
                    tracing::debug!(
                        "[Splash] force_onboard={}, is_first_time={}",
                        self.force_onboard,
                        is_first
                    );
                    if self.force_onboard || is_first {
                        self.force_onboard = false;
                        tracing::info!("[Splash] Starting onboarding wizard");
                        self.onboarding = Some(OnboardingWizard::new());
                        self.switch_mode(AppMode::Onboarding).await?;
                    } else {
                        tracing::debug!("[Splash] Skipping onboarding, going to Chat");
                        self.switch_mode(AppMode::Chat).await?;
                    }
                }
                // If not enough time has elapsed, ignore the key press
            }
            AppMode::Chat => self.handle_chat_key(event).await?,
            AppMode::Plan => self.handle_plan_key(event).await?,
            AppMode::Sessions => self.handle_sessions_key(event).await?,
            AppMode::FilePicker => self.handle_file_picker_key(event).await?,
            AppMode::DirectoryPicker => self.handle_directory_picker_key(event).await?,
            AppMode::ModelSelector => self.handle_model_selector_key(event).await?,
            AppMode::UsageDialog => {
                if keys::is_cancel(&event) || keys::is_enter(&event) {
                    self.switch_mode(AppMode::Chat).await?;
                }
            }
            AppMode::RestartPending => {
                if keys::is_cancel(&event) {
                    self.rebuild_status = None;
                    self.switch_mode(AppMode::Chat).await?;
                } else if keys::is_enter(&event) {
                    // Perform the restart
                    if let Some(session) = &self.current_session {
                        let session_id = session.id;
                        if let Ok(updater) = SelfUpdater::auto_detect()
                            && let Err(e) = updater.restart(session_id)
                        {
                            self.show_error(format!("Restart failed: {}", e));
                            self.switch_mode(AppMode::Chat).await?;
                        }
                        // If restart succeeds, this process is replaced — we never reach here
                    }
                }
            }
            AppMode::Onboarding => {
                self.handle_onboarding_key(event).await?;
            }
            AppMode::Help | AppMode::Settings => {
                if keys::is_cancel(&event) {
                    self.help_scroll_offset = 0;
                    self.switch_mode(AppMode::Chat).await?;
                } else if keys::is_up(&event) {
                    self.help_scroll_offset = self.help_scroll_offset.saturating_sub(1);
                } else if keys::is_down(&event) {
                    self.help_scroll_offset = self.help_scroll_offset.saturating_add(1);
                } else if keys::is_page_up(&event) {
                    self.help_scroll_offset = self.help_scroll_offset.saturating_sub(10);
                } else if keys::is_page_down(&event) {
                    self.help_scroll_offset = self.help_scroll_offset.saturating_add(10);
                }
            }
        }

        Ok(())
    }

    /// Show an error message
    pub(crate) fn show_error(&mut self, error: String) {
        self.is_processing = false;
        self.processing_started_at = None;
        self.streaming_response = None;
        self.streaming_reasoning = None;
        self.cancel_token = None;
        // Preserve context token count from real-time updates if we never got a complete response
        if self.last_input_tokens.is_none() && self.display_token_count > 0 {
            self.last_input_tokens = Some(self.display_token_count as u32);
        }
        // Deny any pending approvals so agent callbacks don't hang, then remove
        for msg in &mut self.messages {
            if let Some(ref mut approval) = msg.approval
                && approval.state == ApprovalState::Pending
            {
                let _ = approval.response_tx.send(ToolApprovalResponse {
                    request_id: approval.request_id,
                    approved: false,
                    reason: Some("Error occurred".to_string()),
                });
                approval.state = ApprovalState::Denied("Error occurred".to_string());
            }
        }
        // Finalize any active tool group
        if let Some(group) = self.active_tool_group.take() {
            let count = group.calls.len();
            self.messages.push(DisplayMessage {
                id: Uuid::new_v4(),
                role: "tool_group".to_string(),
                content: format!("{} tool call{}", count, if count == 1 { "" } else { "s" }),
                timestamp: chrono::Utc::now(),
                token_count: None,
                cost: None,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: Some(group),
                plan_approval: None,
            });
        }
        self.error_message = Some(error);
        // Auto-scroll to show the error
        self.scroll_offset = 0;
    }

    /// Switch to a different mode
    pub(crate) async fn switch_mode(&mut self, mode: AppMode) -> Result<()> {
        tracing::info!("🔄 Switching mode to: {:?}", mode);
        self.mode = mode;

        if mode == AppMode::Sessions {
            self.load_sessions().await?;
        }

        Ok(())
    }

    /// Get total token count for current session
    pub fn total_tokens(&self) -> i32 {
        self.messages.iter().filter_map(|m| m.token_count).sum()
    }

    /// Get context usage as a percentage (0.0 - 100.0, capped)
    /// Uses the latest response's input_tokens as the current context size
    pub fn context_usage_percent(&self) -> f64 {
        if self.context_max_tokens == 0 {
            return 0.0;
        }
        // Find the most recent assistant message's input token count
        // input_tokens represents how much context was sent to the LLM
        let used = self.last_input_tokens.unwrap_or(0) as f64;
        let pct = (used / self.context_max_tokens as f64) * 100.0;
        pct.min(100.0) // Never show more than 100%
    }

    /// Get total cost for current session
    pub fn total_cost(&self) -> f64 {
        self.messages.iter().filter_map(|m| m.cost).sum()
    }

    /// Handle tool approval request — inline in chat
    fn handle_approval_requested(&mut self, request: ToolApprovalRequest) {
        tracing::info!(
            "[APPROVAL] handle_approval_requested called for tool='{}' auto_session={} auto_always={}",
            request.tool_name,
            self.approval_auto_session,
            self.approval_auto_always
        );
        // Deny stale pending approvals from previous requests (keep in chat for context)
        for msg in &mut self.messages {
            if let Some(ref mut approval) = msg.approval
                && approval.state == ApprovalState::Pending
            {
                let _ = approval.response_tx.send(ToolApprovalResponse {
                    request_id: approval.request_id,
                    approved: false,
                    reason: Some("Superseded by new request".to_string()),
                });
                approval.state = ApprovalState::Denied("Superseded by new request".to_string());
            }
        }

        // Auto-approve silently if policy allows
        if self.approval_auto_always || self.approval_auto_session {
            let response = ToolApprovalResponse {
                request_id: request.request_id,
                approved: true,
                reason: None,
            };
            let _ = request.response_tx.send(response.clone());
            let _ = self
                .event_sender()
                .send(TuiEvent::ToolApprovalResponse(response));
            return;
        }

        // Clear streaming overlay so the approval dialog is visible
        if let Some(text) = self.streaming_response.take()
            && !text.trim().is_empty()
        {
            // Persist any streamed text as a regular message before showing approval
            self.messages.push(DisplayMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: text,
                timestamp: chrono::Utc::now(),
                token_count: None,
                cost: None,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: None,
                plan_approval: None,
            });
        }

        // Show inline approval in chat
        self.messages.push(DisplayMessage {
            id: Uuid::new_v4(),
            role: "approval".to_string(),
            content: String::new(),
            timestamp: chrono::Utc::now(),
            token_count: None,
            cost: None,
            approval: Some(ApprovalData {
                tool_name: request.tool_name,
                tool_description: request.tool_description,
                tool_input: request.tool_input,
                capabilities: request.capabilities,
                request_id: request.request_id,
                response_tx: request.response_tx,
                requested_at: request.requested_at,
                state: ApprovalState::Pending,
                selected_option: 0,
                show_details: false,
            }),
            approve_menu: None,
            details: None,
            expanded: false,
            tool_group: None,
            plan_approval: None,
        });
        // Auto-collapse all tool groups so the approval dialog is immediately visible
        if let Some(ref mut group) = self.active_tool_group {
            group.expanded = false;
        }
        for msg in self.messages.iter_mut() {
            if let Some(ref mut group) = msg.tool_group {
                group.expanded = false;
            }
        }
        self.auto_scroll = true;
        self.scroll_offset = 0;
        tracing::info!(
            "[APPROVAL] Pushed approval message for tool='{}', total messages={}, has_pending={}",
            self.messages
                .last()
                .map(|m| m
                    .approval
                    .as_ref()
                    .map(|a| a.tool_name.as_str())
                    .unwrap_or("?"))
                .unwrap_or("?"),
            self.messages.len(),
            self.has_pending_approval()
        );
        // Stay in AppMode::Chat — no mode switch
    }

    /// Update slash command autocomplete suggestions (built-in + user-defined)
    pub(crate) fn update_slash_suggestions(&mut self) {
        let input = self.input_buffer.trim_start();
        if input.starts_with('/') && !input.contains(' ') && !input.is_empty() {
            let prefix = input.to_lowercase();

            // Built-in commands: indices 0..SLASH_COMMANDS.len()
            self.slash_filtered = SLASH_COMMANDS
                .iter()
                .enumerate()
                .filter(|(_, cmd)| cmd.name.starts_with(&prefix))
                .map(|(i, _)| i)
                .collect();

            // User-defined commands: indices starting at SLASH_COMMANDS.len()
            // Skip user commands that shadow a built-in name
            let base = SLASH_COMMANDS.len();
            for (i, ucmd) in self.user_commands.iter().enumerate() {
                if ucmd.name.to_lowercase().starts_with(&prefix)
                    && !SLASH_COMMANDS.iter().any(|b| b.name == ucmd.name)
                {
                    self.slash_filtered.push(base + i);
                }
            }

            // Sort suggestions alphabetically by command name
            self.slash_filtered.sort_by(|&a, &b| {
                let name_a = if a < SLASH_COMMANDS.len() {
                    SLASH_COMMANDS[a].name
                } else {
                    self.user_commands
                        .get(a - SLASH_COMMANDS.len())
                        .map(|c| c.name.as_str())
                        .unwrap_or("")
                };
                let name_b = if b < SLASH_COMMANDS.len() {
                    SLASH_COMMANDS[b].name
                } else {
                    self.user_commands
                        .get(b - SLASH_COMMANDS.len())
                        .map(|c| c.name.as_str())
                        .unwrap_or("")
                };
                name_a.cmp(name_b)
            });

            self.slash_suggestions_active = !self.slash_filtered.is_empty();
            // Clamp selected index
            if self.slash_selected_index >= self.slash_filtered.len() {
                self.slash_selected_index = 0;
            }
        } else {
            self.slash_suggestions_active = false;
            self.slash_filtered.clear();
            self.slash_selected_index = 0;
        }
    }

    /// Get the name of a slash command by its combined index
    /// (built-in indices 0..N, user command indices N..)
    pub fn slash_command_name(&self, index: usize) -> Option<&str> {
        if index < SLASH_COMMANDS.len() {
            Some(SLASH_COMMANDS[index].name)
        } else {
            self.user_commands
                .get(index - SLASH_COMMANDS.len())
                .map(|c| c.name.as_str())
        }
    }

    /// Get the description of a slash command by its combined index
    pub fn slash_command_description(&self, index: usize) -> Option<&str> {
        if index < SLASH_COMMANDS.len() {
            Some(SLASH_COMMANDS[index].description)
        } else {
            self.user_commands
                .get(index - SLASH_COMMANDS.len())
                .map(|c| c.description.as_str())
        }
    }

    /// Reload user commands from brain workspace (called after agent responses)
    pub(crate) fn reload_user_commands(&mut self) {
        let command_loader = CommandLoader::from_brain_path(&self.brain_path);
        self.user_commands = command_loader.load();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_message_from_db_message() {
        let msg = Message {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "Hello".to_string(),
            sequence: 1,
            created_at: chrono::Utc::now(),
            token_count: Some(10),
            cost: Some(0.001),
        };

        let display_msg: DisplayMessage = msg.into();
        assert_eq!(display_msg.role, "user");
        assert_eq!(display_msg.content, "Hello");
    }
}
