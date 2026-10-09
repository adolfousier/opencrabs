//! Slack Integration
//!
//! Runs a Slack bot via Socket Mode alongside the TUI, forwarding messages from
//! allowlisted users to the AgentService and replying with responses.
//!
//! Layout: [`state`] holds the shared `SlackState` struct and its
//! constructor; each concern has its own impl module beside it
//! (`approval`, `cancel`, `connection`, `followups`, `sessions`, and the
//! tool-group methods in `tool_group`); `agent` runs Socket Mode,
//! `handler` routes inbound messages, `blocks` / `final_body` /
//! `formatting_prompt` / `table_convert` shape output, `reactions` /
//! `suggest_options` / `upload` handle their surfaces and `resume`
//! re-delivers background results. This file is declarations only — no
//! function definitions live here (CONTRIBUTING.md).

mod agent;
mod approval;
pub(crate) mod blocks;
mod cancel;
mod connection;
pub(crate) mod final_body;
mod followups;
pub(crate) mod formatting_prompt;
pub(crate) mod governor;
pub(crate) mod handler;
pub(crate) mod reactions;
pub(crate) mod resume;
mod sessions;
mod state;
pub(crate) mod suggest_options;
pub(crate) mod table_convert;
pub(crate) mod tool_group;
pub(crate) mod upload;

pub use agent::SlackAgent;
pub use state::SlackState;
