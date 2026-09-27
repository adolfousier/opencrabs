//! Goal system — autonomous task completion loop.
//!
//! A goal is a free-form user objective that persists across turns. After
//! each turn completes, a lightweight judge call asks the same LLM "is
//! this goal satisfied?". If not, a continuation prompt is injected and
//! the tool loop re-enters. The loop is bounded by a turn budget
//! (default 20) and auto-pauses on consecutive judge parse failures.

pub mod driver;
pub mod judge;
pub mod manager;
pub mod prompt;
pub mod types;

pub use driver::{DRIVER_STALE_AFTER_SECS, driver_line};
pub use manager::GoalManager;
pub use prompt::{goal_command_prompt, goal_usage_warning, is_bare};
pub use types::{GoalDecision, GoalVerdict, JudgeDecision};
