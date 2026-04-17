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

        // Fast path — terminal or round-boundary states already visible
        // without awaiting the task handle (the task never terminates at a
        // round boundary, only on input/cancel, so handle.await would sit
        // until timeout_secs fires and return nothing useful to the LLM).
        if let Some(result) = self.terminal_or_pause_result(agent_id) {
            return Ok(result);
        }

        // Poll state until we see Completed / Failed / Cancelled /
        // AwaitingInput, or the caller's timeout fires. 250ms cadence is
        // cheap (a read-lock + clone) and keeps wait_agent responsive.
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs.max(1));
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
        ticker.tick().await; // burn first immediate tick
        loop {
            if let Some(result) = self.terminal_or_pause_result(agent_id) {
                return Ok(result);
            }
            if tokio::time::Instant::now() >= deadline {
                // Timed out mid-round. Return whatever partial progress
                // we have so the LLM sees work-in-flight instead of an
                // empty "still running" string and giving up the turn
                // (2026-04-17 23:05 log: model ended turn after repeated
                // timeout_secs=5 polls returned no content).
                let partial = self.manager.get_output(agent_id).unwrap_or_default();
                let progress_hint = if partial.is_empty() {
                    "No output yet.".to_string()
                } else {
                    let preview = if partial.len() > 1200 {
                        let mut end = 1200;
                        while !partial.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}…", &partial[..end])
                    } else {
                        partial.clone()
                    };
                    format!("Latest progress from sub-agent so far:\n{}", preview)
                };
                return Ok(ToolResult::success(format!(
                    "Sub-agent {} still running after {}s. Call wait_agent again \
                     with a longer timeout_secs to keep polling, or close_agent \
                     to cancel.\n\n{}",
                    agent_id, timeout_secs, progress_hint
                )));
            }
            ticker.tick().await;
        }
    }
}

impl WaitAgentTool {
    /// Return a ToolResult if the agent has reached a state worth reporting
    /// immediately: terminal (Completed / Failed / Cancelled) or
    /// round-boundary (AwaitingInput). Returns None while still Running.
    fn terminal_or_pause_result(&self, agent_id: &str) -> Option<ToolResult> {
        let state = self.manager.get_state(agent_id)?;
        match state {
            SubAgentState::Completed => {
                let output = self.manager.get_output(agent_id).unwrap_or_default();
                Some(ToolResult::success(format!(
                    "Sub-agent {} completed.\n\nOutput:\n{}",
                    agent_id, output
                )))
            }
            SubAgentState::AwaitingInput => {
                // Round finished, sub-agent paused for follow-up. Surface
                // the round output so the parent LLM can act on it.
                let output = self.manager.get_output(agent_id).unwrap_or_default();
                Some(ToolResult::success(format!(
                    "Sub-agent {} finished a round and is paused for input.\n\n\
                     Round output:\n{}\n\n\
                     Call send_input to continue, or close_agent if done.",
                    agent_id, output
                )))
            }
            SubAgentState::Failed(err) => Some(ToolResult::error(format!(
                "Sub-agent {} failed: {}",
                agent_id, err
            ))),
            SubAgentState::Cancelled => Some(ToolResult::error(format!(
                "Sub-agent {} was cancelled",
                agent_id
            ))),
            SubAgentState::Running => None,
        }
    }
}
