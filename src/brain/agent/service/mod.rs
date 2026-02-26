//! Agent Service Implementation
//!
//! Core service for managing AI agent conversations, coordinating between
//! LLM providers, context management, and data persistence.

mod builder;
mod context;
mod helpers;
mod messaging;
mod tool_loop;
mod types;

#[cfg(test)]
mod tests;

pub use builder::AgentService;
pub use types::{
    AgentResponse, AgentStreamResponse, ApprovalCallback, MessageQueueCallback, ProgressCallback,
    ProgressEvent, SudoCallback, ToolApprovalInfo,
};
