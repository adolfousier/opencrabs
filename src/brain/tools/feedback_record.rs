//! Feedback Record Tool
//!
//! Records observations to the feedback ledger for recursive self-improvement.
//! Called by the agent to log tool outcomes, user corrections, and performance signals.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

pub struct FeedbackRecordTool;

#[async_trait]
impl Tool for FeedbackRecordTool {
    fn name(&self) -> &str {
        "feedback_record"
    }

    fn description(&self) -> &str {
        "Record an observation to the feedback ledger for self-improvement. \
         Use this to log patterns you notice: recurring failures, user corrections, \
         successful strategies, or areas needing improvement. Events accumulate over \
         time and are analyzed by feedback_analyze."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "event_type": {
                    "type": "string",
                    "description": "Category: 'tool_success', 'tool_failure', 'user_correction', 'provider_error', 'context_compaction', 'improvement_applied', 'pattern_observed'",
                    "enum": ["tool_success", "tool_failure", "user_correction", "provider_error", "context_compaction", "improvement_applied", "pattern_observed"]
                },
                "dimension": {
                    "type": "string",
                    "description": "What was observed — tool name, provider name, pattern label, etc."
                },
                "value": {
                    "type": "number",
                    "description": "Numeric signal: 1.0 = success, 0.0 = failure, or duration_ms, count, etc.",
                    "default": 1.0
                },
                "metadata": {
                    "type": "string",
                    "description": "Optional JSON or free-text context about the observation"
                }
            },
            "required": ["event_type", "dimension"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![] // No dangerous capabilities — just DB writes
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let event_type = input
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let dimension = input
            .get("dimension")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let value = input.get("value").and_then(|v| v.as_f64()).unwrap_or(1.0);
        let metadata = input
            .get("metadata")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if event_type.is_empty() || dimension.is_empty() {
            return Ok(ToolResult::error(
                "event_type and dimension are required".to_string(),
            ));
        }

        let Some(ref svc_ctx) = context.service_context else {
            return Ok(ToolResult::error(
                "No service context available — feedback recording requires a database connection"
                    .to_string(),
            ));
        };

        let repo = crate::db::repository::FeedbackLedgerRepository::new(svc_ctx.pool().clone());

        let session_id = context.session_id.to_string();
        match repo
            .record(
                &session_id,
                &event_type,
                &dimension,
                value,
                metadata.as_deref(),
            )
            .await
        {
            Ok(id) => Ok(ToolResult::success(format!(
                "Recorded feedback #{id}: {event_type}/{dimension} = {value}"
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to record feedback: {e}"))),
        }
    }
}
