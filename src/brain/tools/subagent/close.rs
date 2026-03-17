//! close_agent tool — terminates a running child agent.

use super::manager::{SubAgentManager, SubAgentState};
use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool that closes/cancels a spawned sub-agent.
pub struct CloseAgentTool {
    manager: Arc<SubAgentManager>,
}

impl CloseAgentTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for CloseAgentTool {
    fn name(&self) -> &str {
        "close_agent"
    }

    fn description(&self) -> &str {
        "Terminate a running sub-agent and clean up its resources. \
         Use this when a sub-agent's work is no longer needed."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "The ID of the sub-agent to close"
                },
                "remove": {
                    "type": "boolean",
                    "description": "Also remove the agent from tracking (default: false)",
                    "default": false
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

        let remove = input
            .get("remove")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !self.manager.exists(agent_id) {
            return Ok(ToolResult::error(format!(
                "No sub-agent found with id: {}",
                agent_id
            )));
        }

        let state = self.manager.get_state(agent_id);

        // Cancel if running
        if matches!(state, Some(SubAgentState::Running)) {
            self.manager.cancel(agent_id);
        }

        let status = if remove {
            self.manager.remove(agent_id);
            "cancelled and removed from tracking"
        } else {
            "cancelled"
        };

        Ok(ToolResult::success(format!(
            "Sub-agent {} {}.",
            agent_id, status
        )))
    }
}
