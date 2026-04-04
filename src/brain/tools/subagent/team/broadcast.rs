//! team_broadcast tool — send a message to all running agents in a team.

use super::manager::TeamManager;
use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::subagent::manager::SubAgentManager;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool that broadcasts a message to all running agents in a team.
pub struct TeamBroadcastTool {
    subagent_manager: Arc<SubAgentManager>,
    team_manager: Arc<TeamManager>,
}

impl TeamBroadcastTool {
    pub fn new(subagent_manager: Arc<SubAgentManager>, team_manager: Arc<TeamManager>) -> Self {
        Self {
            subagent_manager,
            team_manager,
        }
    }
}

#[async_trait]
impl Tool for TeamBroadcastTool {
    fn name(&self) -> &str {
        "team_broadcast"
    }

    fn description(&self) -> &str {
        "Send a message to all running agents in a team. Non-running agents are skipped. \
         Use this to coordinate team members or provide shared context."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "Name of the team to broadcast to"
                },
                "message": {
                    "type": "string",
                    "description": "Message to send to all running team agents"
                }
            },
            "required": ["team_name", "message"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let team_name = input
            .get("team_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'team_name' is required".into()))?;

        let message = input
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'message' is required".into()))?
            .to_string();

        let agent_ids = self
            .team_manager
            .get_agent_ids(team_name)
            .ok_or_else(|| ToolError::InvalidInput(format!("Team '{}' not found", team_name)))?;

        let mut sent = 0;
        let mut skipped = 0;

        for agent_id in &agent_ids {
            if let Some(tx) = self.subagent_manager.get_input_tx(agent_id) {
                if tx.send(message.clone()).is_ok() {
                    sent += 1;
                } else {
                    skipped += 1;
                }
            } else {
                skipped += 1;
            }
        }

        Ok(ToolResult::success(format!(
            "Broadcast to team '{}': {} agents received, {} skipped (not running)",
            team_name, sent, skipped
        )))
    }
}
