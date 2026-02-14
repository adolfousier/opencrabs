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
pub mod context;
pub mod http;
pub mod plan_tool;
pub mod task;

// Re-exports
pub use error::{Result, ToolError};
pub use r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
pub use registry::ToolRegistry;
