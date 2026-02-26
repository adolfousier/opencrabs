//! Grep Content Search Tool
//!
//! Search file contents for matching patterns.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Directories that are almost never useful to grep through.
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    "__pycache__",
    ".mypy_cache",
    ".tox",
    ".eggs",
    "vendor",
    ".bundle",
];

/// Grep search tool
pub struct GrepTool;

#[derive(Debug, Deserialize, Serialize)]
struct GrepInput {
    /// Pattern to search for
    pattern: String,

    /// Path to search (file or directory)
    #[serde(default)]
    path: Option<String>,

    /// Use regex instead of literal string
    #[serde(default)]
    regex: bool,

    /// Case insensitive search
    #[serde(default)]
    case_insensitive: bool,

    /// Show line numbers
    #[serde(default = "default_true")]
    line_numbers: bool,

    /// Context lines to show before and after match
    #[serde(default)]
    context: Option<usize>,

    /// File pattern to filter (e.g., "*.rs")
    #[serde(default)]
    file_pattern: Option<String>,

    /// Maximum number of matches to return
    #[serde(default)]
    limit: Option<usize>,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search for patterns in file contents. Supports literal string or regex search with context lines."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Pattern to search for (literal string or regex)"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search (defaults to working directory)"
                },
                "regex": {
                    "type": "boolean",
                    "description": "Treat pattern as regex instead of literal string",
                    "default": false
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case insensitive search",
                    "default": false
                },
                "line_numbers": {
                    "type": "boolean",
                    "description": "Show line numbers in results",
                    "default": true
                },
                "context": {
                    "type": "integer",
                    "description": "Number of context lines to show before and after match",
                    "minimum": 0
                },
                "file_pattern": {
                    "type": "string",
                    "description": "File pattern to filter (e.g., '*.rs', '*.{js,ts}')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of matches to return",
                    "minimum": 1
                }
            },
            "required": ["pattern"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Searching is safe
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input: GrepInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;

        if input.pattern.trim().is_empty() {
            return Err(ToolError::InvalidInput(
                "Pattern cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let mut input: GrepInput = serde_json::from_value(input)?;

        // Default limit to prevent runaway searches
        if input.limit.is_none() {
            input.limit = Some(200);
        }

        // Build regex pattern
        let pattern_str = if input.regex {
            input.pattern.clone()
        } else {
            regex::escape(&input.pattern)
        };

        let regex = if input.case_insensitive {
            regex::RegexBuilder::new(&pattern_str)
                .case_insensitive(true)
                .build()
        } else {
            regex::Regex::new(&pattern_str)
        }
        .map_err(|e| ToolError::InvalidInput(format!("Invalid pattern: {}", e)))?;

        // Resolve search path
        let search_path = if let Some(ref p) = input.path {
            if PathBuf::from(p).is_absolute() {
                PathBuf::from(p)
            } else {
                context.working_directory.join(p)
            }
        } else {
            context.working_directory.clone()
        };

        if !search_path.exists() {
            return Ok(ToolResult::error(format!(
                "Path does not exist: {}",
                search_path.display()
            )));
        }

        let mut matches = Vec::new();
        let mut total_matches = 0;

        if search_path.is_file() {
            self.search_file(
                &search_path,
                &regex,
                &input,
                &mut matches,
                &mut total_matches,
            )
            .await?;
        } else {
            self.search_directory(
                &search_path,
                &regex,
                &input,
                &mut matches,
                &mut total_matches,
            )
            .await?;
        }

        if matches.is_empty() {
            return Ok(ToolResult::success(format!(
                "No matches found for pattern: '{}'",
                input.pattern
            )));
        }

        let output = matches.join("\n\n");
        let summary = if let Some(_limit) = input.limit {
            if total_matches > matches.len() {
                format!(
                    "\n\n({} matches shown, {} total)",
                    matches.len(),
                    total_matches
                )
            } else {
                format!("\n\n({} matches)", total_matches)
            }
        } else {
            format!("\n\n({} matches)", total_matches)
        };

        Ok(ToolResult::success(format!("{}{}", output, summary)))
    }
}

impl GrepTool {
    async fn search_file(
        &self,
        path: &Path,
        regex: &regex::Regex,
        input: &GrepInput,
        matches: &mut Vec<String>,
        total_matches: &mut usize,
    ) -> Result<()> {
        // Check file pattern filter
        if let Some(ref pattern) = input.file_pattern {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let glob_pattern = glob::Pattern::new(pattern)
                .map_err(|e| ToolError::InvalidInput(format!("Invalid file pattern: {}", e)))?;

            if !glob_pattern.matches(file_name) {
                return Ok(());
            }
        }

        let content = match fs::read_to_string(path).await {
            Ok(c) => c,
            Err(_) => return Ok(()), // Skip binary files or unreadable files
        };

        let lines: Vec<&str> = content.lines().collect();
        let display_path = path.display().to_string();

        for (line_num, line) in lines.iter().enumerate() {
            if regex.is_match(line) {
                *total_matches += 1;

                // Check limit
                if let Some(limit) = input.limit
                    && matches.len() >= limit
                {
                    return Ok(());
                }

                let mut result = String::new();
                result.push_str(&format!("{}:", display_path));

                if input.line_numbers {
                    result.push_str(&format!("{}:", line_num + 1));
                }

                // Add context before
                if let Some(ctx) = input.context {
                    let start = line_num.saturating_sub(ctx);
                    for (i, line) in lines.iter().enumerate().skip(start).take(line_num - start) {
                        result.push_str(&format!("\n  {}: {}", i + 1, line));
                    }
                }

                // Add matching line
                result.push_str(&format!("\n> {}", line));

                // Add context after
                if let Some(ctx) = input.context {
                    let end = (line_num + ctx + 1).min(lines.len());
                    for (i, line) in lines
                        .iter()
                        .enumerate()
                        .skip(line_num + 1)
                        .take(end - line_num - 1)
                    {
                        result.push_str(&format!("\n  {}: {}", i + 1, line));
                    }
                }

                matches.push(result);
            }
        }

        Ok(())
    }

    fn search_directory<'a>(
        &'a self,
        dir: &'a PathBuf,
        regex: &'a regex::Regex,
        input: &'a GrepInput,
        matches: &'a mut Vec<String>,
        total_matches: &'a mut usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = fs::read_dir(dir).await.map_err(ToolError::Io)?;

            while let Some(entry) = entries.next_entry().await.map_err(ToolError::Io)? {
                let path = entry.path();

                // Check limit
                if let Some(limit) = input.limit
                    && matches.len() >= limit
                {
                    return Ok(());
                }

                if path.is_file() {
                    self.search_file(&path, regex, input, matches, total_matches)
                        .await?;
                } else if path.is_dir() {
                    // Skip hidden and heavy directories
                    if let Some(name) = path.file_name().and_then(|n| n.to_str())
                        && (name.starts_with('.') || SKIP_DIRS.contains(&name))
                    {
                        continue;
                    }
                    self.search_directory(&path, regex, input, matches, total_matches)
                        .await?;
                }
            }

            Ok(())
        })
    }
}
