//! Tool Execution Framework
//!
//! Provides an abstraction for tools that can be called by LLM agents,
//! including file operations, shell commands, and more.

pub mod error;
pub mod registry;
mod r#trait;

// Tool implementations - Phase 1: Essential File Operations
pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod ls;
pub mod read;
pub mod write;

// Tool implementations - Phase 2: Advanced Features
pub mod brave_search;
pub mod code_exec;
pub mod doc_parser;
pub mod exa_search;
pub mod notebook;
pub mod web_search;

// Tool implementations - Phase 3: Workflow & Integration
pub mod config_tool;
pub mod context;
pub mod http;
pub mod memory_search;
pub mod plan_tool;
pub mod rebuild;
pub mod session_search;
pub mod slash_command;
pub mod task;

// Tool implementations - Phase 4: Channel Integrations
#[cfg(feature = "discord")]
pub mod discord_connect;
#[cfg(feature = "discord")]
pub mod discord_send;
#[cfg(feature = "slack")]
pub mod slack_connect;
#[cfg(feature = "slack")]
pub mod slack_send;
#[cfg(feature = "telegram")]
pub mod telegram_connect;
#[cfg(feature = "telegram")]
pub mod telegram_send;
#[cfg(feature = "trello")]
pub mod trello_connect;
#[cfg(feature = "trello")]
pub mod trello_send;
#[cfg(feature = "whatsapp")]
pub mod whatsapp_connect;
#[cfg(feature = "whatsapp")]
pub mod whatsapp_send;

// Re-exports
pub use error::{Result, ToolError};
pub use registry::ToolRegistry;
pub use r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
