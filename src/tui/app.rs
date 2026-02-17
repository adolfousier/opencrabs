//! TUI Application State
//!
//! Core state management for the terminal user interface.

use super::events::{AppMode, EventHandler, ToolApprovalRequest, ToolApprovalResponse, TuiEvent};
use super::onboarding::{OnboardingWizard, WizardAction};
use super::plan::PlanDocument;
use super::prompt_analyzer::PromptAnalyzer;
use crate::brain::{BrainLoader, CommandLoader, SelfUpdater, UserCommand};
use crate::db::models::{Message, Session};
use crate::llm::agent::AgentService;
use crate::llm::provider::{ContentBlock, LLMRequest};
use crate::services::{MessageService, PlanService, ServiceContext, SessionService};
use anyhow::Result;
use serde_json::Value;
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
    pub selected_option: usize,  // 0-2, arrow key navigation
    pub show_details: bool,      // V key toggle
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
    pub selected_option: usize, // 0-2
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
    pub selected_option: usize, // 0=Approve, 1=Reject, 2=Request Changes, 3=View Plan
    pub show_details: bool,     // toggle task list
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
const IMAGE_EXTENSIONS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"];

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
    // Core state
    pub current_session: Option<Session>,
    pub messages: Vec<DisplayMessage>,
    pub sessions: Vec<Session>,

    // UI state
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

    // Streaming state
    pub is_processing: bool,
    pub streaming_response: Option<String>,
    pub error_message: Option<String>,

    // Animation state
    pub animation_frame: usize,

    // Splash screen state
    splash_shown_at: Option<std::time::Instant>,

    // Escape confirmation state (double-press to clear)
    escape_pending_at: Option<std::time::Instant>,

    // Ctrl+C confirmation state (first clears input, second quits)
    ctrl_c_pending_at: Option<std::time::Instant>,

    // Help/Settings scroll offset
    pub help_scroll_offset: usize,

    // Model name for display (from provider default)
    pub default_model_name: String,

    // Approval policy state
    pub approval_auto_session: bool,
    pub approval_auto_always: bool,

    // Plan mode state
    pub current_plan: Option<PlanDocument>,
    pub plan_scroll_offset: usize,
    pub selected_task_index: Option<usize>,
    pub executing_plan: bool,

    // File picker state
    pub file_picker_files: Vec<std::path::PathBuf>,
    pub file_picker_selected: usize,
    pub file_picker_scroll_offset: usize,
    pub file_picker_current_dir: std::path::PathBuf,

    // Slash autocomplete state
    pub slash_suggestions_active: bool,
    pub slash_filtered: Vec<usize>, // indices into SLASH_COMMANDS
    pub slash_selected_index: usize,

    // Session rename state
    pub session_renaming: bool,
    pub session_rename_buffer: String,

    // Model selector state
    pub model_selector_models: Vec<String>,
    pub model_selector_selected: usize,

    // Input history (arrow up/down to cycle through past messages)
    input_history: Vec<String>,
    input_history_index: Option<usize>,  // None = not browsing, Some(i) = viewing history[i]
    input_history_stash: String,         // saves current input when entering history

    // Working directory
    pub working_directory: std::path::PathBuf,

    // Brain state
    pub brain_path: PathBuf,
    pub user_commands: Vec<UserCommand>,

    // Onboarding wizard state
    pub onboarding: Option<OnboardingWizard>,
    pub force_onboard: bool,

    // Cancellation token for aborting in-progress requests
    cancel_token: Option<CancellationToken>,

    // Queued message — shared with agent so it can be injected between tool calls
    pub(crate) message_queue: Arc<tokio::sync::Mutex<Option<String>>>,

    // Shared session ID — channels (Telegram, WhatsApp) read this to use the same session
    shared_session_id: Arc<tokio::sync::Mutex<Option<Uuid>>>,

    // Context window tracking
    pub context_max_tokens: u32,
    pub last_input_tokens: Option<u32>,

    // Active tool call group (during processing)
    pub active_tool_group: Option<ToolCallGroup>,

    // Self-update state
    pub rebuild_status: Option<String>,

    /// Session to resume after restart (set via --session CLI arg)
    pub resume_session_id: Option<Uuid>,

    // Services
    agent_service: Arc<AgentService>,
    session_service: SessionService,
    message_service: MessageService,
    plan_service: PlanService,

    // Events
    event_handler: EventHandler,

    // Prompt analyzer
    prompt_analyzer: PromptAnalyzer,
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
            streaming_response: None,
            error_message: None,
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
            input_history: Self::load_history(),
            input_history_index: None,
            input_history_stash: String::new(),
            working_directory: std::env::current_dir().unwrap_or_default(),
            brain_path,
            user_commands,
            onboarding: None,
            force_onboard: false,
            cancel_token: None,
            message_queue: Arc::new(tokio::sync::Mutex::new(None)),
            shared_session_id: Arc::new(tokio::sync::Mutex::new(None)),
            default_model_name: agent_service.provider_model().to_string(),
            context_max_tokens: agent_service.context_window_for_model(agent_service.provider_model()),
            last_input_tokens: None,
            active_tool_group: None,
            rebuild_status: None,
            resume_session_id: None,
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
            let agent_service = self.agent_service.clone();
            let event_sender = self.event_sender();
            let token = CancellationToken::new();
            self.cancel_token = Some(token.clone());
            tokio::spawn(async move {
                let wake_up = "[SYSTEM: You just rebuilt yourself from source and restarted \
                    via exec(). Greet the user, confirm the restart succeeded, and continue \
                    where you left off.]";
                match agent_service
                    .send_message_with_tools_and_mode(session_id, wake_up.to_string(), None, false, Some(token))
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
                // Handle paste events - only in Chat mode
                if self.mode == AppMode::Chat {
                    // Check if pasted text contains image paths — extract as attachments
                    let (clean_text, new_attachments) = Self::extract_image_paths(&text);
                    if !new_attachments.is_empty() {
                        self.attachments.extend(new_attachments);
                        if !clean_text.trim().is_empty() {
                            self.input_buffer.insert_str(self.cursor_position, &clean_text);
                            self.cursor_position += clean_text.len();
                        }
                    } else {
                        self.input_buffer.insert_str(self.cursor_position, &text);
                        self.cursor_position += text.len();
                    }
                    self.update_slash_suggestions();
                }
            }
            TuiEvent::MessageSubmitted(content) => {
                self.send_message(content).await?;
            }
            TuiEvent::ResponseChunk(chunk) => {
                self.append_streaming_chunk(chunk);
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
            TuiEvent::ToolCallStarted { tool_name, tool_input } => {
                tracing::info!("[TUI] ToolCallStarted: {} (active_group={}, msg_count={})",
                    tool_name, self.active_tool_group.is_some(), self.messages.len());
                // Show tool call in progress
                let desc = Self::format_tool_description(&tool_name, &tool_input);
                let entry = ToolCallEntry { description: desc, success: true, details: None };
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
            TuiEvent::IntermediateText(text) => {
                tracing::info!("[TUI] IntermediateText: len={} active_group={} streaming={}",
                    text.len(), self.active_tool_group.is_some(), self.streaming_response.is_some());
                // Clear streaming response — text was already shown live via streaming chunks,
                // now it becomes a permanent message in the chat history.
                self.streaming_response = None;

                // Agent sent text between tool call batches — flush current
                // tool group first so tool calls appear above the text.
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
                // Add the intermediate text as an assistant message
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
                if self.auto_scroll {
                    self.scroll_offset = 0;
                }
            }
            TuiEvent::ToolCallCompleted { tool_name, tool_input, success, summary } => {
                let desc = Self::format_tool_description(&tool_name, &tool_input);
                let details = if summary.is_empty() { None } else { Some(summary) };
                let entry = ToolCallEntry { description: desc, success, details };

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
            TuiEvent::CompactionSummary(summary) => {
                // Show the compaction summary as a system message in chat
                self.messages.push(DisplayMessage {
                    id: Uuid::new_v4(),
                    role: "system".to_string(),
                    content: format!("**Context compacted.** Summary saved to daily memory log.\n\n{}", summary),
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
            TuiEvent::RestartReady(status) => {
                self.rebuild_status = Some(status);
                self.switch_mode(AppMode::RestartPending).await?;
            }
            TuiEvent::ConfigReloaded => {
                // Refresh cached config values and commands
                self.reload_user_commands();
                tracing::info!("Config reloaded — refreshed commands and settings");
            }
            TuiEvent::FocusGained | TuiEvent::FocusLost => {
                // Handled by the event loop for tick coalescing
            }
            TuiEvent::Resize(_, _) | TuiEvent::AgentProcessing => {
                // These are handled by the render loop
            }
        }
        Ok(())
    }

    /// Handle keyboard input
    async fn handle_key_event(&mut self, event: crossterm::event::KeyEvent) -> Result<()> {
        use super::events::keys;
        use crossterm::event::{KeyCode, KeyModifiers};

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
                && pending_at.elapsed() < std::time::Duration::from_secs(3) {
                    // Second Ctrl+C within window — quit
                    self.should_quit = true;
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

        // Ctrl+Left — jump to previous word boundary
        if event.code == KeyCode::Left && event.modifiers.contains(KeyModifiers::CONTROL) {
            let before = &self.input_buffer[..self.cursor_position];
            // Skip whitespace, then find start of word
            let trimmed = before.trim_end();
            self.cursor_position = trimmed
                .rfind(char::is_whitespace)
                .map(|pos| pos + 1)
                .unwrap_or(0);
            return Ok(());
        }

        // Ctrl+Right — jump to next word boundary
        if event.code == KeyCode::Right && event.modifiers.contains(KeyModifiers::CONTROL) {
            let after = &self.input_buffer[self.cursor_position..];
            // Skip current word chars, then skip whitespace
            let word_end = after.find(char::is_whitespace).unwrap_or(after.len());
            let rest = &after[word_end..];
            let space_end = rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len());
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
                    && shown_at.elapsed() >= std::time::Duration::from_secs(3) {
                        self.splash_shown_at = None;
                        // Check if onboarding should be shown
                        if self.force_onboard
                            || super::onboarding::is_first_time()
                        {
                            self.force_onboard = false;
                            self.onboarding = Some(OnboardingWizard::new());
                            self.switch_mode(AppMode::Onboarding).await?;
                        } else {
                            self.switch_mode(AppMode::Chat).await?;
                        }
                    }
                    // If not enough time has elapsed, ignore the key press
            }
            AppMode::Chat => self.handle_chat_key(event).await?,
            AppMode::Plan => self.handle_plan_key(event).await?,
            AppMode::Sessions => self.handle_sessions_key(event).await?,
            AppMode::FilePicker => self.handle_file_picker_key(event).await?,
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
                            && let Err(e) = updater.restart(session_id) {
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

    /// Delete the word before the cursor (for Ctrl+Backspace and Alt+Backspace)
    fn delete_last_word(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        let before = &self.input_buffer[..self.cursor_position];
        // Skip trailing whitespace
        let trimmed = before.trim_end();
        // Find the last whitespace boundary in the trimmed portion
        let word_start = trimmed
            .rfind(char::is_whitespace)
            .map(|pos| pos + 1)
            .unwrap_or(0);
        // Remove from word_start to cursor_position
        self.input_buffer.drain(word_start..self.cursor_position);
        self.cursor_position = word_start;
    }

    /// History file path: ~/.opencrabs/history.txt
    fn history_path() -> Option<std::path::PathBuf> {
        Some(crate::config::opencrabs_home().join("history.txt"))
    }

    /// Load input history from disk (one entry per line, most recent last)
    fn load_history() -> Vec<String> {
        let Some(path) = Self::history_path() else {
            return Vec::new();
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => content
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Append a single entry to the history file (and trim to 500 entries)
    fn save_history_entry(&self, entry: &str) {
        let Some(path) = Self::history_path() else {
            return;
        };
        // Ensure directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Cap at 500 entries: keep last 499 + new entry
        let max_entries = 500;
        if self.input_history.len() > max_entries {
            // Rewrite the whole file with only the last max_entries
            let start = self.input_history.len().saturating_sub(max_entries);
            let trimmed: Vec<&str> = self.input_history[start..].iter().map(|s| s.as_str()).collect();
            let _ = std::fs::write(&path, trimmed.join("\n") + "\n");
        } else {
            // Just append
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(f, "{}", entry);
            }
        }
    }

    pub fn has_pending_approval(&self) -> bool {
        self.messages.iter().rev().any(|msg| {
            msg.approval
                .as_ref()
                .is_some_and(|a| a.state == ApprovalState::Pending)
        })
    }

    pub fn has_pending_plan_approval(&self) -> bool {
        self.messages.iter().rev().any(|msg| {
            msg.plan_approval
                .as_ref()
                .is_some_and(|p| p.state == PlanApprovalState::Pending)
        })
    }

    fn has_pending_approve_menu(&self) -> bool {
        self.messages.iter().rev().any(|msg| {
            msg.approve_menu
                .as_ref()
                .is_some_and(|m| m.state == ApproveMenuState::Pending)
        })
    }

    /// Handle keys in chat mode
    async fn handle_chat_key(&mut self, event: crossterm::event::KeyEvent) -> Result<()> {
        use super::events::keys;
        use crossterm::event::{KeyCode, KeyModifiers};

        // Intercept keys when /approve menu is pending
        if self.has_pending_approve_menu() {
            if keys::is_up(&event) {
                if let Some(menu) = self.messages.iter_mut().rev()
                    .find_map(|m| m.approve_menu.as_mut())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                {
                    menu.selected_option = menu.selected_option.saturating_sub(1);
                }
                return Ok(());
            } else if keys::is_down(&event) {
                if let Some(menu) = self.messages.iter_mut().rev()
                    .find_map(|m| m.approve_menu.as_mut())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                {
                    menu.selected_option = (menu.selected_option + 1).min(2);
                }
                return Ok(());
            } else if keys::is_enter(&event) || keys::is_submit(&event) {
                let selected = self.messages.iter()
                    .rev()
                    .find_map(|m| m.approve_menu.as_ref())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                    .map(|m| m.selected_option)
                    .unwrap_or(0);

                // Apply policy
                match selected {
                    0 => {
                        // Reset to approve-only
                        self.approval_auto_session = false;
                        self.approval_auto_always = false;
                    }
                    1 => {
                        // Allow all for this session
                        self.approval_auto_session = true;
                        self.approval_auto_always = false;
                    }
                    _ => {
                        // Yolo mode
                        self.approval_auto_session = false;
                        self.approval_auto_always = true;
                    }
                }

                let label = match selected {
                    0 => "Approve-only (always ask)",
                    1 => "Allow all for this session",
                    _ => "Yolo mode (execute without approval)",
                };

                // Mark menu as resolved
                if let Some(menu) = self.messages.iter_mut().rev()
                    .find_map(|m| m.approve_menu.as_mut())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                {
                    menu.state = ApproveMenuState::Selected(selected);
                }

                // Persist to config.toml
                let policy_str = match selected {
                    0 => "ask",
                    1 => "auto-session",
                    _ => "auto-always",
                };
                if let Err(e) = crate::config::Config::write_key("agent", "approval_policy", policy_str) {
                    tracing::warn!("Failed to persist approval policy: {}", e);
                }

                self.push_system_message(format!("Approval policy set to: {}", label));
                return Ok(());
            } else if keys::is_cancel(&event) {
                // Cancel — dismiss menu without changing policy
                if let Some(menu) = self.messages.iter_mut().rev()
                    .find_map(|m| m.approve_menu.as_mut())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                {
                    menu.state = ApproveMenuState::Selected(99); // sentinel for cancelled
                }
                self.push_system_message("Approval policy unchanged.".to_string());
                return Ok(());
            }
            return Ok(());
        }

        // Intercept keys when an inline approval is pending
        // Options: Yes(0), Always(1), No(2)
        if self.has_pending_approval() {
            if keys::is_left(&event) || keys::is_up(&event) {
                // Navigate options left
                if let Some(approval) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approval.as_mut())
                    .filter(|a| a.state == ApprovalState::Pending)
                {
                    approval.selected_option = approval.selected_option.saturating_sub(1);
                }
                return Ok(());
            } else if keys::is_right(&event) || keys::is_down(&event) {
                // Navigate options right
                if let Some(approval) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approval.as_mut())
                    .filter(|a| a.state == ApprovalState::Pending)
                {
                    approval.selected_option = (approval.selected_option + 1).min(2);
                }
                return Ok(());
            } else if keys::is_enter(&event) || keys::is_submit(&event) {
                // Confirm: Yes(0)=approve once, Always(1)=approve always, No(2)=deny
                let approval_data: Option<(Uuid, usize, mpsc::UnboundedSender<ToolApprovalResponse>)> = self
                    .messages
                    .iter()
                    .rev()
                    .find_map(|m| m.approval.as_ref())
                    .filter(|a| a.state == ApprovalState::Pending)
                    .map(|a| (a.request_id, a.selected_option, a.response_tx.clone()));

                if let Some((request_id, selected, response_tx)) = approval_data {
                    if selected == 2 {
                        // "No" — deny
                        let response = ToolApprovalResponse {
                            request_id,
                            approved: false,
                            reason: Some("User denied permission".to_string()),
                        };
                        if let Err(e) = response_tx.send(response.clone()) {
                            tracing::error!("Failed to send denial response back to agent: {:?}", e);
                        }
                        let _ = self.event_sender().send(TuiEvent::ToolApprovalResponse(response));
                    } else {
                        // "Yes" (0) or "Always" (1)
                        let option = if selected == 1 {
                            ApprovalOption::AllowAlways
                        } else {
                            ApprovalOption::AllowOnce
                        };
                        if matches!(option, ApprovalOption::AllowAlways) {
                            self.approval_auto_session = true;
                            self.push_system_message("Auto-approve enabled for this session. Use /approve to reset.".to_string());
                        }
                        let response = ToolApprovalResponse {
                            request_id,
                            approved: true,
                            reason: None,
                        };
                        if let Err(e) = response_tx.send(response.clone()) {
                            tracing::error!("Failed to send approval response back to agent: {:?}", e);
                        }
                        let _ = self.event_sender().send(TuiEvent::ToolApprovalResponse(response));
                    }
                    // Remove resolved approval messages to prevent channel accumulation
                    self.messages.retain(|m| {
                        m.approval.as_ref().is_none_or(|a| a.request_id != request_id)
                    });
                }
                return Ok(());
            } else if keys::is_deny(&event) || keys::is_cancel(&event) {
                // D/Esc shortcut — deny directly
                let approval_data: Option<(Uuid, mpsc::UnboundedSender<ToolApprovalResponse>)> = self
                    .messages
                    .iter()
                    .rev()
                    .find_map(|m| m.approval.as_ref())
                    .filter(|a| a.state == ApprovalState::Pending)
                    .map(|a| (a.request_id, a.response_tx.clone()));

                if let Some((request_id, response_tx)) = approval_data {
                    let response = ToolApprovalResponse {
                        request_id,
                        approved: false,
                        reason: Some("User denied permission".to_string()),
                    };
                    if let Err(e) = response_tx.send(response.clone()) {
                        tracing::error!("Failed to send denial response back to agent: {:?}", e);
                    }
                    let _ = self.event_sender().send(TuiEvent::ToolApprovalResponse(response));
                    // Remove resolved approval message
                    self.messages.retain(|m| {
                        m.approval.as_ref().is_none_or(|a| a.request_id != request_id)
                    });
                }
                return Ok(());
            } else if keys::is_view_details(&event) {
                // V key — toggle details
                if let Some(approval) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approval.as_mut())
                    .filter(|a| a.state == ApprovalState::Pending)
                {
                    approval.show_details = !approval.show_details;
                }
                return Ok(());
            }
            // Other keys ignored while approval pending
            return Ok(());
        }

        // Intercept keys when an inline plan approval is pending
        // Options: Approve(0), Reject(1), Request Changes(2), View Plan(3)
        if self.has_pending_plan_approval() {
            if keys::is_left(&event) || keys::is_up(&event) {
                if let Some(pa) = self.messages.iter_mut().rev()
                    .find_map(|m| m.plan_approval.as_mut())
                    .filter(|p| p.state == PlanApprovalState::Pending)
                {
                    pa.selected_option = pa.selected_option.saturating_sub(1);
                }
                return Ok(());
            } else if keys::is_right(&event) || keys::is_down(&event) {
                if let Some(pa) = self.messages.iter_mut().rev()
                    .find_map(|m| m.plan_approval.as_mut())
                    .filter(|p| p.state == PlanApprovalState::Pending)
                {
                    pa.selected_option = (pa.selected_option + 1).min(3);
                }
                return Ok(());
            } else if keys::is_enter(&event) || keys::is_submit(&event) {
                let selected = self.messages.iter()
                    .rev()
                    .find_map(|m| m.plan_approval.as_ref())
                    .filter(|p| p.state == PlanApprovalState::Pending)
                    .map(|p| p.selected_option);

                if let Some(selected) = selected {
                    match selected {
                        0 => {
                            // Approve — same as Ctrl+A
                            if let Some(pa) = self.messages.iter_mut().rev()
                                .find_map(|m| m.plan_approval.as_mut())
                                .filter(|p| p.state == PlanApprovalState::Pending)
                            {
                                pa.state = PlanApprovalState::Approved;
                            }
                            if let Some(plan) = &mut self.current_plan {
                                plan.approve();
                                plan.start_execution();
                                self.export_plan_to_markdown("PLAN.md").await?;
                                self.save_plan().await?;
                                self.execute_plan_tasks().await?;
                            }
                        }
                        1 => {
                            // Reject — same as Ctrl+R
                            if let Some(pa) = self.messages.iter_mut().rev()
                                .find_map(|m| m.plan_approval.as_mut())
                                .filter(|p| p.state == PlanApprovalState::Pending)
                            {
                                pa.state = PlanApprovalState::Rejected;
                            }
                            if let Some(plan) = &mut self.current_plan {
                                plan.reject();
                                self.save_plan().await?;
                            }
                            self.current_plan = None;
                        }
                        2 => {
                            // Request changes — same as Ctrl+I
                            if let Some(pa) = self.messages.iter_mut().rev()
                                .find_map(|m| m.plan_approval.as_mut())
                                .filter(|p| p.state == PlanApprovalState::Pending)
                            {
                                pa.state = PlanApprovalState::RevisionRequested;
                            }
                            if let Some(plan) = &self.current_plan {
                                let plan_summary = format!(
                                    "Current plan '{}' has {} tasks:\n{}",
                                    plan.title,
                                    plan.tasks.len(),
                                    plan.tasks.iter().enumerate()
                                        .map(|(i, t)| format!("  {}. {} ({})", i + 1, t.title, t.task_type))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                );
                                self.input_buffer = format!(
                                    "Please revise this plan:\n\n{}\n\nRequested changes: ",
                                    plan_summary
                                );
                                self.cursor_position = self.input_buffer.len();
                            }
                        }
                        3 => {
                            // View plan — switch to Plan Mode
                            self.load_plan_for_viewing().await?;
                            if self.current_plan.is_some() {
                                self.switch_mode(AppMode::Plan).await?;
                            }
                        }
                        _ => {}
                    }
                }
                return Ok(());
            } else if keys::is_view_details(&event) {
                // V key — toggle task list
                if let Some(pa) = self.messages.iter_mut().rev()
                    .find_map(|m| m.plan_approval.as_mut())
                    .filter(|p| p.state == PlanApprovalState::Pending)
                {
                    pa.show_details = !pa.show_details;
                }
                return Ok(());
            }
            // Other keys fall through — plan approval is non-blocking, user can still type
        }

        // When slash suggestions are active, intercept navigation keys
        if self.slash_suggestions_active {
            if keys::is_up(&event) {
                self.slash_selected_index = self.slash_selected_index.saturating_sub(1);
                return Ok(());
            } else if keys::is_down(&event) {
                if !self.slash_filtered.is_empty() {
                    self.slash_selected_index =
                        (self.slash_selected_index + 1).min(self.slash_filtered.len() - 1);
                }
                return Ok(());
            } else if keys::is_enter(&event) || keys::is_submit(&event) {
                // Select the highlighted command and execute it
                if let Some(&cmd_idx) = self.slash_filtered.get(self.slash_selected_index) {
                    let cmd_name = self
                        .slash_command_name(cmd_idx)
                        .unwrap_or("")
                        .to_string();
                    self.input_buffer.clear();
                    self.cursor_position = 0;
                    self.slash_suggestions_active = false;
                    self.handle_slash_command(&cmd_name);
                }
                return Ok(());
            } else if keys::is_cancel(&event) {
                // Dismiss dropdown but keep input
                self.slash_suggestions_active = false;
                return Ok(());
            }
            // Other keys fall through to normal handling
        }

        // Any key other than Escape resets escape confirmation
        if !keys::is_cancel(&event) {
            self.escape_pending_at = None;
        }

        if keys::is_newline(&event) {
            // Alt+Enter or Shift+Enter = insert newline for multi-line input
            self.input_buffer.insert(self.cursor_position, '\n');
            self.cursor_position += 1;
        } else if keys::is_submit(&event) && (!self.input_buffer.trim().is_empty() || !self.attachments.is_empty()) {
            // Check for slash commands before sending to LLM
            let content = self.input_buffer.clone();
            if self.handle_slash_command(content.trim()) {
                self.input_buffer.clear();
                self.cursor_position = 0;
                self.slash_suggestions_active = false;
                return Ok(());
            }

            // Also scan typed input for image paths at submit time
            let (clean_text, typed_attachments) = Self::extract_image_paths(&content);
            let mut all_attachments = std::mem::take(&mut self.attachments);
            all_attachments.extend(typed_attachments);

            let final_content = if !all_attachments.is_empty() && clean_text.trim() != content.trim() {
                clean_text
            } else {
                content.clone()
            };

            // Enter = send message
            // Save to input history (dedup consecutive) and persist to disk
            let trimmed = content.trim().to_string();
            if self.input_history.last() != Some(&trimmed) {
                self.input_history.push(trimmed.clone());
                self.save_history_entry(&trimmed);
            }
            self.input_history_index = None;
            self.input_history_stash.clear();

            self.input_buffer.clear();
            self.cursor_position = 0;
            self.attachments.clear();
            self.slash_suggestions_active = false;

            // Build message content with attachment markers for the agent.
            // Format: <<IMG:/path/to/file.png>> — handles spaces in paths.
            let send_content = if all_attachments.is_empty() {
                final_content
            } else {
                let mut msg = final_content.clone();
                for att in &all_attachments {
                    msg.push_str(&format!(" <<IMG:{}>>", att.path));
                }
                msg
            };
            self.send_message(send_content).await?;
        } else if keys::is_cancel(&event) {
            // When processing, double-Escape aborts the operation
            if self.is_processing {
                if let Some(pending_at) = self.escape_pending_at {
                    if pending_at.elapsed() < std::time::Duration::from_secs(3) {
                        // Second Escape within 3 seconds — abort
                        if let Some(token) = &self.cancel_token {
                            token.cancel();
                        }
                        self.is_processing = false;
                        self.streaming_response = None;
                        self.cancel_token = None;
                        self.escape_pending_at = None;
                        // Deny any pending approvals so agent callbacks don't hang
                        for msg in &mut self.messages {
                            if let Some(ref mut approval) = msg.approval && approval.state == ApprovalState::Pending {
                                let _ = approval.response_tx.send(ToolApprovalResponse {
                                    request_id: approval.request_id,
                                    approved: false,
                                    reason: Some("Operation cancelled".to_string()),
                                });
                                approval.state = ApprovalState::Denied("Operation cancelled".to_string());
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
                        self.push_system_message("Operation cancelled.".to_string());
                    } else {
                        self.escape_pending_at = Some(std::time::Instant::now());
                        self.error_message =
                            Some("Press Esc again to abort".to_string());
                    }
                } else {
                    self.escape_pending_at = Some(std::time::Instant::now());
                    self.error_message =
                        Some("Press Esc again to abort".to_string());
                }
            } else if self.input_buffer.is_empty() {
                // Nothing to clear, just dismiss error
                self.error_message = None;
                self.escape_pending_at = None;
            } else if let Some(pending_at) = self.escape_pending_at {
                if pending_at.elapsed() < std::time::Duration::from_secs(3) {
                    // Second Escape within 3 seconds — clear input
                    self.input_buffer.clear();
                    self.cursor_position = 0;
                    self.attachments.clear();
                    self.error_message = None;
                    self.escape_pending_at = None;
                    self.slash_suggestions_active = false;
                } else {
                    // Expired — treat as first Escape again
                    self.escape_pending_at = Some(std::time::Instant::now());
                    self.error_message =
                        Some("Press Esc again to clear input".to_string());
                }
            } else {
                // First Escape — show confirmation hint
                self.escape_pending_at = Some(std::time::Instant::now());
                self.error_message =
                    Some("Press Esc again to clear input".to_string());
            }
        } else if event.code == KeyCode::Char('o') && event.modifiers == KeyModifiers::CONTROL {
            // Ctrl+O — toggle expand/collapse on active tool group or most recent tool group/details
            if let Some(ref mut group) = self.active_tool_group {
                group.expanded = !group.expanded;
            } else if let Some(msg) = self.messages.iter_mut().rev()
                .find(|m| m.tool_group.is_some() || m.details.is_some()) {
                if let Some(ref mut group) = msg.tool_group {
                    group.expanded = !group.expanded;
                } else {
                    msg.expanded = !msg.expanded;
                }
            }
        } else if keys::is_page_up(&event) {
            self.scroll_offset = self.scroll_offset.saturating_add(10);
            self.auto_scroll = false;
        } else if keys::is_page_down(&event) {
            self.scroll_offset = self.scroll_offset.saturating_sub(10);
            if self.scroll_offset == 0 {
                self.auto_scroll = true;
            }
        } else if event.code == KeyCode::Backspace && event.modifiers.contains(KeyModifiers::ALT) {
            // Alt+Backspace — delete last word
            self.delete_last_word();
        } else if keys::is_up(&event) && !self.slash_suggestions_active && !self.input_history.is_empty() {
            // Arrow Up — browse input history (older)
            match self.input_history_index {
                None => {
                    // Entering history — stash current input
                    self.input_history_stash = self.input_buffer.clone();
                    let idx = self.input_history.len() - 1;
                    self.input_history_index = Some(idx);
                    self.input_buffer = self.input_history[idx].clone();
                    self.cursor_position = self.input_buffer.len();
                }
                Some(idx) if idx > 0 => {
                    let idx = idx - 1;
                    self.input_history_index = Some(idx);
                    self.input_buffer = self.input_history[idx].clone();
                    self.cursor_position = self.input_buffer.len();
                }
                _ => {} // already at oldest
            }
        } else if keys::is_down(&event) && !self.slash_suggestions_active && self.input_history_index.is_some() {
            // Arrow Down — browse input history (newer)
            let idx = self.input_history_index.expect("checked is_some");
            if idx + 1 < self.input_history.len() {
                let idx = idx + 1;
                self.input_history_index = Some(idx);
                self.input_buffer = self.input_history[idx].clone();
                self.cursor_position = self.input_buffer.len();
            } else {
                // Past newest — restore stashed input
                self.input_history_index = None;
                self.input_buffer = std::mem::take(&mut self.input_history_stash);
                self.cursor_position = self.input_buffer.len();
            }
        } else {
            // Regular character input
            match event.code {
                KeyCode::Char('@') => {
                    self.open_file_picker().await?;
                }
                KeyCode::Char(c) if event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT => {
                    self.input_buffer.insert(self.cursor_position, c);
                    self.cursor_position += c.len_utf8();
                }
                KeyCode::Backspace if event.modifiers.is_empty() => {
                    if self.cursor_position > 0 {
                        // Find the previous char boundary
                        let prev = self.input_buffer[..self.cursor_position]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.input_buffer.remove(prev);
                        self.cursor_position = prev;
                    }
                }
                KeyCode::Delete if event.modifiers.is_empty() => {
                    if self.cursor_position < self.input_buffer.len() {
                        self.input_buffer.remove(self.cursor_position);
                    }
                }
                KeyCode::Left if event.modifiers.is_empty() => {
                    // Move cursor left one character
                    if self.cursor_position > 0 {
                        let prev = self.input_buffer[..self.cursor_position]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.cursor_position = prev;
                    }
                }
                KeyCode::Right if event.modifiers.is_empty() => {
                    // Move cursor right one character
                    if self.cursor_position < self.input_buffer.len() {
                        let next = self.input_buffer[self.cursor_position..]
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| self.cursor_position + i)
                            .unwrap_or(self.input_buffer.len());
                        self.cursor_position = next;
                    }
                }
                KeyCode::Home => {
                    self.cursor_position = 0;
                }
                KeyCode::End => {
                    self.cursor_position = self.input_buffer.len();
                }
                KeyCode::Enter => {
                    // Fallback — if Enter didn't match is_submit (e.g., empty input)
                    // do nothing
                }
                _ => {}
            }
        }

        // Update slash autocomplete after any keystroke that modifies input
        self.update_slash_suggestions();

        Ok(())
    }

    /// Handle keys in sessions mode
    async fn handle_sessions_key(&mut self, event: crossterm::event::KeyEvent) -> Result<()> {
        use super::events::keys;
        use crossterm::event::KeyCode;

        // Rename mode: typing the new name
        if self.session_renaming {
            match event.code {
                KeyCode::Enter => {
                    // Save the new name
                    if let Some(session) = self.sessions.get(self.selected_session_index) {
                        let new_title = if self.session_rename_buffer.trim().is_empty() {
                            None
                        } else {
                            Some(self.session_rename_buffer.trim().to_string())
                        };
                        let session_id = session.id;
                        self.session_service
                            .update_session_title(session_id, new_title)
                            .await?;
                        // Update current session if it's the one being renamed
                        if let Some(ref mut current) = self.current_session
                            && current.id == session_id {
                                current.title = if self.session_rename_buffer.trim().is_empty() {
                                    None
                                } else {
                                    Some(self.session_rename_buffer.trim().to_string())
                                };
                            }
                        self.load_sessions().await?;
                    }
                    self.session_renaming = false;
                    self.session_rename_buffer.clear();
                }
                KeyCode::Esc => {
                    self.session_renaming = false;
                    self.session_rename_buffer.clear();
                }
                KeyCode::Backspace => {
                    self.session_rename_buffer.pop();
                }
                KeyCode::Char(c) => {
                    self.session_rename_buffer.push(c);
                }
                _ => {}
            }
            return Ok(());
        }

        // Normal sessions mode
        if keys::is_cancel(&event) {
            self.switch_mode(AppMode::Chat).await?;
        } else if keys::is_up(&event) {
            self.selected_session_index = self.selected_session_index.saturating_sub(1);
        } else if keys::is_down(&event) {
            self.selected_session_index =
                (self.selected_session_index + 1).min(self.sessions.len().saturating_sub(1));
        } else if keys::is_enter(&event) {
            if let Some(session) = self.sessions.get(self.selected_session_index) {
                self.load_session(session.id).await?;
                self.switch_mode(AppMode::Chat).await?;
            }
        } else if event.code == KeyCode::Char('r') || event.code == KeyCode::Char('R') {
            // Start renaming the selected session
            if let Some(session) = self.sessions.get(self.selected_session_index) {
                self.session_renaming = true;
                self.session_rename_buffer = session.title.clone().unwrap_or_default();
            }
        } else if event.code == KeyCode::Char('n') || event.code == KeyCode::Char('N') {
            // Create a new session and switch to it
            self.create_new_session().await?;
            self.switch_mode(AppMode::Chat).await?;
        } else if event.code == KeyCode::Char('d') || event.code == KeyCode::Char('D') {
            // Delete the selected session
            if let Some(session) = self.sessions.get(self.selected_session_index) {
                let session_id = session.id;
                let is_current = self
                    .current_session
                    .as_ref()
                    .map(|s| s.id == session_id)
                    .unwrap_or(false);
                self.session_service.delete_session(session_id).await?;
                if is_current {
                    self.current_session = None;
                    self.messages.clear();
                    *self.shared_session_id.lock().await = None;
                }
                self.load_sessions().await?;
                // Adjust index if it's now out of bounds
                if self.selected_session_index >= self.sessions.len() {
                    self.selected_session_index = self.sessions.len().saturating_sub(1);
                }
            }
        }

        Ok(())
    }

    /// Handle keys in plan mode
    async fn handle_plan_key(&mut self, event: crossterm::event::KeyEvent) -> Result<()> {
        use super::events::keys;
        use crossterm::event::{KeyCode, KeyModifiers};

        // Cancel/Escape - return to chat
        if keys::is_cancel(&event) {
            self.switch_mode(AppMode::Chat).await?;
            return Ok(());
        }

        // Ctrl+A - Approve plan
        if event.code == KeyCode::Char('a') && event.modifiers.contains(KeyModifiers::CONTROL) {
            tracing::info!("✅ Ctrl+A pressed - Approving plan");
            if let Some(plan) = &mut self.current_plan {
                plan.approve();
                plan.start_execution();

                // Export plan to markdown file
                self.export_plan_to_markdown("PLAN.md").await?;

                // Save plan to file
                self.save_plan().await?;
                self.switch_mode(AppMode::Chat).await?;
                // Start executing tasks sequentially
                self.execute_plan_tasks().await?;
            }
            return Ok(());
        }

        // Ctrl+R - Reject plan
        if event.code == KeyCode::Char('r') && event.modifiers.contains(KeyModifiers::CONTROL) {
            tracing::info!("❌ Ctrl+R pressed - Rejecting plan");
            if let Some(plan) = &mut self.current_plan {
                plan.reject();
                // Save plan to file
                self.save_plan().await?;
                // Clear the plan from memory and return to chat
                self.current_plan = None;
                self.switch_mode(AppMode::Chat).await?;
            }
            return Ok(());
        }

        // Ctrl+I - Request plan revision
        if event.code == KeyCode::Char('i') && event.modifiers.contains(KeyModifiers::CONTROL) {
            tracing::info!("🔄 Ctrl+I pressed - Requesting plan revision");
            if let Some(plan) = &self.current_plan {
                // Build plan summary for context
                let plan_summary = format!(
                    "Current plan '{}' has {} tasks:\n{}",
                    plan.title,
                    plan.tasks.len(),
                    plan.tasks
                        .iter()
                        .enumerate()
                        .map(|(i, t)| format!("  {}. {} ({})", i + 1, t.title, t.task_type))
                        .collect::<Vec<_>>()
                        .join("\n")
                );

                // Switch back to chat mode
                self.switch_mode(AppMode::Chat).await?;

                // Pre-fill input with revision request
                self.input_buffer = format!(
                    "Please revise this plan:\n\n{}\n\nRequested changes: ",
                    plan_summary
                );
                self.cursor_position = self.input_buffer.len();

                // Keep plan in memory for reference (don't clear it)
            }
            return Ok(());
        }

        // Arrow keys for scrolling tasks
        match event.code {
            KeyCode::Up => {
                self.plan_scroll_offset = self.plan_scroll_offset.saturating_sub(1);
            }
            KeyCode::Down => {
                if let Some(plan) = &self.current_plan {
                    let max_scroll = plan.tasks.len().saturating_sub(1);
                    self.plan_scroll_offset = (self.plan_scroll_offset + 1).min(max_scroll);
                }
            }
            KeyCode::PageUp => {
                self.plan_scroll_offset = self.plan_scroll_offset.saturating_sub(10);
            }
            KeyCode::PageDown => {
                if let Some(plan) = &self.current_plan {
                    let max_scroll = plan.tasks.len().saturating_sub(1);
                    self.plan_scroll_offset = (self.plan_scroll_offset + 10).min(max_scroll);
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Create a new session
    async fn create_new_session(&mut self) -> Result<()> {
        let session = self
            .session_service
            .create_session(Some("New Chat".to_string()))
            .await?;

        self.current_session = Some(session.clone());
        self.messages.clear();
        self.auto_scroll = true;
        self.scroll_offset = 0;
        self.mode = AppMode::Chat;
        self.approval_auto_session = false;
        self.approval_auto_always = false;

        // Sync shared session ID for channels (Telegram, WhatsApp)
        *self.shared_session_id.lock().await = Some(session.id);

        // Reload sessions list
        self.load_sessions().await?;

        Ok(())
    }

    /// Load a session and its messages
    async fn load_session(&mut self, session_id: Uuid) -> Result<()> {
        let session = self
            .session_service
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        let messages = self
            .message_service
            .list_messages_for_session(session_id)
            .await?;

        self.current_session = Some(session.clone());
        self.messages = messages.into_iter().flat_map(Self::expand_message).collect();
        self.auto_scroll = true;
        self.scroll_offset = 0;
        self.approval_auto_session = false;
        self.approval_auto_always = false;

        // Sync shared session ID for channels (Telegram, WhatsApp)
        *self.shared_session_id.lock().await = Some(session.id);

        // Don't estimate context from stored messages — the chars/3 heuristic
        // counts ALL messages (including compacted ones still in DB) which wildly
        // overestimates actual context window usage. Instead, show no percentage
        // until the next API response provides real input_tokens from the model.
        self.last_input_tokens = None;

        Ok(())
    }

    /// Load all sessions
    async fn load_sessions(&mut self) -> Result<()> {
        use crate::db::repository::SessionListOptions;

        self.sessions = self
            .session_service
            .list_sessions(SessionListOptions {
                include_archived: false,
                limit: Some(100),
                offset: 0,
            })
            .await?;

        Ok(())
    }

    /// Clear all messages from the current session
    async fn clear_session(&mut self) -> Result<()> {
        if let Some(session) = &self.current_session {
            // Delete all messages from the database
            self.message_service
                .delete_messages_for_session(session.id)
                .await?;

            // Clear messages from UI
            self.messages.clear();
            self.scroll_offset = 0;
            self.streaming_response = None;
            self.error_message = None;
        }

        Ok(())
    }

    /// Handle slash commands locally (returns true if handled)
    fn handle_slash_command(&mut self, input: &str) -> bool {
        let cmd = input.split_whitespace().next().unwrap_or("");
        match cmd {
            "/models" => {
                self.open_model_selector();
                true
            }
            "/usage" => {
                self.mode = AppMode::UsageDialog;
                true
            }
            "/onboard" => {
                self.onboarding = Some(OnboardingWizard::new());
                self.mode = AppMode::Onboarding;
                true
            }
            "/sessions" => {
                self.mode = AppMode::Sessions;
                let _ = self.event_sender().send(TuiEvent::SwitchMode(AppMode::Sessions));
                true
            }
            "/approve" => {
                self.messages.push(DisplayMessage {
                    id: Uuid::new_v4(),
                    role: "system".to_string(),
                    content: String::new(),
                    timestamp: chrono::Utc::now(),
                    token_count: None,
                    cost: None,
                    approval: None,
                    approve_menu: Some(ApproveMenu {
                        selected_option: 0,
                        state: ApproveMenuState::Pending,
                    }),
                    details: None,
                    expanded: false,
                    tool_group: None,
                    plan_approval: None,
                });
                self.scroll_offset = 0;
                true
            }
            "/compact" => {
                let pct = self.context_usage_percent();
                self.push_system_message(format!(
                    "Compacting context... (currently at {:.0}%)",
                    pct
                ));
                // Trigger compaction by sending a special message to the agent
                let sender = self.event_sender();
                let _ = sender.send(TuiEvent::MessageSubmitted(
                    "[SYSTEM: Compact context now. Summarize this conversation for continuity.]".to_string(),
                ));
                true
            }
            "/rebuild" => {
                self.push_system_message("Building from source...".to_string());
                let sender = self.event_sender();
                tokio::spawn(async move {
                    match SelfUpdater::auto_detect() {
                        Ok(updater) => match updater.build().await {
                            Ok(_) => {
                                let _ = sender.send(TuiEvent::RestartReady(
                                    "Build successful".into(),
                                ));
                            }
                            Err(e) => {
                                let _ = sender.send(TuiEvent::Error(format!(
                                    "Build failed:\n{}", e
                                )));
                            }
                        },
                        Err(e) => {
                            let _ = sender.send(TuiEvent::Error(format!(
                                "Cannot detect project: {}", e
                            )));
                        }
                    }
                });
                true
            }
            "/help" => {
                self.mode = AppMode::Help;
                true
            }
            _ if input.starts_with('/') => {
                // Check user-defined commands
                if let Some(user_cmd) = self.user_commands.iter().find(|c| c.name == cmd) {
                    let prompt = user_cmd.prompt.clone();
                    let action = user_cmd.action.clone();
                    match action.as_str() {
                        "system" => {
                            self.push_system_message(prompt);
                        }
                        _ => {
                            // "prompt" action — send to LLM
                            let sender = self.event_sender();
                            let _ = sender.send(TuiEvent::MessageSubmitted(prompt));
                        }
                    }
                    return true;
                }
                self.push_system_message(format!(
                    "Unknown command: {}. Type /help for available commands.",
                    cmd
                ));
                true
            }
            _ => false,
        }
    }

    /// Format a human-readable description of a tool call from its name and input
    pub fn format_tool_description(tool_name: &str, tool_input: &Value) -> String {
        match tool_name {
            "bash" => {
                let cmd = tool_input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
                let short: String = cmd.chars().take(80).collect();
                if cmd.len() > 80 {
                    format!("bash: {}...", short)
                } else {
                    format!("bash: {}", short)
                }
            }
            "read_file" | "read" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Read {}", path)
            }
            "write_file" | "write" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Write {}", path)
            }
            "edit_file" | "edit" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Edit {}", path)
            }
            "ls" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                format!("ls {}", path)
            }
            "glob" => {
                let pattern = tool_input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Glob {}", pattern)
            }
            "grep" => {
                let pattern = tool_input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if path.is_empty() {
                    format!("Grep '{}'", pattern)
                } else {
                    format!("Grep '{}' in {}", pattern, path)
                }
            }
            "web_search" => {
                let query = tool_input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Search: {}", query)
            }
            "exa_search" => {
                let query = tool_input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
                format!("EXA search: {}", query)
            }
            "brave_search" => {
                let query = tool_input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Brave search: {}", query)
            }
            "http_request" => {
                let url = tool_input.get("url").and_then(|v| v.as_str()).unwrap_or("?");
                let method = tool_input.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
                format!("{} {}", method, url)
            }
            "execute_code" => {
                let lang = tool_input.get("language").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Execute {}", lang)
            }
            "notebook_edit" => {
                let path = tool_input.get("notebook_path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Notebook {}", path)
            }
            "parse_document" => {
                let path = tool_input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Parse {}", path)
            }
            "task_manager" => {
                let op = tool_input.get("operation").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Task: {}", op)
            }
            "plan" => {
                let op = tool_input.get("operation").and_then(|v| v.as_str()).unwrap_or("?");
                format!("Plan: {}", op)
            }
            "session_context" => "Session context".to_string(),
            other => other.to_string(),
        }
    }

    /// Expand a DB message into one or more DisplayMessages.
    /// Assistant messages may contain `<!-- tools: Read /foo | Edit /bar -->` markers
    /// that get reconstructed into ToolCallGroup display messages.
    fn expand_message(msg: crate::db::models::Message) -> Vec<DisplayMessage> {
        if msg.role != "assistant" || !msg.content.contains("<!-- tools:") {
            return vec![DisplayMessage::from(msg)];
        }

        // Extract owned values before borrowing content
        let id = msg.id;
        let timestamp = msg.created_at;
        let token_count = msg.token_count;
        let cost = msg.cost;
        let content = msg.content;

        let mut result = Vec::new();
        let marker = "<!-- tools:";

        // Split on tool markers, preserving the structure
        let mut remaining = content.as_str();
        let mut first_text = true;
        while let Some(marker_start) = remaining.find(marker) {
            // Text before marker
            let text_before = remaining[..marker_start].trim();
            if !text_before.is_empty() {
                result.push(DisplayMessage {
                    id: if first_text { id } else { Uuid::new_v4() },
                    role: "assistant".to_string(),
                    content: text_before.to_string(),
                    timestamp,
                    token_count: if first_text { token_count } else { None },
                    cost: if first_text { cost } else { None },
                    approval: None,
                    approve_menu: None,
                    details: None,
                    expanded: false,
                    tool_group: None,
                    plan_approval: None,
                });
                first_text = false;
            }

            // Parse the tool marker: <!-- tools: desc1 | desc2 -->
            let after_marker = &remaining[marker_start + marker.len()..];
            if let Some(end) = after_marker.find("-->") {
                let tools_str = after_marker[..end].trim();
                let calls: Vec<ToolCallEntry> = tools_str
                    .split(" | ")
                    .map(|desc| ToolCallEntry {
                        description: desc.to_string(),
                        success: true,
                        details: None,
                    })
                    .collect();
                if !calls.is_empty() {
                    let count = calls.len();
                    result.push(DisplayMessage {
                        id: Uuid::new_v4(),
                        role: "tool_group".to_string(),
                        content: format!("{} tool call{}", count, if count == 1 { "" } else { "s" }),
                        timestamp,
                        token_count: None,
                        cost: None,
                        approval: None,
                        approve_menu: None,
                        details: None,
                        expanded: false,
                        tool_group: Some(ToolCallGroup { calls, expanded: false }),
                        plan_approval: None,
                    });
                }
                remaining = &after_marker[end + 3..];
            } else {
                remaining = after_marker;
                break;
            }
        }

        // Any remaining text after the last marker
        let trailing = remaining.trim();
        if !trailing.is_empty() {
            result.push(DisplayMessage {
                id: if first_text { id } else { Uuid::new_v4() },
                role: "assistant".to_string(),
                content: trailing.to_string(),
                timestamp,
                token_count: if first_text { token_count } else { None },
                cost: if first_text { cost } else { None },
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: None,
                plan_approval: None,
            });
        }

        if result.is_empty() {
            // Content was only tool markers with no text — show a placeholder
            result.push(DisplayMessage {
                id,
                role: "assistant".to_string(),
                content: String::new(),
                timestamp,
                token_count,
                cost,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: None,
                plan_approval: None,
            });
        }

        result
    }

    /// Extract image file paths from text and return (remaining_text, attachments).
    /// Handles paths with spaces (e.g. `/home/user/My Screenshots/photo.png`)
    /// and image URLs.
    fn extract_image_paths(text: &str) -> (String, Vec<ImageAttachment>) {
        let trimmed = text.trim();
        let lower = trimmed.to_lowercase();

        // Case 1: Entire pasted text is a single image path (handles spaces in path)
        if IMAGE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
            // Local path
            let path = std::path::Path::new(trimmed);
            if path.exists() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| trimmed.to_string());
                return (String::new(), vec![ImageAttachment {
                    name,
                    path: trimmed.to_string(),
                }]);
            }
            // URL (no spaces — just check prefix)
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                let name = trimmed.rsplit('/').next().unwrap_or(trimmed).to_string();
                return (String::new(), vec![ImageAttachment {
                    name,
                    path: trimmed.to_string(),
                }]);
            }
        }

        // Case 2: Mixed text — scan for image URLs (split by whitespace is fine for URLs)
        // and absolute paths without spaces
        let mut attachments = Vec::new();
        let mut remaining_parts = Vec::new();

        for word in text.split_whitespace() {
            let word_lower = word.to_lowercase();
            let is_image = IMAGE_EXTENSIONS.iter().any(|ext| word_lower.ends_with(ext));

            if is_image {
                let path = std::path::Path::new(word);
                if path.exists() {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| word.to_string());
                    attachments.push(ImageAttachment {
                        name,
                        path: word.to_string(),
                    });
                    continue;
                }
                if word.starts_with("http://") || word.starts_with("https://") {
                    let name = word.rsplit('/').next().unwrap_or(word).to_string();
                    attachments.push(ImageAttachment {
                        name,
                        path: word.to_string(),
                    });
                    continue;
                }
            }
            remaining_parts.push(word);
        }

        (remaining_parts.join(" "), attachments)
    }

    /// Replace `<<IMG:/path/to/file.png>>` markers with readable `[IMG: file.png]` for display.
    fn humanize_image_markers(text: &str) -> String {
        let mut result = text.to_string();
        while let Some(start) = result.find("<<IMG:") {
            if let Some(end) = result[start..].find(">>") {
                let path = &result[start + 6..start + end];
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string());
                let replacement = format!("[IMG: {}]", name);
                result = format!("{}{}{}", &result[..start], replacement, &result[start + end + 2..]);
            } else {
                break;
            }
        }
        result.trim().to_string()
    }

    /// Push a system message into the chat display
    fn push_system_message(&mut self, content: String) {
        self.messages.push(DisplayMessage {
            id: Uuid::new_v4(),
            role: "system".to_string(),
            content,
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
        self.scroll_offset = 0;
    }

    /// Send a message to the agent
    async fn send_message(&mut self, content: String) -> Result<()> {
        tracing::info!("[send_message] START is_processing={} has_session={} content_len={}",
            self.is_processing,
            self.current_session.is_some(),
            content.len());

        // Deny stale pending approvals so they don't block streaming, then remove them
        let stale_count = self.messages.iter()
            .filter(|m| m.approval.as_ref().is_some_and(|a| a.state == ApprovalState::Pending))
            .count();
        if stale_count > 0 {
            tracing::warn!("[send_message] Clearing {} stale pending approvals", stale_count);
        }
        for msg in &mut self.messages {
            if let Some(ref mut approval) = msg.approval && approval.state == ApprovalState::Pending {
                let _ = approval.response_tx.send(ToolApprovalResponse {
                    request_id: approval.request_id,
                    approved: false,
                    reason: Some("Superseded".to_string()),
                });
                approval.state = ApprovalState::Denied("Superseded".to_string());
            }
        }
        self.messages.retain(|m| m.approval.is_none() || m.approval.as_ref().is_some_and(|a| a.state == ApprovalState::Pending));

        if self.is_processing {
            tracing::warn!("[send_message] QUEUED — agent still processing previous request");
            // Show the queued message as a real user message immediately
            let user_msg = DisplayMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: Self::humanize_image_markers(&content),
                timestamp: chrono::Utc::now(),
                token_count: None,
                cost: None,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: None,
                plan_approval: None,
            };
            self.messages.push(user_msg);
            self.scroll_offset = 0;

            // Queue for injection between tool calls
            *self.message_queue.lock().await = Some(content);
            return Ok(());
        }
        if let Some(session) = &self.current_session {
            self.is_processing = true;
            self.error_message = None;

            // Analyze and transform the prompt before sending to agent
            let transformed_content = self.prompt_analyzer.analyze_and_transform(&content);

            // Log if the prompt was transformed
            if transformed_content != content {
                tracing::info!("✨ Prompt transformed with tool hints");
            }

            // Add user message to UI — replace <<IMG:...>> markers with readable names
            let display_content = Self::humanize_image_markers(&content);
            let user_msg = DisplayMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: display_content,
                timestamp: chrono::Utc::now(),
                token_count: None,
                cost: None,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: None,
                plan_approval: None,
            };
            self.messages.push(user_msg);

            // Auto-scroll to show the new user message and re-enable auto-scroll
            self.auto_scroll = true;
            self.scroll_offset = 0;

            // Create cancellation token for this request
            let token = CancellationToken::new();
            self.cancel_token = Some(token.clone());

            // Send transformed content to agent in background
            let agent_service = self.agent_service.clone();
            let session_id = session.id;
            let event_sender = self.event_sender();
            let read_only_mode = self.mode == AppMode::Plan;

            tracing::info!("[send_message] Spawning agent task for session {}", session_id);
            let panic_sender = event_sender.clone();
            let handle = tokio::spawn(async move {
                tracing::info!("[agent_task] START calling send_message_with_tools_and_mode");
                let result = agent_service
                    .send_message_with_tools_and_mode(
                        session_id,
                        transformed_content,
                        None,
                        read_only_mode,
                        Some(token),
                    )
                    .await;

                match result {
                    Ok(response) => {
                        tracing::info!("[agent_task] OK — sending ResponseComplete");
                        if let Err(e) = event_sender.send(TuiEvent::ResponseComplete(response)) {
                            tracing::error!("[agent_task] FAILED to send ResponseComplete: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("[agent_task] ERROR: {}", e);
                        if let Err(e2) = event_sender.send(TuiEvent::Error(e.to_string())) {
                            tracing::error!("[agent_task] FAILED to send Error event: {}", e2);
                        }
                    }
                }
            });
            // Watch for panics — surface them in the UI instead of silent hang
            tokio::spawn(async move {
                if let Err(e) = handle.await {
                    tracing::error!("[agent_task] PANICKED: {}", e);
                    let _ = panic_sender.send(TuiEvent::Error(
                        format!("Agent task crashed unexpectedly: {e}. You can continue chatting."),
                    ));
                }
            });
        }

        Ok(())
    }

    /// Append a streaming chunk
    fn append_streaming_chunk(&mut self, chunk: String) {
        if let Some(ref mut response) = self.streaming_response {
            response.push_str(&chunk);
        } else {
            self.streaming_response = Some(chunk);
            // Auto-scroll when response starts streaming (only if user hasn't scrolled up)
            if self.auto_scroll {
                self.scroll_offset = 0;
            }
        }
    }

    /// Complete the streaming response
    async fn complete_response(
        &mut self,
        response: crate::llm::agent::AgentResponse,
    ) -> Result<()> {
        self.is_processing = false;
        self.streaming_response = None;
        self.cancel_token = None;

        // Clean up stale pending approvals — send deny so agent callbacks don't hang, then remove
        for msg in &mut self.messages {
            if let Some(ref mut approval) = msg.approval && approval.state == ApprovalState::Pending {
                tracing::warn!("Cleaning up stale pending approval for tool '{}'", approval.tool_name);
                let _ = approval.response_tx.send(ToolApprovalResponse {
                    request_id: approval.request_id,
                    approved: false,
                    reason: Some("Agent completed without resolution".to_string()),
                });
                approval.state = ApprovalState::Denied("Agent completed without resolution".to_string());
            }
        }
        self.messages.retain(|m| m.approval.is_none() || m.approval.as_ref().is_some_and(|a| a.state == ApprovalState::Pending));

        // Finalize active tool group into a display message
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

        // Reload user commands (agent may have written new ones to commands.json)
        self.reload_user_commands();

        // Check task completion FIRST (before moving response.content)
        let task_failed = if self.executing_plan {
            self.check_task_completion(&response.content).await?
        } else {
            false
        };

        // Track context usage from latest response
        self.last_input_tokens = Some(response.context_tokens);

        // Add assistant message to UI
        let assistant_msg = DisplayMessage {
            id: response.message_id,
            role: "assistant".to_string(),
            content: response.content,
            timestamp: chrono::Utc::now(),
            token_count: Some(response.usage.output_tokens as i32),
            cost: Some(response.cost),
            approval: None,
            approve_menu: None,
            details: None,
            expanded: false,
            tool_group: None,
            plan_approval: None,
        };
        self.messages.push(assistant_msg);

        // Update session model if not already set
        if let Some(session) = &mut self.current_session
            && session.model.is_none() {
                session.model = Some(response.model.clone());
                // Save the updated session to database
                if let Err(e) = self.session_service.update_session(session).await {
                    tracing::warn!("Failed to update session model: {}", e);
                }
            }

        // Auto-scroll to bottom
        self.scroll_offset = 0;

        // Handle plan execution
        if self.executing_plan {
            if task_failed {
                // Stop execution on failure
                self.executing_plan = false;
                let error_msg = DisplayMessage {
                    id: uuid::Uuid::new_v4(),
                    role: "system".to_string(),
                    content: "Plan execution stopped due to task failure. \
                             Review the error above and decide how to proceed."
                        .to_string(),
                    timestamp: chrono::Utc::now(),
                    token_count: None,
                    cost: None,
                    approval: None,
                    approve_menu: None,
                    details: None,
                    expanded: false,
                    tool_group: None,
                    plan_approval: None,
                };
                self.messages.push(error_msg);
            } else {
                // Execute next task if current one succeeded
                self.execute_next_plan_task().await?;
            }
        } else {
            // Check if a plan was created/finalized
            self.check_and_load_plan().await?;
        }

        Ok(())
    }

    /// Check if the current task completed successfully or failed
    /// Returns true if task failed, false if succeeded
    async fn check_task_completion(&mut self, response_content: &str) -> Result<bool> {
        let Some(plan) = &mut self.current_plan else {
            return Ok(false);
        };

        // Find the in-progress task
        let task_result = plan
            .tasks
            .iter_mut()
            .find(|t| matches!(t.status, crate::tui::plan::TaskStatus::InProgress))
            .map(|task| {
                // Check for error indicators in the response
                let response_lower = response_content.to_lowercase();
                let has_error = response_lower.contains("error:")
                    || response_lower.contains("failed to")
                    || response_lower.contains("cannot")
                    || response_lower.contains("unable to")
                    || response_lower.contains("fatal:")
                    || (response_lower.contains("error") && response_lower.contains("executing"))
                    || response_lower.contains("compilation error")
                    || response_lower.contains("build failed");

                if has_error {
                    // Mark task as failed
                    task.status = crate::tui::plan::TaskStatus::Failed;
                    task.notes = Some(
                        "Task failed during execution. Error detected in response.".to_string(),
                    );
                    true // Task failed
                } else {
                    // Mark task as completed successfully
                    task.status = crate::tui::plan::TaskStatus::Completed;
                    task.completed_at = Some(chrono::Utc::now());
                    task.notes = Some("Task completed successfully".to_string());
                    false // Task succeeded
                }
            });

        // Save updated plan
        self.save_plan().await?;

        Ok(task_result.unwrap_or(false))
    }

    /// Load plan for manual viewing (Ctrl+P)
    /// Loads ANY plan (Draft, PendingApproval, etc.) for viewing
    async fn load_plan_for_viewing(&mut self) -> Result<()> {
        // Get session ID for session-scoped operations
        let session_id = match &self.current_session {
            Some(session) => session.id,
            None => {
                tracing::debug!("No current session, skipping plan load");
                return Ok(());
            }
        };

        tracing::debug!("Loading plan for viewing (session: {})", session_id);

        // Try loading from database first
        match self.plan_service.get_most_recent_plan(session_id).await {
            Ok(Some(plan)) => {
                tracing::info!(
                    "✅ Loaded plan from database: '{}' ({:?}, {} tasks)",
                    plan.title,
                    plan.status,
                    plan.tasks.len()
                );
                self.current_plan = Some(plan);
                return Ok(());
            }
            Ok(None) => {
                tracing::debug!("No plan found in database, checking JSON file");
            }
            Err(e) => {
                tracing::warn!("Failed to load plan from database: {}", e);
            }
        }

        // Fallback to JSON file for backward compatibility / migration
        let plan_filename = format!(".opencrabs_plan_{}.json", session_id);
        let plan_file = self.working_directory.join(&plan_filename);

        tracing::debug!("Looking for plan file at: {}", plan_file.display());

        match tokio::fs::read_to_string(&plan_file).await {
            Ok(content) => {
                tracing::debug!("Found plan JSON file, parsing...");
                match serde_json::from_str::<crate::tui::plan::PlanDocument>(&content) {
                    Ok(plan) => {
                        tracing::info!(
                            "✅ Loaded plan from JSON: '{}' ({:?}, {} tasks)",
                            plan.title,
                            plan.status,
                            plan.tasks.len()
                        );

                        // Migrate to database
                        if let Err(e) = self.plan_service.create(&plan).await {
                            tracing::warn!("Failed to migrate plan to database: {}", e);
                        }

                        self.current_plan = Some(plan);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse plan JSON: {}", e);
                    }
                }
            }
            Err(_) => {
                tracing::debug!("No plan file found");
            }
        }

        Ok(())
    }

    /// Check for and load a plan if one was created
    /// Loads from database first, with JSON fallback for migration
    /// Only loads plans with status PendingApproval (for automatic notification)
    async fn check_and_load_plan(&mut self) -> Result<()> {
        // Get session ID for session-scoped operations
        let session_id = match &self.current_session {
            Some(session) => session.id,
            None => {
                tracing::debug!("No current session, skipping plan load");
                return Ok(());
            }
        };

        tracing::debug!("Checking for pending plan (session: {})", session_id);

        // Try loading from database first
        match self.plan_service.get_most_recent_plan(session_id).await {
            Ok(Some(plan)) => {
                tracing::debug!(
                    "Found plan in database: id={}, status={:?}",
                    plan.id,
                    plan.status
                );
                // Only load if plan is pending approval
                if plan.status == crate::tui::plan::PlanStatus::PendingApproval {
                    tracing::info!("✅ Plan ready for review!");

                    // Only load if not already loaded (avoid duplicate messages)
                    if self.current_plan.is_none() {
                        let plan_title = plan.title.clone();
                        let task_count = plan.tasks.len();
                        let task_summaries: Vec<String> = plan.tasks.iter()
                            .map(|t| format!("{} ({})", t.title, t.task_type))
                            .collect();
                        self.current_plan = Some(plan);

                        // Add inline plan approval selector to chat
                        let notification = DisplayMessage {
                            id: Uuid::new_v4(),
                            role: "plan_approval".to_string(),
                            content: String::new(),
                            timestamp: chrono::Utc::now(),
                            token_count: None,
                            cost: None,
                            approval: None,
                            approve_menu: None,
                            details: None,
                            expanded: false,
                            tool_group: None,
                            plan_approval: Some(PlanApprovalData {
                                plan_title,
                                task_count,
                                task_summaries,
                                state: PlanApprovalState::Pending,
                                selected_option: 0,
                                show_details: false,
                            }),
                        };

                        self.messages.push(notification);
                        self.scroll_offset = 0;
                    }
                }
                return Ok(());
            }
            Ok(None) => {
                tracing::debug!("No pending plan found in database, checking JSON file");
            }
            Err(e) => {
                tracing::warn!("Failed to load plan from database: {}", e);
            }
        }

        // Fallback to JSON file for backward compatibility / migration
        let plan_filename = format!(".opencrabs_plan_{}.json", session_id);
        let plan_file = self.working_directory.join(&plan_filename);

        tracing::debug!("Looking for plan file at: {}", plan_file.display());

        // Check if file exists before trying to read
        let file_exists = plan_file.exists();
        tracing::debug!("Plan file exists: {}", file_exists);

        match tokio::fs::read_to_string(&plan_file).await {
            Ok(content) => {
                tracing::debug!("Found plan JSON file, parsing...");
                match serde_json::from_str::<crate::tui::plan::PlanDocument>(&content) {
                    Ok(plan) => {
                        tracing::debug!(
                            "Parsed plan: id={}, status={:?}, tasks={}",
                            plan.id,
                            plan.status,
                            plan.tasks.len()
                        );
                        // Only load if plan is pending approval
                        if plan.status == crate::tui::plan::PlanStatus::PendingApproval {
                            tracing::info!("✅ Plan ready for review!");

                            // Migrate to database
                            if let Err(e) = self.plan_service.create(&plan).await {
                                tracing::warn!("Failed to migrate plan to database: {}", e);
                            }

                            // Only load if not already loaded (avoid duplicate messages)
                            if self.current_plan.is_none() {
                                let plan_title = plan.title.clone();
                                let task_count = plan.tasks.len();
                                let task_summaries: Vec<String> = plan.tasks.iter()
                                    .map(|t| format!("{} ({})", t.title, t.task_type))
                                    .collect();
                                self.current_plan = Some(plan);

                                // Add inline plan approval selector to chat
                                let notification = DisplayMessage {
                                    id: Uuid::new_v4(),
                                    role: "plan_approval".to_string(),
                                    content: String::new(),
                                    timestamp: chrono::Utc::now(),
                                    token_count: None,
                                    cost: None,
                                    approval: None,
                                    approve_menu: None,
                                    details: None,
                                    expanded: false,
                                    tool_group: None,
                                    plan_approval: Some(PlanApprovalData {
                                        plan_title,
                                        task_count,
                                        task_summaries,
                                        state: PlanApprovalState::Pending,
                                        selected_option: 0,
                                        show_details: false,
                                    }),
                                };

                                self.messages.push(notification);
                                self.scroll_offset = 0;
                            }
                        } else {
                            tracing::debug!(
                                "Plan status is {:?}, not PendingApproval - skipping",
                                plan.status
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse plan JSON: {}", e);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("Plan file not found (this is normal if no plan was created)");
            }
            Err(e) => {
                tracing::warn!("Failed to read plan JSON file: {}", e);
            }
        }

        Ok(())
    }

    /// Save the current plan
    /// Dual-write: database as primary, JSON as backup
    /// Export plan to markdown file
    async fn export_plan_to_markdown(&self, filename: &str) -> Result<()> {
        if let Some(plan) = &self.current_plan {
            // Generate markdown content
            let mut markdown = String::new();
            markdown.push_str(&format!("# {}\n\n", plan.title));
            markdown.push_str(&format!("{}\n\n", plan.description));

            if !plan.context.is_empty() {
                markdown.push_str("## Context\n\n");
                markdown.push_str(&format!("{}\n\n", plan.context));
            }

            if !plan.risks.is_empty() {
                markdown.push_str("## Risks & Considerations\n\n");
                for risk in &plan.risks {
                    markdown.push_str(&format!("- {}\n", risk));
                }
                markdown.push('\n');
            }

            markdown.push_str("## Tasks\n\n");

            for task in &plan.tasks {
                markdown.push_str(&format!("### Task {}: {}\n\n", task.order, task.title));
                markdown.push_str(&format!(
                    "**Type:** {:?} | **Complexity:** {}★\n\n",
                    task.task_type, task.complexity
                ));

                if !task.dependencies.is_empty() {
                    let dep_orders: Vec<String> = task
                        .dependencies
                        .iter()
                        .filter_map(|dep_id| {
                            plan.tasks
                                .iter()
                                .find(|t| &t.id == dep_id)
                                .map(|t| t.order.to_string())
                        })
                        .collect();
                    markdown.push_str(&format!(
                        "**Dependencies:** Task(s) {}\n\n",
                        dep_orders.join(", ")
                    ));
                }

                markdown.push_str("**Implementation Steps:**\n\n");
                markdown.push_str(&format!("{}\n\n", task.description));
                markdown.push_str("---\n\n");
            }

            markdown.push_str(&format!(
                "\n*Plan created: {}*\n",
                plan.created_at.format("%Y-%m-%d %H:%M:%S")
            ));
            markdown.push_str(&format!(
                "*Last updated: {}*\n",
                plan.updated_at.format("%Y-%m-%d %H:%M:%S")
            ));

            // Write markdown file to working directory
            let output_path = self.working_directory.join(filename);

            // Write markdown file (overwrite if exists)
            tokio::fs::write(&output_path, markdown)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write markdown file: {}", e))?;

            tracing::info!("Exported plan to {}", output_path.display());
        }

        Ok(())
    }

    async fn save_plan(&self) -> Result<()> {
        if let Some(plan) = &self.current_plan {
            // Get session ID for session-scoped operations
            let session_id = match &self.current_session {
                Some(session) => session.id,
                None => {
                    tracing::warn!("Cannot save plan: no current session");
                    return Ok(());
                }
            };

            // Primary: Save to database
            // Try to update first (plan may already exist)
            match self.plan_service.update(plan).await {
                Ok(_) => {
                    tracing::debug!("Updated plan in database: {}", plan.id);
                }
                Err(_) => {
                    // If update fails, try creating (plan doesn't exist yet)
                    if let Err(e) = self.plan_service.create(plan).await {
                        tracing::error!("Failed to save plan to database: {}", e);
                        // Continue to JSON backup even if database fails
                    } else {
                        tracing::debug!("Created plan in database: {}", plan.id);
                    }
                }
            }

            // Backup: Save to JSON file (for backward compatibility and backup)
            let plan_filename = format!(".opencrabs_plan_{}.json", session_id);
            let plan_file = self.working_directory.join(&plan_filename);

            if let Err(e) = self.plan_service.export_to_json(plan, &plan_file).await {
                tracing::warn!("Failed to save plan JSON backup: {}", e);
            }
        }
        Ok(())
    }

    /// Execute plan tasks sequentially
    async fn execute_plan_tasks(&mut self) -> Result<()> {
        self.executing_plan = true;
        self.execute_next_plan_task().await
    }

    /// Execute the next pending task in the plan
    async fn execute_next_plan_task(&mut self) -> Result<()> {
        // Collect necessary data from plan first to avoid borrow issues
        let (task_message, completion_data) = {
            let Some(plan) = &mut self.current_plan else {
                self.executing_plan = false;
                return Ok(());
            };

            // Get tasks in dependency order
            let Some(ordered_tasks) = plan.tasks_in_order() else {
                self.executing_plan = false;
                self.show_error(
                    "❌ Cannot Execute Plan\n\n\
                     Circular dependency detected in task graph. Tasks cannot be ordered \
                     because they form a dependency cycle.\n\n\
                     💡 Fix: Review task dependencies and remove circular references.\n\
                     You can reject this plan (Ctrl+R) and ask the AI to revise it."
                        .to_string(),
                );
                return Ok(());
            };

            // Find the next pending task and extract its data
            let next_task_data = ordered_tasks
                .iter()
                .find(|task| matches!(task.status, crate::tui::plan::TaskStatus::Pending))
                .map(|task| {
                    (
                        task.id,
                        task.order,
                        task.title.clone(),
                        task.description.clone(),
                    )
                });

            let total_tasks = plan.tasks.len();

            // Drop the immutable borrow of ordered_tasks
            drop(ordered_tasks);

            match next_task_data {
                Some((task_id, order, title, description)) => {
                    // Mark task as in progress
                    if let Some(task_mut) = plan.tasks.iter_mut().find(|t| t.id == task_id) {
                        task_mut.status = crate::tui::plan::TaskStatus::InProgress;
                    }

                    // Prepare task message
                    let message = format!(
                        "📋 Executing Plan Task #{}/{}\n\n\
                         **{}**\n\n\
                         {}\n\n\
                         Please complete this task.",
                        order, total_tasks, title, description
                    );

                    (Some(message), None)
                }
                None => {
                    // No more pending tasks - plan is complete
                    let title = plan.title.clone();
                    let task_count = plan.tasks.len();
                    plan.complete();
                    self.executing_plan = false;

                    (None, Some((title, task_count)))
                }
            }
        };

        // Save plan after releasing borrow
        self.save_plan().await?;

        // Handle results
        if let Some((title, task_count)) = completion_data {
            // Add completion message
            let completion_msg = DisplayMessage {
                id: uuid::Uuid::new_v4(),
                role: "system".to_string(),
                content: format!(
                    "Plan '{}' completed successfully!\n\
                     All {} tasks have been executed.",
                    title, task_count
                ),
                timestamp: chrono::Utc::now(),
                token_count: None,
                cost: None,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: None,
                plan_approval: None,
            };
            self.messages.push(completion_msg);
        } else if let Some(message) = task_message {
            // Send task message to agent
            tracing::info!("Sending plan task to agent (is_processing={})", self.is_processing);
            self.send_message(message).await?;
            tracing::info!("Plan task sent (is_processing={})", self.is_processing);
        }

        Ok(())
    }

    /// Show an error message
    fn show_error(&mut self, error: String) {
        self.is_processing = false;
        self.streaming_response = None;
        self.cancel_token = None;
        // Deny any pending approvals so agent callbacks don't hang, then remove
        for msg in &mut self.messages {
            if let Some(ref mut approval) = msg.approval && approval.state == ApprovalState::Pending {
                let _ = approval.response_tx.send(ToolApprovalResponse {
                    request_id: approval.request_id,
                    approved: false,
                    reason: Some("Error occurred".to_string()),
                });
                approval.state = ApprovalState::Denied("Error occurred".to_string());
            }
        }
        self.messages.retain(|m| m.approval.is_none() || m.approval.as_ref().is_some_and(|a| a.state == ApprovalState::Pending));
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
    async fn switch_mode(&mut self, mode: AppMode) -> Result<()> {
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
        tracing::info!("[APPROVAL] handle_approval_requested called for tool='{}' auto_session={} auto_always={}",
            request.tool_name, self.approval_auto_session, self.approval_auto_always);
        // Deny and remove stale pending approvals from previous requests
        for msg in &mut self.messages {
            if let Some(ref mut approval) = msg.approval && approval.state == ApprovalState::Pending {
                let _ = approval.response_tx.send(ToolApprovalResponse {
                    request_id: approval.request_id,
                    approved: false,
                    reason: Some("Superseded by new request".to_string()),
                });
                approval.state = ApprovalState::Denied("Superseded by new request".to_string());
            }
        }
        self.messages.retain(|m| {
            m.approval.as_ref().is_none_or(|a| !matches!(a.state, ApprovalState::Denied(_)))
        });

        // Auto-approve silently if policy allows
        if self.approval_auto_always || self.approval_auto_session {
            let response = ToolApprovalResponse {
                request_id: request.request_id,
                approved: true,
                reason: None,
            };
            let _ = request.response_tx.send(response.clone());
            let _ = self.event_sender().send(TuiEvent::ToolApprovalResponse(response));
            return;
        }

        // Clear streaming overlay so the approval dialog is visible
        if let Some(text) = self.streaming_response.take() && !text.trim().is_empty() {
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
        self.scroll_offset = 0;
        tracing::info!("[APPROVAL] Pushed approval message for tool='{}', total messages={}, has_pending={}",
            self.messages.last().map(|m| m.approval.as_ref().map(|a| a.tool_name.as_str()).unwrap_or("?")).unwrap_or("?"),
            self.messages.len(),
            self.has_pending_approval());
        // Stay in AppMode::Chat — no mode switch
    }

    /// Update slash command autocomplete suggestions (built-in + user-defined)
    fn update_slash_suggestions(&mut self) {
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
            let base = SLASH_COMMANDS.len();
            for (i, ucmd) in self.user_commands.iter().enumerate() {
                if ucmd.name.to_lowercase().starts_with(&prefix) {
                    self.slash_filtered.push(base + i);
                }
            }

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
    fn reload_user_commands(&mut self) {
        let command_loader = CommandLoader::from_brain_path(&self.brain_path);
        self.user_commands = command_loader.load();
    }

    /// Open the model selector dialog
    fn open_model_selector(&mut self) {
        self.model_selector_models = self.agent_service.supported_models();
        let current = self
            .current_session
            .as_ref()
            .and_then(|s| s.model.as_deref())
            .unwrap_or_else(|| self.agent_service.provider_model())
            .to_string();

        // Pre-select the current model
        self.model_selector_selected = self
            .model_selector_models
            .iter()
            .position(|m| m == &current)
            .unwrap_or(0);

        self.mode = AppMode::ModelSelector;
    }

    /// Handle keys in model selector mode
    async fn handle_model_selector_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;

        if keys::is_cancel(&event) {
            self.switch_mode(AppMode::Chat).await?;
        } else if keys::is_up(&event) {
            self.model_selector_selected = self.model_selector_selected.saturating_sub(1);
        } else if keys::is_down(&event) {
            if !self.model_selector_models.is_empty() {
                self.model_selector_selected = (self.model_selector_selected + 1)
                    .min(self.model_selector_models.len() - 1);
            }
        } else if keys::is_enter(&event)
            && let Some(model) = self.model_selector_models.get(self.model_selector_selected) {
                let model_name = model.clone();
                // Update session model
                if let Some(session) = &mut self.current_session {
                    session.model = Some(model_name.clone());
                    if let Err(e) = self.session_service.update_session(session).await {
                        tracing::warn!("Failed to update session model: {}", e);
                    }
                }
                self.push_system_message(format!("Model changed to: {}", model_name));
                self.mode = AppMode::Chat;
            }

        Ok(())
    }

    /// Handle keys in onboarding wizard mode
    async fn handle_onboarding_key(&mut self, event: crossterm::event::KeyEvent) -> Result<()> {
        if let Some(ref mut wizard) = self.onboarding {
            let action = wizard.handle_key(event);
            match action {
                WizardAction::Cancel => {
                    self.onboarding = None;
                    self.switch_mode(AppMode::Chat).await?;
                }
                WizardAction::Complete => {
                    // Apply wizard config before transitioning
                    if let Some(ref wizard) = self.onboarding {
                        match wizard.apply_config() {
                            Ok(()) => {
                                self.push_system_message(
                                    "Setup complete! OpenCrabs is configured and ready."
                                        .to_string(),
                                );
                            }
                            Err(e) => {
                                self.push_system_message(format!(
                                    "Setup finished with warnings: {}",
                                    e
                                ));
                            }
                        }
                    }
                    self.onboarding = None;
                    self.switch_mode(AppMode::Chat).await?;
                }
                WizardAction::GenerateBrain => {
                    self.generate_brain_files().await;
                }
                WizardAction::None => {
                    // Stay in onboarding
                }
            }
        }
        Ok(())
    }

    /// Generate personalized brain files via the AI provider
    async fn generate_brain_files(&mut self) {
        // Extract what we need before borrowing wizard mutably
        let prompt = {
            let Some(ref wizard) = self.onboarding else { return };
            wizard.build_brain_prompt()
        };

        // Mark as generating
        if let Some(ref mut wizard) = self.onboarding {
            wizard.brain_generating = true;
            wizard.brain_error = None;
        }

        // Get provider and model from the wizard's selected provider
        let provider = self.agent_service.provider().clone();
        let model = self.agent_service.provider_model().to_string();

        // Build LLM request
        let request = LLMRequest::new(
            model,
            vec![crate::llm::provider::Message::user(prompt)],
        )
        .with_max_tokens(65536);

        // Call the provider
        match provider.complete(request).await {
            Ok(response) => {
                // Extract text from response
                let text: String = response
                    .content
                    .iter()
                    .filter_map(|block| {
                        if let ContentBlock::Text { text } = block {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();

                if let Some(ref mut wizard) = self.onboarding {
                    wizard.apply_generated_brain(&text);
                    // Auto-advance to Complete if generation succeeded
                    if wizard.brain_generated {
                        wizard.step = super::onboarding::OnboardingStep::Complete;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Brain generation failed: {}", e);
                if let Some(ref mut wizard) = self.onboarding {
                    wizard.brain_generating = false;
                    wizard.brain_error = Some(format!("Generation failed: {}", e));
                }
            }
        }
    }

    /// Open file picker and populate file list
    async fn open_file_picker(&mut self) -> Result<()> {
        // Get list of files in current directory
        let mut files = Vec::new();

        // Add parent directory option if not at root
        if self.file_picker_current_dir.parent().is_some() {
            files.push(self.file_picker_current_dir.join(".."));
        }

        // Read directory entries
        if let Ok(entries) = std::fs::read_dir(&self.file_picker_current_dir) {
            for entry in entries.flatten() {
                files.push(entry.path());
            }
        }

        // Sort: directories first, then files, alphabetically
        files.sort_by(|a, b| {
            let a_is_dir = a.is_dir();
            let b_is_dir = b.is_dir();
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.file_name().cmp(&b.file_name()),
            }
        });

        self.file_picker_files = files;
        self.file_picker_selected = 0;
        self.file_picker_scroll_offset = 0;
        self.switch_mode(AppMode::FilePicker).await?;

        Ok(())
    }

    /// Handle keys in file picker mode
    async fn handle_file_picker_key(&mut self, event: crossterm::event::KeyEvent) -> Result<()> {
        use super::events::keys;
        use crossterm::event::KeyCode;

        if keys::is_cancel(&event) {
            // Cancel file picker and return to chat
            self.switch_mode(AppMode::Chat).await?;
        } else if keys::is_up(&event) {
            // Move selection up
            self.file_picker_selected = self.file_picker_selected.saturating_sub(1);

            // Adjust scroll offset if needed
            if self.file_picker_selected < self.file_picker_scroll_offset {
                self.file_picker_scroll_offset = self.file_picker_selected;
            }
        } else if keys::is_down(&event) {
            // Move selection down
            if self.file_picker_selected + 1 < self.file_picker_files.len() {
                self.file_picker_selected += 1;

                // Adjust scroll offset if needed (assuming 20 visible items)
                let visible_items = 20;
                if self.file_picker_selected >= self.file_picker_scroll_offset + visible_items {
                    self.file_picker_scroll_offset = self.file_picker_selected - visible_items + 1;
                }
            }
        } else if keys::is_enter(&event) || event.code == KeyCode::Char(' ') {
            // Select file or navigate into directory
            if let Some(selected_path) = self.file_picker_files.get(self.file_picker_selected) {
                if selected_path.is_dir() {
                    // Navigate into directory
                    if selected_path.ends_with("..") {
                        // Go to parent directory
                        if let Some(parent) = self.file_picker_current_dir.parent() {
                            self.file_picker_current_dir = parent.to_path_buf();
                        }
                    } else {
                        self.file_picker_current_dir = selected_path.clone();
                    }
                    // Refresh file list
                    self.open_file_picker().await?;
                } else {
                    // Insert file path into input buffer at cursor
                    let path_str = selected_path.to_string_lossy().to_string();
                    self.input_buffer.insert_str(self.cursor_position, &path_str);
                    self.cursor_position += path_str.len();
                    self.switch_mode(AppMode::Chat).await?;
                }
            }
        }

        Ok(())
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
