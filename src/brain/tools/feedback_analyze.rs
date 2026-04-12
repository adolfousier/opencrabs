//! Feedback Analyze Tool
//!
//! Queries the feedback ledger and returns aggregated stats, trends, and
//! patterns for the agent to reason about self-improvement opportunities.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

pub struct FeedbackAnalyzeTool;

#[async_trait]
impl Tool for FeedbackAnalyzeTool {
    fn name(&self) -> &str {
        "feedback_analyze"
    }

    fn description(&self) -> &str {
        "Analyze the feedback ledger to identify patterns, success rates, and \
         improvement opportunities. Returns aggregated stats on tool performance, \
         failure patterns, and trends. Use this to understand what's working well \
         and what needs improvement before proposing changes via self_improve."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to analyze: 'summary' (overall stats), 'tool_stats' (per-tool success rates), 'recent' (last N events), 'failures' (recent failures only)",
                    "enum": ["summary", "tool_stats", "recent", "failures"]
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results for 'recent' and 'failures' queries",
                    "default": 50
                }
            },
            "required": ["query"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![] // Read-only DB access
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("summary");
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as u32;

        let Some(ref svc_ctx) = context.service_context else {
            return Ok(ToolResult::error(
                "No service context — feedback analysis requires a database connection".to_string(),
            ));
        };

        let repo =
            crate::db::repository::FeedbackLedgerRepository::new(svc_ctx.pool().clone());

        match query {
            "summary" => {
                let total = repo.total_count().await.map_err(|e| {
                    crate::brain::tools::ToolError::Execution(e.to_string())
                })?;
                let breakdown = repo.summary().await.map_err(|e| {
                    crate::brain::tools::ToolError::Execution(e.to_string())
                })?;

                if total == 0 {
                    return Ok(ToolResult::success(
                        "No feedback data yet. The feedback ledger is empty — \
                         observations accumulate as you use tools and interact with users. \
                         Use feedback_record to manually log patterns you notice."
                            .to_string(),
                    ));
                }

                let mut out = format!("Feedback Ledger Summary ({total} total events)\n\n");
                out.push_str("Event Type Breakdown:\n");
                for (event_type, count) in &breakdown {
                    let pct = (*count as f64 / total as f64) * 100.0;
                    out.push_str(&format!("  {event_type}: {count} ({pct:.1}%)\n"));
                }
                Ok(ToolResult::success(out))
            }

            "tool_stats" => {
                let stats = repo.stats_by_dimension("tool_").await.map_err(|e| {
                    crate::brain::tools::ToolError::Execution(e.to_string())
                })?;

                if stats.is_empty() {
                    return Ok(ToolResult::success(
                        "No tool execution data yet.".to_string(),
                    ));
                }

                let mut out = String::from("Tool Performance Stats:\n\n");
                out.push_str(&format!(
                    "{:<20} {:>6} {:>6} {:>6} {:>8}\n",
                    "Tool", "Total", "OK", "Fail", "Rate"
                ));
                out.push_str(&"-".repeat(50));
                out.push('\n');
                for s in &stats {
                    out.push_str(&format!(
                        "{:<20} {:>6} {:>6} {:>6} {:>7.1}%\n",
                        s.dimension,
                        s.total_events,
                        s.successes,
                        s.failures,
                        s.success_rate * 100.0
                    ));
                }
                Ok(ToolResult::success(out))
            }

            "recent" => {
                let entries = repo.recent(limit).await.map_err(|e| {
                    crate::brain::tools::ToolError::Execution(e.to_string())
                })?;

                if entries.is_empty() {
                    return Ok(ToolResult::success("No recent feedback.".to_string()));
                }

                let mut out = format!("Recent Feedback ({} entries):\n\n", entries.len());
                for e in &entries {
                    out.push_str(&format!(
                        "[{}] {}/{} = {} {}\n",
                        e.created_at.format("%Y-%m-%d %H:%M"),
                        e.event_type,
                        e.dimension,
                        e.value,
                        e.metadata.as_deref().unwrap_or("")
                    ));
                }
                Ok(ToolResult::success(out))
            }

            "failures" => {
                let entries = repo.by_event_type("tool_failure", limit).await.map_err(|e| {
                    crate::brain::tools::ToolError::Execution(e.to_string())
                })?;

                if entries.is_empty() {
                    return Ok(ToolResult::success(
                        "No tool failures recorded.".to_string(),
                    ));
                }

                let mut out = format!("Recent Failures ({} entries):\n\n", entries.len());
                for e in &entries {
                    out.push_str(&format!(
                        "[{}] {} — {}\n",
                        e.created_at.format("%Y-%m-%d %H:%M"),
                        e.dimension,
                        e.metadata.as_deref().unwrap_or("(no details)")
                    ));
                }
                Ok(ToolResult::success(out))
            }

            other => Ok(ToolResult::error(format!(
                "Unknown query type: '{other}'. Use 'summary', 'tool_stats', 'recent', or 'failures'."
            ))),
        }
    }
}
