//! spawn_agent tool — creates a child agent with forked context.

use super::manager::{SubAgent, SubAgentManager, SubAgentState};
use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Tool that spawns a child agent to handle a sub-task.
pub struct SpawnAgentTool {
    manager: Arc<SubAgentManager>,
}

impl SpawnAgentTool {
    pub fn new(manager: Arc<SubAgentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Spawn a child agent to handle a sub-task autonomously. The child gets its own session \
         and runs in the background. Returns an agent_id you can use with wait_agent, send_input, \
         close_agent, or resume_agent. Use this to delegate independent work items."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The task/instruction for the child agent to execute"
                },
                "label": {
                    "type": "string",
                    "description": "Short human-readable label for this sub-agent (e.g., 'refactor-auth', 'test-runner')"
                }
            },
            "required": ["prompt"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'prompt' is required".into()))?
            .to_string();

        let label = input
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("sub-agent")
            .to_string();

        // We need a ServiceContext to create a session for the child
        let service_context = context
            .service_context
            .as_ref()
            .ok_or_else(|| ToolError::Execution("No service context available".into()))?
            .clone();

        // Create a new session for the child agent
        let session_service = crate::services::SessionService::new(service_context.clone());
        let child_session = session_service
            .create_session(Some(format!("subagent: {}", label)))
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to create child session: {}", e)))?;

        let child_session_id = child_session.id;
        let agent_id = SubAgentManager::generate_id();

        // Create cancel token and input channel for the child
        let cancel_token = CancellationToken::new();
        let (input_tx, _input_rx) = mpsc::unbounded_channel::<String>();

        // Build a minimal AgentService for the child
        let child_service = {
            let config = crate::config::Config::load()
                .map_err(|e| ToolError::Execution(format!("Config load failed: {}", e)))?;

            // Get provider from parent context
            let provider = crate::brain::provider::create_provider(&config)
                .map_err(|e| ToolError::Execution(format!("Failed to create provider: {}", e)))?;

            // Build tool registry for child (same tools as parent, minus spawn to prevent recursion)
            let child_registry = crate::brain::tools::ToolRegistry::new();

            // Register essential tools for the child
            child_registry.register(Arc::new(crate::brain::tools::read::ReadTool));
            child_registry.register(Arc::new(crate::brain::tools::write::WriteTool));
            child_registry.register(Arc::new(crate::brain::tools::edit::EditTool));
            child_registry.register(Arc::new(crate::brain::tools::bash::BashTool));
            child_registry.register(Arc::new(crate::brain::tools::glob::GlobTool));
            child_registry.register(Arc::new(crate::brain::tools::grep::GrepTool));
            child_registry.register(Arc::new(crate::brain::tools::ls::LsTool));
            child_registry.register(Arc::new(crate::brain::tools::web_search::WebSearchTool));

            let agent =
                crate::brain::agent::AgentService::new(provider, service_context.clone(), &config)
                    .with_tool_registry(Arc::new(child_registry))
                    .with_auto_approve_tools(true) // children auto-approve (parent already approved spawn)
                    .with_working_directory(context.working_directory.clone());

            Arc::new(agent)
        };

        // Spawn background task
        let cancel_clone = cancel_token.clone();
        let manager = self.manager.clone();
        let agent_id_clone = agent_id.clone();
        let prompt_clone = prompt.clone();

        let handle = tokio::spawn(async move {
            tracing::info!("Sub-agent {} starting: {}", agent_id_clone, prompt_clone);

            let result = child_service
                .send_message_with_tools_and_mode(
                    child_session_id,
                    prompt_clone,
                    None,
                    Some(cancel_clone),
                )
                .await;

            match result {
                Ok(response) => {
                    tracing::info!("Sub-agent {} completed", agent_id_clone);
                    manager.mark_completed(&agent_id_clone, response.content);
                }
                Err(e) => {
                    tracing::error!("Sub-agent {} failed: {}", agent_id_clone, e);
                    manager.mark_failed(&agent_id_clone, e.to_string());
                }
            }
        });

        // Register in manager
        self.manager.insert(SubAgent {
            id: agent_id.clone(),
            label: label.clone(),
            session_id: child_session_id,
            state: SubAgentState::Running,
            cancel_token,
            join_handle: Some(handle),
            input_tx: Some(input_tx),
            output: None,
            spawned_at: chrono::Utc::now(),
        });

        Ok(ToolResult::success(format!(
            "Spawned sub-agent '{}' with id: {}\nSession: {}\nPrompt: {}",
            label, agent_id, child_session_id, prompt
        )))
    }
}
