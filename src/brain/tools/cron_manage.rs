//! Cron Manage Tool
//!
//! Allows the agent to create, list, delete, enable, and disable cron jobs.
//! Jobs run in isolated sessions with configurable provider/model/thinking.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::db::CronJobRepository;
use crate::db::models::CronJob;
use async_trait::async_trait;
use serde_json::Value;

/// Tool for managing cron jobs via the agent.
pub struct CronManageTool {
    repo: CronJobRepository,
}

impl CronManageTool {
    pub fn new(repo: CronJobRepository) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl Tool for CronManageTool {
    fn name(&self) -> &str {
        "cron_manage"
    }

    fn description(&self) -> &str {
        "Manage scheduled cron jobs. Jobs run in isolated sessions with configurable provider/model. \
         Use 'create' to schedule a new job, 'list' to see all jobs, 'delete' to remove one, \
         'enable'/'disable' to toggle a job without deleting it."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "delete", "enable", "disable", "test"],
                    "description": "Action to perform. 'test' triggers a job immediately (runs on next scheduler tick within 60s)"
                },
                "name": {
                    "type": "string",
                    "description": "Job name (required for create)"
                },
                "cron": {
                    "type": "string",
                    "description": "Cron expression, 5-field (min hour dom mon dow). Required for create. Examples: '0 9 * * *' (daily 9am), '*/30 * * * *' (every 30min)"
                },
                "tz": {
                    "type": "string",
                    "description": "Timezone (default: UTC). Examples: America/New_York, Europe/London"
                },
                "prompt": {
                    "type": "string",
                    "description": "Instructions for the agent to execute (required for create)"
                },
                "provider": {
                    "type": "string",
                    "description": "Override provider (e.g. 'anthropic', 'openai'). Omit for current default"
                },
                "model": {
                    "type": "string",
                    "description": "Override model (e.g. 'claude-sonnet-4-20250514'). Omit for provider default"
                },
                "thinking": {
                    "type": "string",
                    "enum": ["off", "on", "budget"],
                    "description": "Thinking mode (default: off)"
                },
                "auto_approve": {
                    "type": "boolean",
                    "description": "Auto-approve tool executions (default: true for cron)"
                },
                "deliver_to": {
                    "type": "string",
                    "description": "Where to deliver results. Format: 'telegram:chat_id', 'discord:channel_id', 'slack:channel_id', or an HTTP(S) URL for webhook delivery"
                },
                "job_id": {
                    "type": "string",
                    "description": "Job ID (required for delete/enable/disable)"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Whether the job is enabled (for create, default: true)"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval_for_input(&self, input: &Value) -> bool {
        // Only create and delete need approval; list/enable/disable are safe
        matches!(
            input.get("action").and_then(|v| v.as_str()),
            Some("create") | Some("delete") | Some("test")
        )
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        match action {
            "create" => self.create_job(&input).await,
            "list" => self.list_jobs().await,
            "delete" => self.delete_job(&input).await,
            "enable" => self.toggle_job(&input, true).await,
            "disable" => self.toggle_job(&input, false).await,
            "test" => self.test_job(&input).await,
            unknown => Ok(ToolResult::error(format!(
                "Unknown action '{unknown}'. Valid: create, list, delete, enable, disable, test"
            ))),
        }
    }
}

impl CronManageTool {
    async fn create_job(&self, input: &Value) -> Result<ToolResult> {
        let name = match input.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.is_empty() => n,
            _ => {
                return Ok(ToolResult::error(
                    "'name' is required for create".to_string(),
                ));
            }
        };

