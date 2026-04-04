//! team_delete tool — cancel all agents in a team and remove the team.

use super::manager::TeamManager;
use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::subagent::manager::SubAgentManager;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool that deletes a team, cancelling all its running agents.
pub struct TeamDeleteTool {
    subagent_manager: Arc<SubAgentManager>,
    team_manager: Arc<TeamManager>,
}

impl TeamDeleteTool {
    pub fn new(subagent_manager: Arc<SubAgentManager>, team_manager: Arc<TeamManager>) -> Self {
        Self {
            subagent_manager,
            team_manager,
        }
    }
}

#[async_trait]
impl Tool for TeamDeleteTool {
    fn name(&self) -> &str {
        "team_delete"
    }

    fn description(&self) -> &str {
        "Delete a named team, cancelling all its running agents. Completed agents are left \
         in the subagent manager but removed from the team."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "Name of the team to delete"
                }
            },
            "required": ["team_name"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let team_name = input
            .get("team_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'team_name' is required".into()))?;

        let team = self
            .team_manager
            .delete_team(team_name)
            .ok_or_else(|| ToolError::InvalidInput(format!("Team '{}' not found", team_name)))?;

        let mut cancelled = 0;
        let mut already_done = 0;

        for agent_id in &team.agent_ids {
            if self.subagent_manager.cancel(agent_id) {
                cancelled += 1;
            } else {
                already_done += 1;
            }
        }

        Ok(ToolResult::success(format!(
            "Deleted team '{}': {} agents cancelled, {} already completed/failed",
            team_name, cancelled, already_done
        )))
    }
}
