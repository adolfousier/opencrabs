//! Agent Service Implementation
//!
//! Core service for managing AI agent conversations, coordinating between
//! LLM providers, context management, and data persistence.

mod builder;
mod context;
mod helpers;
mod messaging;
pub(crate) mod tool_loop;
mod types;

#[cfg(test)]
mod tests;

pub use builder::AgentService;
pub use helpers::{
    detect_text_repetition, has_investigative_intent, has_phantom_tool_intent,
    has_phantom_tool_intent_no_tools, is_gaslighting_preamble, looks_truncated_mid_sentence,
    strip_gaslighting_preamble,
};
pub use types::{
    AgentResponse, AgentStreamResponse, ApprovalCallback, ChannelSessionEvent,
    MessageQueueCallback, ProgressCallback, ProgressEvent, SudoCallback, ToolApprovalInfo,
};