        let cron_expr = match input.get("cron").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c,
            _ => {
                return Ok(ToolResult::error(
                    "'cron' expression is required for create".to_string(),
                ));
            }
        };

        // Validate cron expression (user provides 5-field, we prepend "0" for seconds)
        let cron_with_secs = format!("0 {cron_expr}");
        if let Err(e) = cron_with_secs.parse::<cron::Schedule>() {
            return Ok(ToolResult::error(format!(
                "Invalid cron expression '{cron_expr}': {e}. Use 5-field format: 'min hour dom mon dow'. Example: '0 9 * * *' for daily at 9am."
            )));
        }

        let prompt = match input.get("prompt").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => {
                return Ok(ToolResult::error(
                    "'prompt' is required for create".to_string(),
                ));
            }
        };

        // Check for duplicate name
        if let Ok(Some(_)) = self.repo.find_by_name(name).await {
            return Ok(ToolResult::error(format!(
                "A cron job named '{name}' already exists. Use a different name or delete the existing one first."
            )));
        }

        let tz = input
            .get("tz")
            .and_then(|v| v.as_str())
            .unwrap_or("UTC")
            .to_string();
        let provider = input
            .get("provider")
            .and_then(|v| v.as_str())
            .map(String::from);
        let model = input
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from);
        let thinking = input
            .get("thinking")
            .and_then(|v| v.as_str())
            .unwrap_or("off")
            .to_string();
        let auto_approve = input
            .get("auto_approve")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let deliver_to = input
            .get("deliver_to")
            .and_then(|v| v.as_str())
            .map(String::from);

        let job = CronJob::new(
            name.to_string(),
            cron_expr.to_string(),
            tz,
            prompt.to_string(),
            provider,
            model,
            thinking,
            auto_approve,
            deliver_to.clone(),
        );

        let job_id = job.id.to_string();

        self.repo
            .insert(&job)
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        let delivery = deliver_to
            .as_deref()
            .unwrap_or("none (results logged only)");

        Ok(ToolResult::success(format!(
            "Cron job created:\n  ID: {job_id}\n  Name: {name}\n  Schedule: {cron_expr}\n  Timezone: {}\n  Deliver to: {delivery}\n  Enabled: true",
            job.timezone
        )))
    }

    async fn list_jobs(&self) -> Result<ToolResult> {
        let jobs = self
            .repo
            .list_all()
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        if jobs.is_empty() {
            return Ok(ToolResult::success("No cron jobs configured.".to_string()));
        }

        let lines: Vec<String> = jobs
            .iter()
            .map(|j| {
                let status = if j.enabled { "enabled" } else { "disabled" };
                let deliver = j.deliver_to.as_deref().unwrap_or("none");
                let last = j
                    .last_run_at
                    .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| "never".to_string());
                format!(
                    "- [{}] {} (id={})\n    Schedule: {} ({})\n    Deliver: {}\n    Last run: {}\n    Prompt: {}",
                    status,
                    j.name,
                    j.id,
                    j.cron_expr,
                    j.timezone,
                    deliver,
                    last,
                    truncate(&j.prompt, 80),
                )
            })
            .collect();

        Ok(ToolResult::success(format!(
            "Cron jobs ({}):\n{}",
            jobs.len(),
            lines.join("\n")
        )))
    }

    async fn delete_job(&self, input: &Value) -> Result<ToolResult> {
        let job_id = match input.get("job_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id,
            _ => {
                return Ok(ToolResult::error(
                    "'job_id' is required for delete".to_string(),
                ));
            }
        };

        let deleted = self
            .repo
            .delete(job_id)
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        if deleted {
            Ok(ToolResult::success(format!("Cron job {job_id} deleted.")))
        } else {
            Ok(ToolResult::error(format!(
                "No cron job found with ID '{job_id}'."
            )))
        }
    }

    async fn toggle_job(&self, input: &Value, enabled: bool) -> Result<ToolResult> {
        let job_id = match input.get("job_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id,
            _ => {
                return Ok(ToolResult::error(
                    "'job_id' is required for enable/disable".to_string(),
                ));
            }
        };

        let updated = self
            .repo
            .set_enabled(job_id, enabled)
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        if updated {
            let state = if enabled { "enabled" } else { "disabled" };
            Ok(ToolResult::success(format!("Cron job {job_id} {state}.")))
        } else {
            Ok(ToolResult::error(format!(
                "No cron job found with ID '{job_id}'."
            )))
        }
    }

    async fn test_job(&self, input: &Value) -> Result<ToolResult> {
        let job_id = match input.get("job_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id,
            _ => {
                // Also accept name
                match input.get("name").and_then(|v| v.as_str()) {
                    Some(name) if !name.is_empty() => {
                        if let Ok(Some(job)) = self.repo.find_by_name(name).await {
                            return self.trigger_by_id(&job.id.to_string(), &job.name).await;
                        }
                        return Ok(ToolResult::error(format!(
                            "No cron job found with name '{name}'."
                        )));
                    }
                    _ => {
                        return Ok(ToolResult::error(
                            "'job_id' or 'name' is required for test".to_string(),
                        ));
                    }
                }
            }
        };

        // Try ID first, then name
        if let Ok(Some(job)) = self.repo.find_by_id(job_id).await {
            return self.trigger_by_id(&job.id.to_string(), &job.name).await;
        }
        if let Ok(Some(job)) = self.repo.find_by_name(job_id).await {
            return self.trigger_by_id(&job.id.to_string(), &job.name).await;
        }

        Ok(ToolResult::error(format!(
            "No cron job found with ID or name '{job_id}'."
        )))
    }

    async fn trigger_by_id(&self, id: &str, name: &str) -> Result<ToolResult> {
        let triggered = self
            .repo
            .trigger_now(id)
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        if triggered {
            Ok(ToolResult::success(format!(
                "Cron job '{name}' (id={id}) triggered. It will execute on the next scheduler tick (within 60 seconds). Check logs for execution status."
            )))
        } else {
            Ok(ToolResult::error(format!(
                "Failed to trigger cron job '{id}'."
            )))
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max).collect::<String>())
    }
}
