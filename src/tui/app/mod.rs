//! App Module — TUI application state and logic.

pub(crate) mod background_session;
mod dialogs;
pub(crate) mod dropped_path;
pub(crate) mod duplicate_submit;
pub(crate) mod help_catalog;
pub(crate) mod input;
pub(crate) mod messaging;
pub mod mission_control;
pub mod profiles_dialog;
pub(crate) mod reasoning_split;
pub mod skills_dialog;
pub(crate) mod state;

pub use background_session::{BackgroundSessionState, SessionStateMut};
pub use state::*;

// Re-export sibling modules so sub-modules can use `super::events`, etc.
pub(crate) use super::events;
pub(crate) use super::onboarding;
