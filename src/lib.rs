//! OpenCrabs - High-Performance Terminal AI Orchestration Agent
//!
//! A blazingly fast, memory-efficient terminal-based AI orchestration agent for software development.
//! Written in Rust for superior performance, memory safety, and reduced resource consumption.
//!
//! ## Features
//!
//! - **Multi-LLM Support:** Anthropic, OpenAI, Google Gemini, AWS Bedrock, Azure, VertexAI
//! - **LSP Integration:** Semantic code understanding via Language Server Protocol
//! - **Local-First:** SQLite storage for privacy and offline functionality
//! - **Modern TUI:** Built with Ratatui for responsive terminal interface
//! - **Tool System:** Extensible tools for file operations, shell commands, and more
//! - **Context Files:** Automatic loading of .cursorrules, .claudemd files
//! - **Session Management:** Persistent chat sessions with token/cost tracking
//!
//! ## Quick Start
//!
//! ```bash
//! # Interactive mode
//! opencrabs
//!
//! # Non-interactive mode
//! opencrabs run "explain this code"
//!
//! # With auto-approve (dangerous!)
//! opencrabs run --auto-approve "refactor this file"
//! ```

pub mod app;
pub mod brain;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod llm;
pub mod logging;
pub mod services;
pub mod tui;
pub mod utils;

// Placeholder modules for future features
// TODO: Implement these modules when ready
#[allow(unused)]
pub mod events;
#[allow(unused)]
pub mod lsp;
#[allow(unused)]
pub mod message;
#[allow(unused)]
pub mod sync;

// Re-export commonly used types
pub use error::{OpenCrabsError, ErrorCode};

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
