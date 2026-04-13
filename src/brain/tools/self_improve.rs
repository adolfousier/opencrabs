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

/// Known brain files that the RSI tool is allowed to read/modify.
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
                    "description": "What to do:\n\
                        - 'read': Read a brain file BEFORE modifying it. ALWAYS do this first.\n\
                        - 'apply': Append NEW content to a brain file (only for genuinely new instructions).\n\
                        - 'update': Surgically replace an existing section/paragraph. Use when an existing instruction needs refinement rather than a new one added.\n\
                        - 'list': Show previously applied improvements.",
                    "enum": ["read", "apply", "update", "list"]
                },
                "target_file": {
                    "type": "string",
                    "description": "Brain file to read/modify (e.g. 'SOUL.md', 'TOOLS.md'). Must be a known brain file."
                },
                "description": {
                    "type": "string",
                    "description": "For 'apply'/'update': human-readable description of the improvement"
                },
                "rationale": {
                    "type": "string",
                    "description": "For 'apply'/'update': why this improvement is needed (reference feedback data)"
                },
                "content": {
                    "type": "string",
                    "description": "For 'apply': new content to append. For 'update': the replacement content."
                },
                "old_content": {
                    "type": "string",
                    "description": "For 'update' only: the existing text to find and replace (must be an exact match of the current content)."
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

        // Brain files MUST go to ~/.opencrabs/, never the repo working directory.
        // Tests can override via working_directory pointing to a temp dir, but
        // only if it looks like an opencrabs home (contains "opencrabs" or is a
        // temp dir), NOT a git repo root.
        let home = if !context.working_directory.as_os_str().is_empty()
            && context.working_directory != std::path::Path::new(".")
            && !context.working_directory.join(".git").exists()
        {
            context.working_directory.clone()
        } else {
            crate::config::opencrabs_home()
        };

        match action {
            "read" => {
                let target_file = input
                    .get("target_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if target_file.is_empty() {
                    return Ok(ToolResult::error(
                        "target_file is required for 'read'".to_string(),
                    ));
                }
                if !ALLOWED_FILES.contains(&target_file) {
                    return Ok(ToolResult::error(format!(
                        "target_file must be one of: {}",
                        ALLOWED_FILES.join(", ")
                    )));
                }

                let target_path = home.join(target_file);
                if !target_path.exists() {
                    return Ok(ToolResult::success(format!(
                        "{target_file} does not exist yet (empty). \
                         You can create it with action='apply'."
                    )));
                }
                match std::fs::read_to_string(&target_path) {
                    Ok(content) => Ok(ToolResult::success(format!(
                        "--- {target_file} ({} bytes) ---\n{content}",
                        content.len()
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to read {target_file}: {e}"
                    ))),
                }
            }

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

            "update" => {
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
                let old_content = input
                    .get("old_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new_content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");

                if target_file.is_empty()
                    || old_content.is_empty()
                    || new_content.is_empty()
                    || description.is_empty()
                {
                    return Ok(ToolResult::error(
                        "target_file, description, old_content, and content are all required for 'update'"
                            .to_string(),
                    ));
                }
                if !ALLOWED_FILES.contains(&target_file) {
                    return Ok(ToolResult::error(format!(
                        "target_file must be one of: {}",
                        ALLOWED_FILES.join(", ")
                    )));
                }

                let target_path = home.join(target_file);
                let existing = match std::fs::read_to_string(&target_path) {
                    Ok(c) => c,
                    Err(_) => {
                        return Ok(ToolResult::error(format!(
                            "{target_file} does not exist — use 'apply' to create new content instead."
                        )));
                    }
                };

                // Find the old_content in the file (exact substring match).
                // The agent is responsible for providing an accurate old_content
                // snippet after reading the file with action='read'.
                if !existing.contains(old_content) {
                    return Ok(ToolResult::error(format!(
                        "old_content not found in {target_file}. \
                         Use action='read' first to get the exact current content, \
                         then copy the section you want to replace verbatim into old_content."
                    )));
                }

                // Perform the replacement (first occurrence only)
                let updated = existing.replacen(old_content, new_content.trim(), 1);

                // Ensure RSI dirs exist for logging
                ensure_rsi_dirs(&home).map_err(|e| {
                    crate::brain::tools::ToolError::Execution(format!(
                        "Failed to create RSI directories: {e}"
                    ))
                })?;

                // Write the updated file
                std::fs::write(&target_path, updated.as_bytes()).map_err(|e| {
                    crate::brain::tools::ToolError::Execution(format!(
                        "Failed to write {target_file}: {e}"
                    ))
                })?;

                // Log to rsi/improvements.md
                let entry = format!(
                    "\n## [Updated] {}\n\n**Date:** {}\n**Target:** {}\n**Rationale:** {}\n**Status:** Updated (surgical replace)\n",
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
                    let _ = f.write_all(
                        format!(
                            "\n### [Updated] {description}\n\n**Replaced:**\n```\n{old_content}\n```\n**With:**\n```\n{new_content}\n```\n"
                        )
                        .as_bytes(),
                    );
                }

                // Record in feedback ledger
                if let Some(ref svc_ctx) = context.service_context {
                    let repo = crate::db::repository::FeedbackLedgerRepository::new(
                        svc_ctx.pool().clone(),
                    );
                    let meta = serde_json::json!({
                        "target_file": target_file,
                        "rationale": rationale,
                        "action": "update",
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
                    "Surgically updated {target_file} and logged to rsi/improvements.md: {description}"
                )))
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

                let target_path = home.join(target_file);

                // Append content to target brain file
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&target_path)
                    .map_err(|e| {
                        crate::brain::tools::ToolError::Execution(format!(
                            "Failed to open {target_file}: {e}"
                        ))
                    })?;
                file.write_all(format!("\n{}\n", content.trim()).as_bytes())
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
                "Unknown action: '{other}'. Use 'read', 'apply', 'update', or 'list'."
            ))),
        }
    }
}
