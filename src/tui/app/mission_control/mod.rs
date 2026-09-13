//! Mission Control — app-side state, input, actions.
//!
//! Pairs with `src/brain/mission_control/` (data services) and
//! `src/tui/render/mission_control/` (renderers). Single responsibility
//! per file: state lives in `state.rs`, key handling in `input.rs`,
//! side-effecting actions in `actions.rs`.

pub mod actions;
pub mod input;
pub mod log_viewer;
pub mod state;

pub use state::{McPanel, McState};
