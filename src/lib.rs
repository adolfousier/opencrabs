//! # OpenCrabs
//!
//! The autonomous AI agent. Single Rust binary. Every channel.
//!
//! OpenCrabs is a high-performance AI orchestration agent with a modern TUI,
//! multi-channel messaging (Telegram, Discord, Slack, WhatsApp), and an
//! extensible tool system — all in one statically-linked binary.
//!
//! ## Providers
//!
//! - **Anthropic** — Claude models (Sonnet, Opus, Haiku)
//! - **OpenAI** — GPT-4, GPT-5, o-series models
//! - **Google Gemini** — Gemini 2.x family
//! - **OpenRouter** — 300+ models via single API key
//! - **MiniMax** — MiniMax-M2.5, Kimi, and Text models
//! - **Custom** — any OpenAI-compatible endpoint (Ollama, LM Studio, vLLM, etc.)
//!
//! ## Features
//!
//! - **Modern TUI** — Ratatui-based terminal interface with streaming, markdown, and syntax highlighting
//! - **Multi-Channel** — Telegram, Discord, Slack, and WhatsApp gateways with shared sessions
//! - **Tool System** — 30+ built-in tools (shell, file I/O, web search, code execution, etc.)
//! - **A2A Protocol** — Agent-to-Agent communication for multi-agent workflows
//! - **Local-First** — SQLite storage, config hot-reload, no cloud dependency
//! - **Session Management** — persistent chat sessions with token/cost tracking
//! - **Cron Scheduler** — scheduled agent tasks with natural-language definitions
//!
//! ## Quick Start
//!
//! ```bash
//! # Interactive TUI mode
//! opencrabs
//!
//! # Non-interactive (pipe-friendly)
//! opencrabs run "explain this code"
//!
//! # With auto-approve
//! opencrabs run --auto-approve "refactor this file"
//! ```
//!
//! ## Architecture
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`brain`] | LLM providers, tool registry, agent service |
//! | [`channels`] | Telegram, Discord, Slack, WhatsApp handlers |
//! | [`tui`] | Terminal UI (Ratatui + crossterm) |
//! | [`db`] | SQLite persistence (deadpool-sqlite) |
//! | [`config`] | TOML config with hot-reload and key separation |
//! | [`a2a`] | Agent-to-Agent protocol server |
//! | [`cron`] | Scheduled task execution |
//! | [`services`] | Session, message, and file services |

pub mod app;
pub mod brain;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod logging;
pub mod memory;
pub mod services;
pub mod tui;
pub mod utils;

pub mod a2a;
pub mod channels;
pub mod cron;
pub mod pricing;

// Re-export commonly used types
pub use error::{ErrorCode, OpenCrabsError};

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Package authors
pub const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
/// Package description
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

#[cfg(test)]
mod tests;
