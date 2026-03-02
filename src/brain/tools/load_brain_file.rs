//! Load Brain File Tool
//!
//! Loads a specific brain context file from `~/.opencrabs/` on demand.
//! Use this to fetch USER.md, MEMORY.md, AGENTS.md, etc. only when the
//! current request actually needs that context, rather than injecting all
//! files into every turn.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

use crate::brain::prompt_builder::CONTEXTUAL_BRAIN_FILES;

pub struct LoadBrainFileTool;

#[async_trait]
impl Tool for LoadBrainFileTool {
    fn name(&self) -> &str {
        "load_brain_file"
    }

    fn description(&self) -> &str {
        "Load a specific brain context file from the OpenCrabs home directory (~/.opencrabs/). \
         Use this to retrieve USER.md, MEMORY.md, AGENTS.md, TOOLS.md, SECURITY.md, etc. \
         on demand — only when the current request actually needs that context. \
         Pass name=\"all\" to load all available contextual files at once. \
         To edit or update brain files, use the `write_opencrabs_file` tool."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Brain file to load, e.g. \"MEMORY.md\", \"USER.md\", \"AGENTS.md\", \"TOOLS.md\", \"SECURITY.md\". Use \"all\" to load all contextual files."
                }
            },
            "required": ["name"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, _ctx: &ToolExecutionContext) -> Result<ToolResult> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if name.is_empty() {
            return Ok(ToolResult::error("name parameter is required".to_string()));
        }

        let home = crate::config::opencrabs_home();

        if name == "all" {
            let mut out = String::new();
            for (fname, label) in CONTEXTUAL_BRAIN_FILES {
                let path = home.join(fname);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        out.push_str(&format!("--- {} ({}) ---\n{}\n\n", fname, label, trimmed));
                    }
                }
            }
            return if out.is_empty() {
                Ok(ToolResult::success(
                    "No contextual brain files found.".to_string(),
                ))
            } else {
                Ok(ToolResult::success(out))
            };
        }

        // Validate name against the known list (prevents path traversal)
        let is_known = CONTEXTUAL_BRAIN_FILES
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(name));

        if !is_known {
            let known: Vec<&str> = CONTEXTUAL_BRAIN_FILES.iter().map(|(n, _)| *n).collect();
            return Ok(ToolResult::error(format!(
                "Unknown brain file '{}'. Valid options: {} (or \"all\")",
                name,
                known.join(", ")
            )));
        }

        // Use the canonical casing from the list
        let canonical = CONTEXTUAL_BRAIN_FILES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(n, _)| *n)
            .unwrap_or(name);

        let path = home.join(canonical);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    Ok(ToolResult::success(format!(
                        "{} exists but is empty.",
                        canonical
                    )))
                } else {
                    Ok(ToolResult::success(format!(
                        "--- {} ---\n{}",
                        canonical, trimmed
                    )))
                }
            }
            Err(_) => Ok(ToolResult::success(format!(
                "{} not found at ~/.opencrabs/{}. No content available.",
                canonical, canonical
            ))),
        }
    }
}

#[cfg(test)]
#[path = "load_brain_file_tests.rs"]
mod load_brain_file_tests;
