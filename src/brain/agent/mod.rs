//! Agent Service Module
//!
//! Provides high-level agent functionality for managing conversations,
//! executing tools, and coordinating with LLM providers.

pub mod context;
pub mod error;
pub mod service;

// Re-exports
pub use context::AgentContext;
pub use error::{AgentError, Result};
pub use service::{
    AgentResponse, AgentService, AgentStreamResponse, ApprovalCallback, ChannelSessionEvent,
    MessageQueueCallback, ProgressCallback, ProgressEvent, SudoCallback, ToolApprovalInfo,
};
