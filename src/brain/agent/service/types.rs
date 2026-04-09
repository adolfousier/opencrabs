use crate::brain::provider::{ProviderStream, StopReason};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

use super::builder::AgentService;

/// Result type alias used by approval/sudo callbacks
pub(super) type Result<T> = super::super::error::Result<T>;

/// Tool approval request information
#[derive(Debug, Clone)]
pub struct ToolApprovalInfo {
    /// Session this tool call belongs to
    pub session_id: Uuid,
    /// Tool name
    pub tool_name: String,
    /// Tool description
    pub tool_description: String,
    /// Tool input parameters
    pub tool_input: Value,
    /// Tool capabilities
    pub capabilities: Vec<String>,
}

/// Type alias for approval callback function.
/// Returns `(approved, always_approve)`:
/// - `approved`: whether this tool call is allowed
/// - `always_approve`: if true, skip approval for all subsequent tools in this loop
pub type ApprovalCallback = Arc<
    dyn Fn(ToolApprovalInfo) -> Pin<Box<dyn Future<Output = Result<(bool, bool)>> + Send>>
        + Send
        + Sync,
>;

/// Progress event emitted during tool execution
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Thinking,
    ToolStarted {
        tool_name: String,
        tool_input: Value,
    },
    ToolCompleted {
        tool_name: String,
        tool_input: Value,
        success: bool,
        summary: String,
    },
    /// Intermediate text the agent sends between tool call batches
    IntermediateText {
        text: String,
        reasoning: Option<String>,
    },
    /// A queued user message was injected between tool iterations
    QueuedUserMessage {
        text: String,
    },
    /// Real-time streaming chunk from the LLM (word-by-word)
    StreamingChunk {
        text: String,
    },
    Compacting,
    /// Compaction finished — carry the summary so the TUI can display it
    CompactionSummary {
        summary: String,
    },
    /// A single build-output line (e.g. "Compiling foo v1.0"). The TUI keeps a
    /// rolling window of the last few lines and clears them on RestartReady.
    BuildLine(String),
    /// Build completed — TUI should offer restart
    RestartReady {
        status: String,
    },
    /// Real-time token count update — fire after every API response and tool execution
    TokenCount(usize),
    /// Reasoning/thinking content from providers like MiniMax (display-only)
    ReasoningChunk {
        text: String,
    },
    /// Self-healing action was taken (config recovery, emergency compaction, truncation, etc.)
    SelfHealingAlert {
        message: String,
    },
    /// The just-streamed assistant text has been detected as a gaslighting
    /// refusal preamble (e.g. "tools aren't responding") emitted alongside a
    /// valid tool_use block. The UI should wipe its in-progress streaming
    /// buffer so the lie doesn't stay on screen.
    StripStreamedContent {
        /// Number of bytes to strip from the START of the streaming buffer.
        /// The gaslighting preamble is always leading, so consumers should
        /// drain exactly this many bytes (at a char boundary) rather than
        /// wiping the whole buffer — otherwise any legitimate draft that
        /// followed the preamble in the same text block is destroyed.
        bytes: usize,
        reason: String,
    },
    /// Sticky fallback promoted a new provider/model. Carries structured data
    /// so UIs can update the session + footer without parsing text.
    ProviderSwitched {
        from_name: String,
        from_model: String,
        to_name: String,
        to_model: String,
        reason: String,
    },
}

/// Callback for reporting progress during agent execution.
/// The first parameter is the `session_id` the event belongs to.
pub type ProgressCallback = Arc<dyn Fn(Uuid, ProgressEvent) + Send + Sync>;

/// Callback for requesting sudo password from the user.
/// Takes the command string, returns Ok(Some(password)) or Ok(None) if cancelled.
pub type SudoCallback = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send>> + Send + Sync,
>;

/// Callback for checking if a user message has been queued during tool execution.
/// Returns Some(message) if a message is waiting, None otherwise. Must not block.
pub type MessageQueueCallback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

/// Response from the agent
#[derive(Debug, Clone)]
pub struct AgentResponse {
    /// Message ID in database
    pub message_id: Uuid,

    /// Response content
    pub content: String,

    /// Stop reason
    pub stop_reason: Option<StopReason>,

    /// Token usage (accumulated across all tool-loop iterations — for billing)
    pub usage: crate::brain::provider::TokenUsage,

    /// Actual context window usage from the last API call (for display)
    pub context_tokens: u32,

    /// Cost in USD
    pub cost: f64,

    /// Model used
    pub model: String,
}

/// Streaming response from the agent
pub struct AgentStreamResponse {
    /// Session ID
    pub session_id: Uuid,

    /// Message ID that will be created
    pub message_id: Uuid,

    /// Stream of events
    pub stream: ProviderStream,

    /// Model being used
    pub model: String,
}

// Make AgentService's extract_text_from_response available to types that need it
impl AgentService {
    /// Extract text content from an LLM response (text blocks only — tool calls
    /// are displayed via the tool group UI, not as raw text).
    pub(super) fn extract_text_from_response(
        response: &crate::brain::provider::LLMResponse,
    ) -> String {
        let mut text = String::new();

        for content in &response.content {
            if let crate::brain::provider::ContentBlock::Text { text: t } = content
                && !t.trim().is_empty()
            {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(t);
            }
        }

        text
    }
}
