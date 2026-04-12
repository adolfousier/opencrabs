//! Self-Improve Tool — Recursive Self-Improvement (RSI)
//!
//! Autonomously applies improvements to brain files based on feedback analysis.
//! Writes to ~/.opencrabs/rsi/ directory — no human approval required.
//! Each improvement is logged to rsi/improvements.md and archived daily in rsi/history/.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::io::Write;

/// Ensures the RSI directory structure exists.
fn ensure_rsi_dirs(home: &std::path::Path) -> std::io::Result<()> {
    let rsi_dir = home.join("rsi");
    let history_dir = rsi_dir.join("history");
    std::fs::create_dir_all(&history_dir)
}

pub struct SelfImproveTool;

#[async_trait]
impl Tool for SelfImproveTool {
    fn name(&self) -> &str {
        "self_improve"
    }

    fn description(&self) -> &str {
        "Autonomously apply self-improvements based on feedback analysis. \
         Modifies brain files (SOUL.md, AGENTS.md, etc.) and logs changes to \
         ~/.opencrabs/rsi/improvements.md. No human approval needed — the agent \
         identifies patterns via feedback_analyze and applies fixes directly. \
         Use feedback_analyze first to identify what needs improvement."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "What to do: 'apply' (edit a brain file + log), 'list' (show applied improvements)",
                    "enum": ["apply", "list"]
                },
                "target_file": {
                    "type": "string",
                    "description": "For 'apply': brain file to modify (e.g. 'SOUL.md', 'AGENTS.md', 'TOOLS.md'). Must be a known brain file."
                },
                "description": {
                    "type": "string",
                    "description": "For 'apply': human-readable description of the improvement"
                },
                "rationale": {
                    "type": "string",
                    "description": "For 'apply': why this improvement is needed (reference feedback data)"
                },
                "content": {
                    "type": "string",
                    "description": "For 'apply': the new content to append to the target brain file"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::WriteFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Autonomous — no human-in-the-loop
    }

    fn requires_approval_for_input(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");

        let home = crate::config::opencrabs_home();

        match action {
            "list" => {
                let improvements_path = home.join("rsi").join("improvements.md");
                if !improvements_path.exists() {
                    return Ok(ToolResult::success(
                        "No improvements recorded yet. Run self_improve with action='apply' to start.".to_string(),
                    ));
                }
                match std::fs::read_to_string(&improvements_path) {
                    Ok(content) => Ok(ToolResult::success(content)),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to read rsi/improvements.md: {e}"
                    ))),
                }
            }

            "apply" => {
                let target_file = input
                    .get("target_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let description = input
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let rationale = input
                    .get("rationale")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");

                if target_file.is_empty() || content.is_empty() || description.is_empty() {
                    return Ok(ToolResult::error(
                        "target_file, description, and content are required for 'apply'"
                            .to_string(),
                    ));
                }

                // Validate target is a known brain file
                const ALLOWED_FILES: &[&str] = &[
                    "SOUL.md",
                    "USER.md",
                    "AGENTS.md",
                    "TOOLS.md",
                    "CODE.md",
                    "SECURITY.md",
                    "MEMORY.md",
                    "BOOT.md",
                    "IDENTITY.md",
                ];
                if !ALLOWED_FILES.contains(&target_file) {
                    return Ok(ToolResult::error(format!(
                        "target_file must be one of: {}",
                        ALLOWED_FILES.join(", ")
                    )));
                }

                // Ensure RSI dirs exist
                ensure_rsi_dirs(&home).map_err(|e| {
                    crate::brain::tools::ToolError::Execution(format!(
                        "Failed to create RSI directories: {e}"
                    ))
                })?;

                // Append content to target brain file
                let target_path = home.join(target_file);
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&target_path)
                    .map_err(|e| {
                        crate::brain::tools::ToolError::Execution(format!(
                            "Failed to open {target_file}: {e}"
                        ))
                    })?;
                file.write_all(format!("\n{content}\n").as_bytes())
                    .map_err(|e| {
                        crate::brain::tools::ToolError::Execution(format!(
                            "Failed to write {target_file}: {e}"
                        ))
                    })?;

                // Log to rsi/improvements.md
                let entry = format!(
                    "\n## [Applied] {}\n\n**Date:** {}\n**Target:** {}\n**Rationale:** {}\n**Status:** Applied\n",
                    description,
                    chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
                    target_file,
                    if rationale.is_empty() {
                        "(none)"
                    } else {
                        rationale
                    },
                );
                let improvements_path = home.join("rsi").join("improvements.md");
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&improvements_path)
                {
                    let _ = f.write_all(entry.as_bytes());
                }

                // Archive to daily history file
                let history_path = home
                    .join("rsi")
                    .join("history")
                    .join(format!("{}.md", chrono::Utc::now().format("%Y-%m-%d")));
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&history_path)
                {
                    let _ = f.write_all(format!("\n### {description}\n\n{content}\n").as_bytes());
                }

                // Record in feedback ledger
                if let Some(ref svc_ctx) = context.service_context {
                    let repo = crate::db::repository::FeedbackLedgerRepository::new(
                        svc_ctx.pool().clone(),
                    );
                    let meta = serde_json::json!({
                        "target_file": target_file,
                        "rationale": rationale,
                    })
                    .to_string();
                    let _ = repo
                        .record(
                            &context.session_id.to_string(),
                            "improvement_applied",
                            description,
                            1.0,
                            Some(&meta),
                        )
                        .await;
                }

                Ok(ToolResult::success(format!(
                    "Improvement applied to {target_file} and logged to rsi/improvements.md: {description}"
                )))
            }

            other => Ok(ToolResult::error(format!(
                "Unknown action: '{other}'. Use 'propose', 'apply', or 'list'."
            ))),
        }
    }
}
