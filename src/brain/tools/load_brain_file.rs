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
        "Load any .md file from the OpenCrabs home directory (~/.opencrabs/). \
         Works with built-in files (USER.md, MEMORY.md, AGENTS.md, TOOLS.md, SECURITY.md) \
         and user-created files (VOICE.md, custom notes, etc.). \
         Pass name=\"all\" to load all .md files at once. \
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
            let mut seen = std::collections::HashSet::new();

            // Known contextual files first (stable order)
            for (fname, label) in CONTEXTUAL_BRAIN_FILES {
                seen.insert(fname.to_lowercase());
                let path = home.join(fname);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        out.push_str(&format!("--- {} ({}) ---\n{}\n\n", fname, label, trimmed));
                    }
                }
            }

            // User-created .md files not in the known list
            if let Ok(entries) = std::fs::read_dir(&home) {
                let mut extras: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        name.ends_with(".md") && !seen.contains(&name.to_lowercase())
                    })
                    .collect();
                extras.sort_by_key(|e| e.file_name());
                for entry in extras {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            out.push_str(&format!("--- {} (user) ---\n{}\n\n", fname, trimmed));
                        }
                    }
                }
            }

            return if out.is_empty() {
                Ok(ToolResult::success("No brain files found.".to_string()))
            } else {
                Ok(ToolResult::success(out))
            };
        }

        // Validate filename: must be a simple .md name (no path traversal)
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Ok(ToolResult::error(format!(
                "Invalid brain file name '{}'. Must be a simple filename (e.g. VOICE.md)",
                name
            )));
        }

        // Use canonical casing from the known list if it matches, otherwise use as-is
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
