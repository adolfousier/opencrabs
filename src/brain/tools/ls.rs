//! List Directory Tool
//!
//! List contents of directories for exploration.

use super::error::{Result, ToolError, resolve_tool_path};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::fs;

/// List directory tool
pub struct LsTool;

#[derive(Debug, Deserialize, Serialize)]
struct LsInput {
    /// Path to list (defaults to current working directory)
    #[serde(default)]
    path: Option<String>,

    /// Show hidden files (starting with .)
    #[serde(default)]
    show_hidden: bool,

    /// Show detailed information (size, modified time)
    #[serde(default)]
    detailed: bool,

    /// Recursive listing
    #[serde(default)]
    recursive: bool,
}

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "List contents of a directory. Shows files and subdirectories with optional details."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path to list (defaults to current working directory)"
                },
                "show_hidden": {
                    "type": "boolean",
                    "description": "Include hidden files (starting with .)",
                    "default": false
                },
                "detailed": {
                    "type": "boolean",
                    "description": "Show detailed information (size, modified time)",
                    "default": false
                },
                "recursive": {
                    "type": "boolean",
                    "description": "List subdirectories recursively",
                    "default": false
                }
            }
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Listing directories is safe
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: LsInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: LsInput = serde_json::from_value(input)?;

        // Resolve path (expands leading `~`, absolutes pass through,
        // relatives join to the session working directory).
        let path = if let Some(ref p) = input.path {
            resolve_tool_path(p, &context.working_directory)
        } else {
            context.working_directory.clone()
        };

        // Check if path exists
        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "Path does not exist: {}",
                path.display()
            )));
        }

        // Check if it's a directory
        if !path.is_dir() {
            return Ok(ToolResult::error(format!(
                "Path is not a directory: {}",
                path.display()
            )));
        }

        let mut output = String::new();

        if input.recursive {
            Self::list_recursive(&path, &input, &mut output, 0).await?;
        } else {
            self.list_directory(&path, &input, &mut output).await?;
        }

        Ok(ToolResult::success(output))
    }
}

impl LsTool {
    async fn list_directory(
        &self,
        path: &Path,
        input: &LsInput,
        output: &mut String,
    ) -> Result<()> {
        let mut read_dir = fs::read_dir(path).await.map_err(ToolError::Io)?;

        let mut entries = Vec::new();
        while let Some(entry) = read_dir.next_entry().await.map_err(ToolError::Io)? {
            entries.push(entry);
        }

        // Sort entries
        entries.sort_by_key(|entry| entry.file_name().into_string().unwrap_or_default());

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries {
            let file_name = entry.file_name().into_string().unwrap_or_default();

            // Skip hidden files if not requested
            if !input.show_hidden && file_name.starts_with('.') {
                continue;
            }

            let metadata = entry.metadata().await.map_err(ToolError::Io)?;
            let is_dir = metadata.is_dir();

            let entry_info = if input.detailed {
                let size = metadata.len();
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .map(|d| d.as_secs())
                    })
                    .unwrap_or(0);

                let modified_time = chrono::DateTime::from_timestamp(modified as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                if is_dir {
                    format!("{:>10}  {}  {}/", "<DIR>", modified_time, file_name)
                } else {
                    format!("{:>10}  {}  {}", size, modified_time, file_name)
                }
            } else if is_dir {
                format!("{}/", file_name)
            } else {
                file_name.clone()
            };

            if is_dir {
                dirs.push(entry_info);
            } else {
                files.push(entry_info);
            }
        }

        // Output directories first, then files
        for dir in dirs {
            output.push_str(&dir);
            output.push('\n');
        }
        for file in files {
            output.push_str(&file);
            output.push('\n');
        }

        Ok(())
    }

    fn list_recursive<'a>(
        path: &'a PathBuf,
        input: &'a LsInput,
        output: &'a mut String,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let indent = "  ".repeat(depth);

            if depth > 0 {
                output.push_str(&format!("{}{}:\n", indent, path.display()));
            }

            let mut read_dir = fs::read_dir(path).await.map_err(ToolError::Io)?;

            let mut entries = Vec::new();
            while let Some(entry) = read_dir.next_entry().await.map_err(ToolError::Io)? {
                entries.push(entry);
            }

            entries.sort_by_key(|entry| entry.file_name().into_string().unwrap_or_default());

            for entry in entries {
                let file_name = entry.file_name().into_string().unwrap_or_default();

                if !input.show_hidden && file_name.starts_with('.') {
                    continue;
                }

                let metadata = entry.metadata().await.map_err(ToolError::Io)?;
                let is_dir = metadata.is_dir();

                if is_dir {
                    output.push_str(&format!("{}{}/\n", indent, file_name));
                    let subdir = entry.path();
                    Self::list_recursive(&subdir, input, output, depth + 1).await?;
                } else {
                    output.push_str(&format!("{}{}\n", indent, file_name));
                }
            }

            Ok(())
        })
    }
}
