//! Glob Pattern Matching Tool
//!
//! Find files matching glob patterns.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Glob pattern matching tool
pub struct GlobTool;

#[derive(Debug, Deserialize, Serialize)]
struct GlobInput {
    /// Glob pattern to match
    pattern: String,

    /// Base directory for search (defaults to working directory)
    #[serde(default)]
    base_dir: Option<String>,

    /// Maximum number of results to return
    #[serde(default)]
    limit: Option<usize>,

    /// Include hidden files
    #[serde(default)]
    include_hidden: bool,
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Supports wildcards: * (any chars), ** (recursive directories), ? (single char), [abc] (char class)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (e.g., '**/*.rs', 'src/**/*.test.js', '*.{md,txt}')"
                },
                "base_dir": {
                    "type": "string",
                    "description": "Base directory for search (defaults to working directory)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return",
                    "minimum": 1
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "Include hidden files (starting with .)",
                    "default": false
                }
            },
            "required": ["pattern"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Pattern matching is safe
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input: GlobInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;

        if input.pattern.trim().is_empty() {
            return Err(ToolError::InvalidInput(
                "Pattern cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: GlobInput = serde_json::from_value(input)?;

        // Resolve base directory
        let base_dir = if let Some(ref dir) = input.base_dir {
            if PathBuf::from(dir).is_absolute() {
                PathBuf::from(dir)
            } else {
                context.working_directory.join(dir)
            }
        } else {
            context.working_directory.clone()
        };

        if !base_dir.exists() {
            return Ok(ToolResult::error(format!(
                "Base directory does not exist: {}",
                base_dir.display()
            )));
        }

        // Build full pattern with base directory
        let full_pattern = base_dir.join(&input.pattern);
        let pattern_str = full_pattern
            .to_str()
            .ok_or_else(|| ToolError::InvalidInput("Invalid path encoding".to_string()))?;

        // Use glob crate to find matches
        let glob_result = glob::glob(pattern_str)
            .map_err(|e| ToolError::InvalidInput(format!("Invalid glob pattern: {}", e)))?;

        let mut matches: Vec<PathBuf> = Vec::new();

        for entry in glob_result {
            match entry {
                Ok(path) => {
                    // Filter hidden files if not requested
                    if !input.include_hidden
                        && let Some(file_name) = path.file_name()
                        && file_name
                            .to_str()
                            .map(|s| s.starts_with('.'))
                            .unwrap_or(false)
                    {
                        continue;
                    }

                    matches.push(path);

                    // Apply limit
                    if let Some(limit) = input.limit
                        && matches.len() >= limit
                    {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!("Error reading glob entry: {}", e);
                }
            }
        }

        if matches.is_empty() {
            return Ok(ToolResult::success(format!(
                "No files found matching pattern: {}",
                input.pattern
            )));
        }

        // Sort matches for consistent output
        matches.sort();

        // Format output
        let mut output = format!(
            "Found {} files matching '{}':\n\n",
            matches.len(),
            input.pattern
        );

        for path in &matches {
            // Make path relative to base_dir for cleaner output
            let display_path = path
                .strip_prefix(&base_dir)
                .unwrap_or(path)
                .display()
                .to_string();
            output.push_str(&format!("  {}\n", display_path));
        }

        if let Some(limit) = input.limit
            && matches.len() >= limit
        {
            output.push_str(&format!("\n(Limited to {} results)", limit));
        }

        Ok(ToolResult::success(output))
    }
}
