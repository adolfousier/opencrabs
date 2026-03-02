//! Write OpenCrabs File Tool
//!
//! Writes or edits any file within `~/.opencrabs/` — brain files (MEMORY.md,
//! USER.md, etc.), config files (commands.toml), memory logs, and any other
//! app-owned files. The standard `edit_file`/`write_file` tools are restricted
//! to the working directory and cannot reach `~/.opencrabs/`; use this tool
//! instead.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

pub struct WriteOpenCrabsFileTool;

/// Validate that `path` is a safe relative path within `~/.opencrabs/`.
/// Prevents path traversal outside the app home directory.
pub(super) fn validate_opencrabs_path(path: &str) -> std::result::Result<(), String> {
    if path.is_empty() {
        return Err("path is required".into());
    }
    // Reject absolute paths — must be relative to ~/.opencrabs/
    if path.starts_with('/') || path.starts_with('~') {
        return Err(format!(
            "Use a relative path (e.g. \"MEMORY.md\" or \"memory/2026-03-02.md\"), \
             not an absolute path '{}'",
            path
        ));
    }
    // Reject traversal attempts
    if path.contains("..") {
        return Err(format!(
            "'{}' contains '..' — path traversal is not allowed",
            path
        ));
    }
    // Reject null bytes
    if path.contains('\0') {
        return Err("path contains null bytes".into());
    }
    Ok(())
}

#[async_trait]
impl Tool for WriteOpenCrabsFileTool {
    fn name(&self) -> &str {
        "write_opencrabs_file"
    }

    fn description(&self) -> &str {
        "Write or edit any file within the OpenCrabs home directory (~/.opencrabs/). \
         Use this for brain files (MEMORY.md, USER.md, AGENTS.md, SOUL.md, etc.), \
         config files (commands.toml), memory logs, and any other app files. \
         The standard edit_file/write_file tools cannot reach ~/.opencrabs/ — use this instead. \
         Provide a relative path (e.g. \"MEMORY.md\" or \"memory/note.md\"). \
         Supports three operations: \
         \"overwrite\" replaces entire file content, \
         \"append\" adds text to the end, \
         \"replace\" does a find-and-replace within the file."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path within ~/.opencrabs/ (e.g. \"MEMORY.md\", \"memory/2026-03-02.md\", \"commands.toml\"). No leading slash, no '..'."
                },
                "operation": {
                    "type": "string",
                    "enum": ["overwrite", "append", "replace"],
                    "description": "\"overwrite\": replace entire file. \"append\": add to end. \"replace\": find old_text and replace with new_text."
                },
                "content": {
                    "type": "string",
                    "description": "Content to write (required for overwrite and append)."
                },
                "old_text": {
                    "type": "string",
                    "description": "Text to find (required for replace)."
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text (required for replace)."
                }
            },
            "required": ["path", "operation"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::WriteFiles]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, _ctx: &ToolExecutionContext) -> Result<ToolResult> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if let Err(e) = validate_opencrabs_path(path_str) {
            return Ok(ToolResult::error(e));
        }

        let operation = input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        let home = crate::config::opencrabs_home();
        let full_path = home.join(path_str);

        match operation {
            "overwrite" => {
                let content = match input.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c,
                    None => {
                        return Ok(ToolResult::error(
                            "content is required for overwrite".into(),
                        ));
                    }
                };
                if let Some(parent) = full_path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    return Ok(ToolResult::error(format!(
                        "Failed to create directory: {}",
                        e
                    )));
                }
                match std::fs::write(&full_path, content) {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Wrote {} bytes to ~/.opencrabs/{}",
                        content.len(),
                        path_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to write {}: {}",
                        path_str, e
                    ))),
                }
            }

            "append" => {
                let content = match input.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c,
                    None => return Ok(ToolResult::error("content is required for append".into())),
                };
                if let Some(parent) = full_path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    return Ok(ToolResult::error(format!(
                        "Failed to create directory: {}",
                        e
                    )));
                }
                use std::io::Write;
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&full_path)
                {
                    Ok(mut f) => match f.write_all(content.as_bytes()) {
                        Ok(()) => Ok(ToolResult::success(format!(
                            "Appended {} bytes to ~/.opencrabs/{}",
                            content.len(),
                            path_str
                        ))),
                        Err(e) => Ok(ToolResult::error(format!(
                            "Failed to append to {}: {}",
                            path_str, e
                        ))),
                    },
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to open {}: {}",
                        path_str, e
                    ))),
                }
            }

            "replace" => {
                let old_text = match input.get("old_text").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => {
                        return Ok(ToolResult::error("old_text is required for replace".into()));
                    }
                };
                let new_text = match input.get("new_text").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => {
                        return Ok(ToolResult::error("new_text is required for replace".into()));
                    }
                };
                let existing = match std::fs::read_to_string(&full_path) {
                    Ok(s) => s,
                    Err(_) => {
                        return Ok(ToolResult::error(format!(
                            "~/.opencrabs/{} not found. Use overwrite to create it.",
                            path_str
                        )));
                    }
                };
                if !existing.contains(old_text) {
                    return Ok(ToolResult::error(format!(
                        "old_text not found in {}. No changes made.",
                        path_str
                    )));
                }
                let updated = existing.replacen(old_text, new_text, 1);
                match std::fs::write(&full_path, &updated) {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Replaced text in ~/.opencrabs/{}",
                        path_str
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to write {}: {}",
                        path_str, e
                    ))),
                }
            }

            other => Ok(ToolResult::error(format!(
                "Unknown operation '{}'. Use: overwrite, append, replace.",
                other
            ))),
        }
    }
}

#[cfg(test)]
#[path = "write_opencrabs_file_tests.rs"]
mod write_opencrabs_file_tests;
