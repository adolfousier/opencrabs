//! TUI Event System
//!
//! Handles user input and application events for the terminal interface.

use crate::brain::agent::AgentResponse;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Events that can occur in the TUI
#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// User pressed a key
    Key(KeyEvent),

    /// Mouse scroll event
    MouseScroll(i8), // positive = up, negative = down

    /// Terminal gained focus
    FocusGained,

    /// Terminal lost focus
    FocusLost,

    /// User pasted text
    Paste(String),

    /// Terminal was resized
    Resize(u16, u16),

    /// User submitted a message
    MessageSubmitted(String),

    /// Agent started processing
    AgentProcessing,

    /// Agent sent a response chunk (streaming)
    ResponseChunk(String),

    /// Agent completed response
    ResponseComplete(AgentResponse),

    /// An error occurred
    Error(String),

    /// Request to switch UI mode
    SwitchMode(AppMode),

    /// Request to select a session
    SelectSession(Uuid),

    /// Request to create new session
    NewSession,

    /// Request to quit
    Quit,

    /// Tick event for animations/updates
    Tick,

    /// Tool approval requested
    ToolApprovalRequested(ToolApprovalRequest),

    /// Tool approval response
    ToolApprovalResponse(ToolApprovalResponse),

    /// A tool call has started executing
    ToolCallStarted {
        tool_name: String,
        tool_input: Value,
    },

    /// A tool call has completed
    ToolCallCompleted {
        tool_name: String,
        tool_input: Value,
        success: bool,
        summary: String,
    },

    /// Intermediate text the agent sent between tool call batches
    IntermediateText(String, Option<String>),

    /// Context was auto-compacted — show the summary to the user
    CompactionSummary(String),

    /// Build completed — offer restart to the user
    RestartReady(String),

    /// Configuration was reloaded (e.g. after config_tool write)
    ConfigReloaded,

    /// Real-time token count update from the agent loop
    TokenCountUpdated(usize),

    /// Onboarding wizard received fetched model list from provider API
    OnboardingModelsFetched(Vec<String>),

    /// Model selector (/models) received fetched model list after paste
    ModelSelectorModelsFetched(Vec<String>),

    /// WhatsApp QR code data received during onboarding pairing
    WhatsAppQrCode(String),
    /// WhatsApp pairing successful during onboarding
    WhatsAppConnected,
    /// WhatsApp pairing failed during onboarding
    WhatsAppError(String),

    /// A system message to display in chat
    SystemMessage(String),

    /// Channel test message result during onboarding
    ChannelTestResult {
        channel: String,
        success: bool,
        error: Option<String>,
    },

    /// Sudo password requested by bash tool
    SudoPasswordRequested(SudoPasswordRequest),

    /// Reasoning/thinking content chunk from providers like MiniMax (display-only)
    ReasoningChunk(String),
}

/// Sudo password request from the bash tool
#[derive(Debug)]
pub struct SudoPasswordRequest {
    /// Unique ID for this request
    pub request_id: Uuid,
    /// The sudo command being run
    pub command: String,
    /// Channel to send password back
    pub response_tx: mpsc::UnboundedSender<SudoPasswordResponse>,
}

// Manual Clone — response_tx is Clone-able (UnboundedSender)
impl Clone for SudoPasswordRequest {
    fn clone(&self) -> Self {
        Self {
            request_id: self.request_id,
            command: self.command.clone(),
            response_tx: self.response_tx.clone(),
        }
    }
}

/// Sudo password response from the TUI
#[derive(Debug, Clone)]
pub struct SudoPasswordResponse {
    /// The password (None if cancelled by user)
    pub password: Option<String>,
}

/// Tool approval request details
#[derive(Debug, Clone)]
pub struct ToolApprovalRequest {
    /// Unique ID for this approval request
    pub request_id: Uuid,

    /// Tool name
    pub tool_name: String,

    /// Tool description
    pub tool_description: String,

    /// Tool input parameters
    pub tool_input: Value,

    /// Tool capabilities
    pub capabilities: Vec<String>,

    /// Channel to send response back
    pub response_tx: mpsc::UnboundedSender<ToolApprovalResponse>,

    /// When this request was created (for timeout)
    pub requested_at: std::time::Instant,
}

impl ToolApprovalRequest {
    /// How long this request has been waiting
    pub fn elapsed(&self) -> std::time::Duration {
        self.requested_at.elapsed()
    }
}

