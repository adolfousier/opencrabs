//! Terminal User Interface
//!
//! Provides an interactive terminal interface for the AI orchestration agent using Ratatui.

pub mod app;
pub(crate) mod compact_notice;
pub mod error;
pub mod events;
pub(crate) mod model_order;
pub mod onboarding;
pub mod onboarding_render;
pub mod pane;
pub mod plan;
pub mod provider_selector;
pub mod remote_upload;
pub mod render;
pub mod runner;
pub mod theme_catalog;

// Enhanced rendering modules
pub mod highlight;
pub mod markdown;

pub mod components;

// Re-exports
pub use app::{App, DisplayMessage};
pub use events::{AppMode, EventHandler, TuiEvent};
pub use runner::run;
