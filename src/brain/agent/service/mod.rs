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
pub use helpers::{detect_text_repetition, is_gaslighting_preamble, strip_gaslighting_preamble};
pub use types::{
    AgentResponse, AgentStreamResponse, ApprovalCallback, ChannelSessionEvent,
    MessageQueueCallback, ProgressCallback, ProgressEvent, SudoCallback, ToolApprovalInfo,
};
