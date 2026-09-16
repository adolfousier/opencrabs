//! Discord Integration
//!
//! Runs a Discord bot alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.
//!
//! Layout: [`state`] holds the shared `DiscordState` struct and its
//! constructor; each concern has its own impl module beside it
//! (`approval`, `cancel`, `connection`, `pending_interactions`,
//! `sessions`, and the tool-group methods in `tool_group`); `agent` runs
//! the gateway, `handler` routes inbound messages, `interactions` /
//! `reactions` / `suggest_options` / `typing` handle their UI surfaces and
//! `resume` re-delivers background results. This file is declarations
//! only — no function definitions live here (CONTRIBUTING.md).

mod agent;
mod approval;
mod cancel;
mod connection;
pub(crate) mod handler;
pub(crate) mod interactions;
mod pending_interactions;
pub(crate) mod reactions;
pub(crate) mod resume;
mod sessions;
mod state;
pub(crate) mod suggest_options;
pub(crate) mod table_convert;
pub(crate) mod tool_group;
pub(crate) mod typing;

pub use agent::DiscordAgent;
pub use state::DiscordState;
