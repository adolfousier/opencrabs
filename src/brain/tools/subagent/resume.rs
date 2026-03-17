//! resume_agent tool — resumes a completed/failed child agent with new input.

use super::manager::{SubAgentManager, SubAgentState};
use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Tool that resumes a previously completed or failed sub-agent.
pub struct ResumeAgentTool {
    manager: Arc<SubAgentManager>,
}

impl ResumeAgentTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ResumeAgentTool {
    fn name(&self) -> &str {
        "resume_agent"
    }

    fn description(&self) -> &str {
        "Resume a completed or failed sub-agent with a new prompt. \
         The agent continues in the same session, preserving its prior context."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "The ID of the sub-agent to resume"
                },
                "prompt": {
                    "type": "string",
                    "description": "New instruction/prompt for the resumed agent"
                }
            },
            "required": ["agent_id", "prompt"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let agent_id = input
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'agent_id' is required".into()))?;

        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'prompt' is required".into()))?
            .to_string();

        // Check agent exists and is in a resumable state
        match self.manager.get_state(agent_id) {
            None => {
                return Ok(ToolResult::error(format!(
                    "No sub-agent found with id: {}",
                    agent_id
                )));
            }
            Some(SubAgentState::Running) => {
                return Ok(ToolResult::error(format!(
                    "Sub-agent {} is still running. Use wait_agent first or close_agent to cancel.",
                    agent_id
                )));
            }
            Some(SubAgentState::Completed) | Some(SubAgentState::Failed(_)) => {}
            Some(SubAgentState::Cancelled) => {
                return Ok(ToolResult::error(format!(
                    "Sub-agent {} was cancelled and cannot be resumed.",
                    agent_id
                )));
            }
        }

        let session_id = self.manager.get_session_id(agent_id).ok_or_else(|| {
            ToolError::Execution(format!("No session found for sub-agent {}", agent_id))
        })?;

        let service_context = context
            .service_context
            .as_ref()
            .ok_or_else(|| ToolError::Execution("No service context available".into()))?
            .clone();

        // Create new cancel token and input channel
        let cancel_token = CancellationToken::new();
        let (input_tx, _input_rx) = mpsc::unbounded_channel::<String>();

        // Prepare the agent for resumption
        let agent_id_str = agent_id.to_string();
        if !self
            .manager
            .prepare_resume(&agent_id_str, cancel_token.clone(), input_tx)
        {
            return Ok(ToolResult::error(format!(
                "Failed to prepare sub-agent {} for resumption",
                agent_id
            )));
        }

        // Build a new AgentService for the resumed run
        let child_service = {
            let config = crate::config::Config::load().unwrap_or_default();
            let provider = crate::brain::provider::create_provider(&config)
                .map_err(|e| ToolError::Execution(format!("Failed to create provider: {}", e)))?;

            let mut child_registry = crate::brain::tools::ToolRegistry::new();
            child_registry.register(Arc::new(crate::brain::tools::read::ReadTool));
            child_registry.register(Arc::new(crate::brain::tools::write::WriteTool));
            child_registry.register(Arc::new(crate::brain::tools::edit::EditTool));
            child_registry.register(Arc::new(crate::brain::tools::bash::BashTool));
            child_registry.register(Arc::new(crate::brain::tools::glob::GlobTool));
            child_registry.register(Arc::new(crate::brain::tools::grep::GrepTool));
            child_registry.register(Arc::new(crate::brain::tools::ls::LsTool));
            child_registry.register(Arc::new(crate::brain::tools::web_search::WebSearchTool));

            Arc::new(
                crate::brain::agent::AgentService::new(provider, service_context)
                    .with_tool_registry(Arc::new(child_registry))
                    .with_auto_approve_tools(true)
                    .with_working_directory(context.working_directory.clone()),
            )
        };

        // Spawn the resumed task
        let cancel_clone = cancel_token.clone();
        let manager = self.manager.clone();
        let agent_id_clone = agent_id_str.clone();
        let prompt_clone = prompt.clone();

        let handle = tokio::spawn(async move {
            tracing::info!("Sub-agent {} resuming: {}", agent_id_clone, prompt_clone);

            let result = child_service
                .send_message_with_tools_and_mode(
                    session_id,
                    prompt_clone,
                    None,
                    Some(cancel_clone),
                )
                .await;

            match result {
                Ok(response) => {
                    tracing::info!("Sub-agent {} resumed and completed", agent_id_clone);
                    manager.mark_completed(&agent_id_clone, response.content);
                }
                Err(e) => {
                    tracing::error!("Sub-agent {} resumed and failed: {}", agent_id_clone, e);
                    manager.mark_failed(&agent_id_clone, e.to_string());
                }
            }
        });

        self.manager.set_join_handle(&agent_id_str, handle);

        Ok(ToolResult::success(format!(
            "Resumed sub-agent {} with new prompt:\n{}",
            agent_id, prompt
        )))
    }
}
