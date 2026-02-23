//! App Module — TUI application state and logic.

mod state;
mod input;
mod messaging;
mod plan_exec;
mod dialogs;

pub use state::*;

// Re-export sibling modules so sub-modules can use `super::events`, etc.
pub(crate) use super::events;
pub(crate) use super::onboarding;
pub(crate) use super::plan;
pub(crate) use super::prompt_analyzer;
