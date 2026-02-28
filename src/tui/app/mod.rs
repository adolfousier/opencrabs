//! App Module — TUI application state and logic.

mod dialogs;
mod input;
mod messaging;
mod state;

pub use state::*;

// Re-export sibling modules so sub-modules can use `super::events`, etc.
pub(crate) use super::events;
pub(crate) use super::onboarding;
pub(crate) use super::prompt_analyzer;