/// Tool approval response
#[derive(Debug, Clone)]
pub struct ToolApprovalResponse {
    /// Request ID this is responding to
    pub request_id: Uuid,

    /// Whether the user approved
    pub approved: bool,

    /// Optional reason for denial
    pub reason: Option<String>,
}

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Splash screen
    Splash,
    /// Main chat interface (full execution)
    Chat,
    /// Plan mode (read-only, planning phase)
    Plan,
    /// Session list/management
    Sessions,
    /// Help screen
    Help,
    /// Settings
    Settings,
    /// File picker dialog (triggered by @)
    FilePicker,
    /// Model selector dialog (triggered by /models)
    ModelSelector,
    /// Usage stats dialog (triggered by /usage)
    UsageDialog,
    /// Restart confirmation pending (after successful /rebuild)
    RestartPending,
    /// Directory picker dialog (triggered by /cd)
    DirectoryPicker,
    /// Onboarding wizard
    Onboarding,
}

/// Event handler for the TUI
pub struct EventHandler {
    /// Event sender
    tx: mpsc::UnboundedSender<TuiEvent>,

    /// Event receiver
    rx: mpsc::UnboundedReceiver<TuiEvent>,
}

impl EventHandler {
    /// Create a new event handler
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx }
    }

    /// Get a sender for sending events
    pub fn sender(&self) -> mpsc::UnboundedSender<TuiEvent> {
        self.tx.clone()
    }

    /// Receive the next event (blocks until available)
    pub async fn next(&mut self) -> Option<TuiEvent> {
        self.rx.recv().await
    }

    /// Try to receive the next event without blocking
    pub fn try_next(&mut self) -> Option<TuiEvent> {
        self.rx.try_recv().ok()
    }

    /// Start listening for terminal events
    ///
    /// Uses crossterm's async EventStream instead of blocking poll/read
    /// to avoid starving the tokio runtime during I/O-heavy operations
    /// (e.g. Telegram voice processing, agent responses).
    pub fn start_terminal_listener(tx: mpsc::UnboundedSender<TuiEvent>) {
        use crossterm::event::EventStream;
        use futures::StreamExt;

        tokio::spawn(async move {
            let mut reader = EventStream::new();
            let tick_interval = std::time::Duration::from_millis(100);

            loop {
                // Race: next terminal event vs tick timer
                let event = tokio::select! {
                    maybe_event = reader.next() => {
                        match maybe_event {
                            Some(Ok(event)) => Some(event),
                            Some(Err(_)) => None,
                            None => break, // Stream closed
                        }
                    }
                    _ = tokio::time::sleep(tick_interval) => None,
                };

                if let Some(event) = event {
                    let should_break = match event {
                        crossterm::event::Event::Key(key) => {
                            // Only process key press events to avoid duplicates
                            if key.kind == crossterm::event::KeyEventKind::Press {
                                tx.send(TuiEvent::Key(key)).is_err()
                            } else {
                                false
                            }
                        }
                        crossterm::event::Event::Mouse(mouse) => {
                            use crossterm::event::MouseEventKind;
                            match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    tx.send(TuiEvent::MouseScroll(1)).is_err()
                                }
                                MouseEventKind::ScrollDown => {
                                    tx.send(TuiEvent::MouseScroll(-1)).is_err()
                                }
                                _ => false,
                            }
                        }
                        crossterm::event::Event::Resize(w, h) => {
                            tx.send(TuiEvent::Resize(w, h)).is_err()
                        }
                        crossterm::event::Event::Paste(text) => {
                            tx.send(TuiEvent::Paste(text)).is_err()
                        }
                        crossterm::event::Event::FocusGained => {
                            tx.send(TuiEvent::FocusGained).is_err()
                        }
                        crossterm::event::Event::FocusLost => tx.send(TuiEvent::FocusLost).is_err(),
                    };
                    if should_break {
                        break;
                    }
                }

                // Send tick event for animations
                if tx.send(TuiEvent::Tick).is_err() {
                    break;
                }
            }
        });
    }
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to check if a key event matches
pub fn key_matches(event: &KeyEvent, code: KeyCode, modifiers: KeyModifiers) -> bool {
    event.code == code && event.modifiers == modifiers
}

/// Common key bindings
pub mod keys {
    use super::*;

