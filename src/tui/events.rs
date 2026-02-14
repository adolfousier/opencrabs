//! TUI Event System
//!
//! Handles user input and application events for the terminal interface.

use crate::llm::agent::AgentResponse;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::time::Duration;
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
    ToolCallStarted { tool_name: String, tool_input: Value },

    /// A tool call has completed
    ToolCallCompleted { tool_name: String, tool_input: Value, success: bool, summary: String },
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
    pub fn start_terminal_listener(tx: mpsc::UnboundedSender<TuiEvent>) {
        tokio::spawn(async move {
            loop {
                // Poll for crossterm events with timeout
                if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false)
                    && let Ok(event) = crossterm::event::read() {
                        match event {
                            crossterm::event::Event::Key(key) => {
                                // Only process key press events to avoid duplicates
                                // Ignore key release and repeat events
                                if key.kind == crossterm::event::KeyEventKind::Press
                                    && tx.send(TuiEvent::Key(key)).is_err()
                                {
                                    break;
                                }
                            }
                            crossterm::event::Event::Mouse(mouse) => {
                                use crossterm::event::MouseEventKind;
                                match mouse.kind {
                                    MouseEventKind::ScrollUp => {
                                        if tx.send(TuiEvent::MouseScroll(1)).is_err() {
                                            break;
                                        }
                                    }
                                    MouseEventKind::ScrollDown => {
                                        if tx.send(TuiEvent::MouseScroll(-1)).is_err() {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            crossterm::event::Event::Resize(w, h) => {
                                if tx.send(TuiEvent::Resize(w, h)).is_err() {
                                    break;
                                }
                            }
                            crossterm::event::Event::Paste(text) => {
                                if tx.send(TuiEvent::Paste(text)).is_err() {
                                    break;
                                }
                            }
                            crossterm::event::Event::FocusGained => {
                                if tx.send(TuiEvent::FocusGained).is_err() {
                                    break;
                                }
                            }
                            crossterm::event::Event::FocusLost => {
                                if tx.send(TuiEvent::FocusLost).is_err() {
                                    break;
                                }
                            }
                        }
                    }

                // Send tick event for animations
                if tx.send(TuiEvent::Tick).is_err() {
                    break;
                }

                // Small delay to prevent CPU spinning
                tokio::time::sleep(Duration::from_millis(50)).await;
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
            && (event.modifiers.is_empty()
                || event.modifiers.contains(KeyModifiers::CONTROL))
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
