//! wait_agent tool — blocks until a child agent completes and returns its output.

use super::manager::{SubAgentManager, SubAgentState};
use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool that waits for a spawned child agent to finish.
pub struct WaitAgentTool {
    manager: Arc<SubAgentManager>,
}

impl WaitAgentTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for WaitAgentTool {
    fn name(&self) -> &str {
        "wait_agent"
    }

    fn description(&self) -> &str {
        "Wait for a spawned sub-agent to complete and return its output. \
         If the agent is already finished, returns immediately. \
         Use with an optional timeout_secs (default: 300s)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "The ID returned by spawn_agent"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Maximum seconds to wait (default: 300)",
                    "default": 300
                }
            },
            "required": ["agent_id"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let agent_id = input
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'agent_id' is required".into()))?;

        let timeout_secs = input
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        if !self.manager.exists(agent_id) {
            return Ok(ToolResult::error(format!(
                "No sub-agent found with id: {}",
                agent_id
            )));
        }

        // If already completed/failed, return immediately
        if let Some(state) = self.manager.get_state(agent_id) {
            match state {
                SubAgentState::Completed => {
                    let output = self.manager.get_output(agent_id).unwrap_or_default();
                    return Ok(ToolResult::success(format!(
                        "Sub-agent {} completed.\n\nOutput:\n{}",
                        agent_id, output
                    )));
                }
                SubAgentState::Failed(err) => {
                    return Ok(ToolResult::error(format!(
                        "Sub-agent {} failed: {}",
                        agent_id, err
                    )));
                }
                SubAgentState::Cancelled => {
                    return Ok(ToolResult::error(format!(
                        "Sub-agent {} was cancelled",
                        agent_id
                    )));
                }
                SubAgentState::Running => {}
            }
        }

        // Take the join handle and await with timeout
        let handle = self.manager.take_join_handle(agent_id);
        if let Some(handle) = handle {
            let timeout =
                tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), handle).await;

            match timeout {
                Ok(Ok(())) => {
                    // Task completed — check final state
                    if let Some(state) = self.manager.get_state(agent_id) {
                        match state {
                            SubAgentState::Completed => {
                                let output = self.manager.get_output(agent_id).unwrap_or_default();
                                return Ok(ToolResult::success(format!(
                                    "Sub-agent {} completed.\n\nOutput:\n{}",
                                    agent_id, output
                                )));
                            }
                            SubAgentState::Failed(err) => {
                                return Ok(ToolResult::error(format!(
                                    "Sub-agent {} failed: {}",
                                    agent_id, err
                                )));
                            }
                            _ => {}
                        }
                    }
                    Ok(ToolResult::success(format!(
                        "Sub-agent {} finished (state unknown)",
                        agent_id
                    )))
                }
                Ok(Err(e)) => Ok(ToolResult::error(format!(
                    "Sub-agent {} task panicked: {}",
                    agent_id, e
                ))),
                Err(_) => {
                    // Timeout — agent still running
                    Ok(ToolResult::error(format!(
                        "Timed out after {}s waiting for sub-agent {}. \
                         It is still running. Use wait_agent again or close_agent to cancel.",
                        timeout_secs, agent_id
                    )))
                }
            }
        } else {
            // No handle — already awaited or never set
            let state = self.manager.get_state(agent_id);
            Ok(ToolResult::success(format!(
                "Sub-agent {} state: {:?}",
                agent_id,
                state.unwrap_or(SubAgentState::Failed("unknown".into()))
            )))
        }
    }
}
