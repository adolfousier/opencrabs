//! team_create tool — spawn a named team of agents from a single command.

use super::manager::TeamManager;
use crate::brain::tools::error::{Result, ToolError};
use crate::brain::tools::subagent::manager::{SubAgent, SubAgentManager};
use crate::brain::tools::subagent::map_deprecated_agent_type;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Tool that spawns a named team of sub-agents from a list of tasks.
pub struct TeamCreateTool {
    subagent_manager: Arc<SubAgentManager>,
    team_manager: Arc<TeamManager>,
    parent_registry: Arc<crate::brain::tools::ToolRegistry>,
}

impl TeamCreateTool {
    pub fn new(
        subagent_manager: Arc<SubAgentManager>,
        team_manager: Arc<TeamManager>,
        parent_registry: Arc<crate::brain::tools::ToolRegistry>,
    ) -> Self {
        Self {
            subagent_manager,
            team_manager,
            parent_registry,
        }
    }
}

#[async_trait]
impl Tool for TeamCreateTool {
    fn name(&self) -> &str {
        "team_create"
    }

    fn description(&self) -> &str {
        "Create a named team by spawning multiple sub-agents at once. Each agent gets its own \
         task and optional type. Returns team name and all agent IDs. \
         \n\nProvider and model resolution is PER MEMBER. Each entry in the `agents` array \
         can carry its own `provider` and `model` fields, with the same precedence as \
         spawn_agent: per-member > config.agent.subagent_* > parent. This lets one \
         team_create call spawn agents that each use a different model — useful when a \
         skill orchestrates a plan-with-GLM / code-with-Deepseek / review-with-Kimi flow \
         as one atomic team rather than chained spawn_agent calls."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "Unique name for this team (e.g., 'backend-refactor', 'test-suite')"
                },
                "agents": {
                    "type": "array",
                    "description": "List of agents to spawn",
                    "items": {
                        "type": "object",
                        "properties": {
                            "prompt": {
                                "type": "string",
                                "description": "Task for this agent"
                            },
                            "label": {
                                "type": "string",
                                "description": "Short label for this agent"
                            },
                            "allow_nested": {
                                "type": "boolean",
                                "description": "Whether THIS MEMBER may spawn further sub-agents or background tasks (#1195). Default true. false = pure worker."
                            },
                            "read_only": {
                                "type": "boolean",
                                "description": "Spawn this member with a read-restricted tool registry (#1173): reads/search/research only. Omit or false for full capability."
                            },
                            "include_brain": {
                                "type": "boolean",
                                "description": "Whether to pre-inject workspace brain files (SOUL.md, USER.md, AGENTS.md) into this team member (#145). Default false (lean execution). Set true when delegating full co-worker tasks."
                            },
                            "provider": {
                                "type": "string",
                                "description": "Optional per-member provider override (e.g., 'zhipu', 'openrouter', 'custom:my-provider'). Highest precedence — overrides config.agent.subagent_provider for THIS member only."
                            },
                            "model": {
                                "type": "string",
                                "description": "Optional per-member model override. Highest precedence — overrides config.agent.subagent_model for THIS member only."
                            }
                        },
                        "required": ["prompt"]
                    }
                }
            },
            "required": ["team_name", "agents"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        // Nesting enforcement (#1195), mirroring spawn_agent.
        if let Some(ref mgr) = context.subagent_manager
            && !mgr.nesting_allowed_for_session(context.session_id)
        {
            return Ok(ToolResult::error(
                "refused: this agent was spawned without nesting permissions \
                 (allow_nested=false) and may not create teams"
                    .to_string(),
            ));
        }
        let team_name = input
            .get("team_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("'team_name' is required".into()))?
            .to_string();

        let agents_array = input
            .get("agents")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::InvalidInput("'agents' must be an array".into()))?;

        if agents_array.is_empty() {
            return Err(ToolError::InvalidInput(
                "'agents' array cannot be empty".into(),
            ));
        }

        if self.team_manager.exists(&team_name) {
            return Err(ToolError::InvalidInput(format!(
                "Team '{}' already exists",
                team_name
            )));
        }

        let service_context = context
            .service_context
            .as_ref()
            .ok_or_else(|| ToolError::Execution("No service context available".into()))?
            .clone();

        let config = crate::config::Config::load()
            .map_err(|e| ToolError::Execution(format!("Config load failed: {}", e)))?;

        let mut spawned_ids = Vec::new();
        let mut spawn_results = Vec::new();

        for agent_def in agents_array {
            let prompt = agent_def
                .get("prompt")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidInput("Each agent needs a 'prompt'".into()))?
                .to_string();

            let label = agent_def
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("team-member")
                .to_string();

            // Per-member capability grant (#1173): explicit read_only wins;
            // a deprecated typed agent_type maps to its historical grant,
            // loudly; unknown types fail closed for THIS member only.
            let member_read_only = match agent_def.get("read_only") {
                Some(v) => Some(v.as_bool().ok_or_else(|| {
                    ToolError::InvalidInput(format!(
                        "Team member '{label}': 'read_only' must be a boolean"
                    ))
                })?),
                None => None,
            };
            let deprecated_raw = agent_def
                .get("agent_type")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            // Nesting grant (#1195): absent = true; non-boolean = hard error.
            let member_allow_nested = match agent_def.get("allow_nested") {
                None => true,
                Some(serde_json::Value::Bool(b)) => *b,
                Some(_) => {
                    return Ok(ToolResult::error(format!(
                        "Team member '{label}': 'allow_nested' must be a boolean"
                    )));
                }
            };

            // Brain pre-injection flag (#145): absent = false; non-boolean = hard error.
            let member_include_brain = match agent_def.get("include_brain") {
                None => false,
                Some(serde_json::Value::Bool(b)) => *b,
                Some(_) => {
                    return Ok(ToolResult::error(format!(
                        "Team member '{label}': 'include_brain' must be a boolean"
                    )));
                }
            };

            let (read_only, member_grant_note) = match (member_read_only, deprecated_raw) {
                (Some(ro), _) => (ro, None),
                (None, Some(raw)) => {
                    let grant = map_deprecated_agent_type(&raw)
                        .map_err(|e| ToolError::InvalidInput(format!("{label}: {e}")))?;
                    tracing::warn!(
                        "team_create member '{label}' uses deprecated agent_type='{raw}'; \
                         mapped to read_only={grant} (#1173)"
                    );
                    (
                        grant,
                        Some(format!("deprecated agent_type '{raw}' → read_only={grant}")),
                    )
                }
                (None, None) => (false, None),
            };

            // Per-member provider / model overrides (issue #152). Same
            // precedence as spawn_agent: per-member > config > parent.
            // Resolved here inside the loop so each team member can
            // route to a different model in a single team_create call
            // — that's the orchestration shape the issue requested.
            let member_provider = agent_def
                .get("provider")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let member_model = agent_def
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            // Per-member > config, normalised the same way spawn resolves
            // it (#1316).
            let child = super::super::provider_pair::child_pair(
                &config,
                member_provider.as_deref(),
                member_model.as_deref(),
            );
            let model_override = child.model.clone();
            let effective_provider_name = child.provider.clone();

            // Create session for this agent
            let session_service = crate::services::SessionService::new(service_context.clone());
            let child_session = session_service
                .create_session(Some(format!("team:{}/{}", team_name, label)))
                .await
                .map_err(|e| ToolError::Execution(format!("Failed to create session: {}", e)))?;

            let child_session_id = child_session.id;
            let agent_id = SubAgentManager::generate_id();

            let cancel_token = CancellationToken::new();
            let (input_tx, mut input_rx) = mpsc::unbounded_channel::<String>();

            // Create provider — per-member override wins over config.
            let provider = if let Some(ref provider_name) = effective_provider_name {
                match crate::brain::provider::create_provider_by_name(&config, provider_name).await
                {
                    Ok(p) => {
                        let source = match child.source {
                            super::super::provider_pair::ProviderSource::PerCall => "per-member",
                            super::super::provider_pair::ProviderSource::Config => "config",
                        };
                        tracing::info!(
                            "Team member '{label}' using {source} provider '{provider_name}'"
                        );
                        p
                    }
                    Err(_) => crate::brain::provider::create_provider(&config)
                        .await
                        .map_err(|e| {
                            ToolError::Execution(format!(
                                "Fallback provider creation failed: {}",
                                e
                            ))
                        })?,
                }
            } else {
                crate::brain::provider::create_provider(&config)
                    .await
                    .map_err(|e| ToolError::Execution(format!("Provider creation failed: {}", e)))?
            };

            let child_registry = super::super::build_child_registry(&self.parent_registry);
            if read_only {
                crate::brain::tools::plan_gate::restrict_registry_to_read_only(&child_registry);
                tracing::info!(
                    "Team member '{label}' spawned read-only: registry restricted (#1173)"
                );
            }

            let child_dir = context.working_dir();
            let system_brain = super::super::brain::child_system_brain(
                member_include_brain,
                &child_dir,
                model_override.as_deref(),
                effective_provider_name.as_deref(),
            );

            let mut child_service_builder =
                crate::brain::agent::AgentService::new(provider, service_context.clone(), &config)
                    .await
                    .with_tool_registry(child_registry)
                    .with_auto_approve_tools(true)
                    .with_working_directory(child_dir);

            if let Some(brain) = system_brain {
                child_service_builder = child_service_builder.with_system_brain(brain);
            }

            let child_service = Arc::new(child_service_builder);

            // Typed preambles are gone (#1173, Proposal B): restricted
            // members get one factual capability line, full members just the
            // task.
            // #145: stacked capability + lean context note.
            let full_prompt =
                super::super::brain::child_prompt(read_only, member_include_brain, &prompt);

            let cancel_clone = cancel_token.clone();
            let manager = self.subagent_manager.clone();
            let agent_id_clone = agent_id.clone();
            let model_clone = model_override.clone();
            // Delivery identity (#1197): team members must wake the parent
            // on completion, same contract as spawned agents.
            let parent_of_member = context.session_id;
            let member_label = label.clone();

            let handle = tokio::spawn(async move {
                tracing::info!("Team agent {} starting", agent_id_clone);

                let mut current_prompt = full_prompt;

                let final_output = loop {
                    let result = child_service
                        .send_message_with_tools_and_mode(
                            child_session_id,
                            current_prompt,
                            model_clone.clone(),
                            Some(cancel_clone.clone()),
                        )
                        .await;

                    match result {
                        Ok(response) => {
                            manager.update_output(&agent_id_clone, response.content.clone());
                            // Natural completion (#1184), same rule as
                            // spawn.rs: only a genuinely gated round keeps
                            // waiting; a finished answer delivers instead of
                            // parking the agent forever.
                            if response.stop_reason
                                != Some(crate::brain::provider::types::StopReason::ToolUse)
                            {
                                tracing::info!(
                                    "Team agent {} round complete naturally, delivering result",
                                    agent_id_clone
                                );
                                break response.content;
                            }
                            manager.mark_awaiting_input(&agent_id_clone);
                            tracing::info!(
                                "Team agent {} round complete, waiting for input",
                                agent_id_clone
                            );

                            let next = tokio::select! {
                                msg = input_rx.recv() => msg,
                                _ = cancel_clone.cancelled() => {
                                    tracing::info!("Team agent {} cancelled while waiting for input", agent_id_clone);
                                    None
                                }
                            };

                            match next {
                                Some(text) => {
                                    // Flip back to Running so the in-memory
                                    // state matches the round now in flight —
                                    // without this a wait_agent during the new
                                    // round reads the parked state and returns
                                    // the PREVIOUS round's stale output (#1183).
                                    manager.mark_running_again(&agent_id_clone);
                                    current_prompt = text;
                                }
                                None => break response.content,
                            }
                        }
                        Err(e) => {
                            tracing::error!("Team agent {} failed: {}", agent_id_clone, e);
                            manager.mark_failed(&agent_id_clone, e.to_string());
                            return;
                        }
                    }
                };

                manager.complete_and_deliver(
                    &agent_id_clone,
                    final_output,
                    parent_of_member,
                    &member_label,
                );
            });

            // Register in subagent manager
            self.subagent_manager.insert(SubAgent {
                read_only,
                include_brain: member_include_brain,
                allow_nested: member_allow_nested,
                cancel_token,
                join_handle: Some(handle),
                input_tx: Some(input_tx),
                ..SubAgent::new(
                    agent_id.clone(),
                    label.clone(),
                    child_session_id,
                    context.session_id,
                )
            });

            spawned_ids.push(agent_id.clone());
            spawn_results.push(format!(
                "  {} ({}) → {}{}",
                label,
                if read_only { "read-only" } else { "full" },
                agent_id,
                member_grant_note
                    .as_deref()
                    .map(|n| format!(" [{n}]"))
                    .unwrap_or_default()
            ));
        }

        // Register team
        self.team_manager
            .create_team(team_name.clone(), spawned_ids.clone());

        Ok(ToolResult::success(format!(
            "Created team '{}' with {} agents:\n{}",
            team_name,
            spawned_ids.len(),
            spawn_results.join("\n")
        )))
    }
}