    /// Ctrl+C - Quit
    pub fn is_quit(event: &KeyEvent) -> bool {
        key_matches(event, KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    /// Ctrl+N - New session
    pub fn is_new_session(event: &KeyEvent) -> bool {
        key_matches(event, KeyCode::Char('n'), KeyModifiers::CONTROL)
    }

    /// Ctrl+L - List sessions
    pub fn is_list_sessions(event: &KeyEvent) -> bool {
        key_matches(event, KeyCode::Char('l'), KeyModifiers::CONTROL)
    }

    /// Ctrl+K - Clear current session
    pub fn is_clear_session(event: &KeyEvent) -> bool {
        key_matches(event, KeyCode::Char('k'), KeyModifiers::CONTROL)
    }

    /// Ctrl+P - Toggle Plan mode
    pub fn is_toggle_plan(event: &KeyEvent) -> bool {
        key_matches(event, KeyCode::Char('p'), KeyModifiers::CONTROL)
    }

    /// Enter - Submit (plain Enter sends the message)
    /// Also accepts Ctrl+Enter for backwards compatibility
    pub fn is_submit(event: &KeyEvent) -> bool {
        event.code == KeyCode::Enter
            && (event.modifiers.is_empty() || event.modifiers.contains(KeyModifiers::CONTROL))
    }

    /// Alt+Enter or Shift+Enter - Insert newline
    pub fn is_newline(event: &KeyEvent) -> bool {
        event.code == KeyCode::Enter
            && (event.modifiers.contains(KeyModifiers::ALT)
                || event.modifiers.contains(KeyModifiers::SHIFT))
    }

    /// Escape - Cancel/Back
    pub fn is_cancel(event: &KeyEvent) -> bool {
        event.code == KeyCode::Esc
    }

    /// Enter - Select/Confirm
    pub fn is_enter(event: &KeyEvent) -> bool {
        event.code == KeyCode::Enter && event.modifiers.is_empty()
    }

    /// Up arrow
    pub fn is_up(event: &KeyEvent) -> bool {
        event.code == KeyCode::Up && event.modifiers.is_empty()
    }

    /// Down arrow
    pub fn is_down(event: &KeyEvent) -> bool {
        event.code == KeyCode::Down && event.modifiers.is_empty()
    }

    /// Left arrow
    pub fn is_left(event: &KeyEvent) -> bool {
        event.code == KeyCode::Left && event.modifiers.is_empty()
    }

    /// Right arrow
    pub fn is_right(event: &KeyEvent) -> bool {
        event.code == KeyCode::Right && event.modifiers.is_empty()
    }

    /// Page up
    pub fn is_page_up(event: &KeyEvent) -> bool {
        event.code == KeyCode::PageUp
    }

    /// Page down
    pub fn is_page_down(event: &KeyEvent) -> bool {
        event.code == KeyCode::PageDown
    }

    /// Tab - Select/Navigate
    pub fn is_tab(event: &KeyEvent) -> bool {
        event.code == KeyCode::Tab && event.modifiers.is_empty()
    }

    /// 'A' or 'Y' - Approve
    pub fn is_approve(event: &KeyEvent) -> bool {
        matches!(
            event.code,
            KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('y') | KeyCode::Char('Y')
        ) && event.modifiers.is_empty()
    }

    /// 'D' or 'N' - Deny
    pub fn is_deny(event: &KeyEvent) -> bool {
        matches!(
            event.code,
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Char('n') | KeyCode::Char('N')
        ) && event.modifiers.is_empty()
    }

    /// 'V' - View details
    pub fn is_view_details(event: &KeyEvent) -> bool {
        matches!(event.code, KeyCode::Char('v') | KeyCode::Char('V')) && event.modifiers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_handler_creation() {
        let handler = EventHandler::new();
        let sender = handler.sender();
        // Should be able to send events
        assert!(sender.send(TuiEvent::Quit).is_ok());
    }

    #[test]
    fn test_key_matches() {
        let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(key_matches(
            &event,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        ));
        assert!(!key_matches(
            &event,
            KeyCode::Char('c'),
            KeyModifiers::empty()
        ));
    }

    #[test]
    fn test_quit_key() {
        let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(keys::is_quit(&event));

        let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::empty());
        assert!(!keys::is_quit(&event));
    }

    #[test]
    fn test_submit_key() {
        // Plain Enter sends
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        assert!(keys::is_submit(&event));

        // Ctrl+Enter also sends (backwards compat)
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(keys::is_submit(&event));

        // Alt+Enter does NOT send (it inserts newline)
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
        assert!(!keys::is_submit(&event));
        assert!(keys::is_newline(&event));
    }
}
