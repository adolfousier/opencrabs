//! Dynamic Tool System
//!
//! Runtime-defined tools loaded from `~/.opencrabs/tools.toml`.
//! These appear in the agent's tool list alongside compiled tools
//! and can be added/removed/reloaded without restarting.

pub mod loader;
pub mod tool;

pub use loader::DynamicToolLoader;
pub use tool::{DynamicTool, DynamicToolDef, DynamicToolsConfig, ExecutorType, ParamDef};
