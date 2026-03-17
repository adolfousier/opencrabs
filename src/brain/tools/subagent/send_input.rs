//! send_input tool — sends follow-up input to a running child agent.

use super::manager::{SubAgentManager, SubAgentState};
use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool that sends input to a running sub-agent.
pub struct SendInputTool {
    manager: Arc<SubAgentManager>,
}

impl SendInputTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SendInputTool {
    fn name(&self) -> &str {
        "send_input"
    }

    fn description(&self) -> &str {
        "Send follow-up input/instructions to a running sub-agent. \
         The message is queued and the agent will process it at its next iteration."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "The ID of the running sub-agent"
                },
                "text": {
                    "type": "string",
                    "description": "The input/instruction to send to the sub-agent"
                }
            },
            "required": ["agent_id", "text"]
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

        let text = input
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'text' is required".into()))?;

        // Check agent exists and is running
        match self.manager.get_state(agent_id) {
            None => {
                return Ok(ToolResult::error(format!(
                    "No sub-agent found with id: {}",
                    agent_id
                )));
            }
            Some(SubAgentState::Running) => {}
            Some(state) => {
                return Ok(ToolResult::error(format!(
                    "Sub-agent {} is not running (state: {:?}). Cannot send input.",
                    agent_id, state
                )));
            }
        }

        // Send via the input channel
        if let Some(tx) = self.manager.get_input_tx(agent_id) {
            tx.send(text.to_string()).map_err(|_| {
                ToolError::Execution(format!(
                    "Failed to send input to sub-agent {} — channel closed",
                    agent_id
                ))
            })?;

            Ok(ToolResult::success(format!(
                "Input sent to sub-agent {}:\n{}",
                agent_id, text
            )))
        } else {
            Ok(ToolResult::error(format!(
                "Sub-agent {} has no input channel (may have finished)",
                agent_id
            )))
        }
    }
}
