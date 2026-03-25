//! Split pane management — tmux-style pane splitting, layout, and focus tracking.

mod layout;
mod state;

pub use layout::{SplitDirection, SplitNode};
pub use state::{Pane, PaneId, PaneManager};
