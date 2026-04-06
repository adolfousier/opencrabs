//! Sub-Agent Spawning Tools
//!
//! Provides 5 tools for multi-agent orchestration inspired by codex-rs:
//! spawn_agent, wait_agent, send_input, close_agent, resume_agent.
//!
//! Each child agent gets a forked context, its own CancellationToken,
//! and runs in a background tokio task. The parent can wait, send input,
//! close, or resume any child by agent_id.

pub mod agent_type;
mod close;
pub mod manager;
mod resume;
mod send_input;
pub mod spawn;
pub mod status;
pub mod team;
mod wait;

pub use agent_type::AgentType;
pub use close::CloseAgentTool;
pub use manager::{SubAgent, SubAgentManager, SubAgentState};
pub use resume::ResumeAgentTool;
pub use send_input::SendInputTool;
pub use spawn::SpawnAgentTool;
pub use team::{TeamBroadcastTool, TeamCreateTool, TeamDeleteTool, TeamManager};
pub use wait::WaitAgentTool;
