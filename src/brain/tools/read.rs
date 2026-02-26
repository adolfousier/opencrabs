//! Read File Tool
//!
//! Allows reading file contents from the filesystem.

use super::error::{Result, ToolError, validate_file_path};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Maximum file size to read without warning (10MB)
const LARGE_FILE_THRESHOLD: u64 = 10 * 1024 * 1024;

/// Maximum file size to read at all (100MB)
const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Maximum number of lines to read in a single request
const MAX_LINES: usize = 100_000;

/// Read file tool
pub struct ReadTool;

#[derive(Debug, Deserialize, Serialize)]
struct ReadInput {
    /// Path to the file to read
    path: String,

    /// Optional: Start line (0-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    start_line: Option<usize>,

    /// Optional: Number of lines to read
    #[serde(skip_serializing_if = "Option::is_none")]
    line_count: Option<usize>,
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read contents of a file from the filesystem. Can optionally read specific line ranges."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (absolute or relative to working directory)"
                },
                "start_line": {
                    "type": "integer",
                    "description": "Optional: Starting line number (0-indexed)",
                    "minimum": 0
                },
                "line_count": {
                    "type": "integer",
                    "description": "Optional: Number of lines to read from start_line",
                    "minimum": 1
                }
            },
            "required": ["path"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Reading files is generally safe
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: ReadInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: ReadInput = serde_json::from_value(input)?;

        // Validate path: safety check, existence, and file type
        let path = match validate_file_path(&input.path, &context.working_directory) {
            Ok(p) => p,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };

        // Check file size to prevent memory exhaustion
        let metadata = fs::metadata(&path).await.map_err(ToolError::Io)?;
        let file_size = metadata.len();

        if file_size > MAX_FILE_SIZE {
            return Ok(ToolResult::error(format!(
                "File too large: {} MB exceeds maximum {} MB. Use start_line and line_count to read portions.",
                file_size / (1024 * 1024),
                MAX_FILE_SIZE / (1024 * 1024)
            )));
        }

        let is_large_file = file_size > LARGE_FILE_THRESHOLD;

        // For large files or line-range requests, use buffered streaming
        let (output, total_lines, warning) =
            if input.start_line.is_some() || input.line_count.is_some() || is_large_file {
                self.read_with_buffer(&path, input.start_line, input.line_count, is_large_file)
                    .await?
            } else {
                // Small file: read entire contents directly
                let contents = fs::read_to_string(&path).await.map_err(ToolError::Io)?;
                let line_count = contents.lines().count();
                (contents, line_count, None)
            };

        let output_len = output.len();
        let mut result = ToolResult::success(output)
            .with_metadata("path".to_string(), path.display().to_string())
            .with_metadata("bytes".to_string(), output_len.to_string())
            .with_metadata("total_lines".to_string(), total_lines.to_string());

        // Add warning for large files
        if let Some(warn_msg) = warning {
            result = result.with_metadata("warning".to_string(), warn_msg);
        }

        Ok(result)
    }
}

impl ReadTool {
    /// Read file using buffered I/O for memory efficiency
    async fn read_with_buffer(
        &self,
        path: &std::path::Path,
        start_line: Option<usize>,
        line_count: Option<usize>,
        is_large_file: bool,
    ) -> Result<(String, usize, Option<String>)> {
        let file = fs::File::open(path).await.map_err(ToolError::Io)?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let start = start_line.unwrap_or(0);
        let max_lines = line_count.unwrap_or(MAX_LINES).min(MAX_LINES);

        let mut output = String::new();
        let mut current_line = 0;
        let mut lines_read = 0;
        let mut total_lines = 0;
        let mut truncated = false;

        // Skip lines before start
        while current_line < start {
            match lines.next_line().await.map_err(ToolError::Io)? {
                Some(_) => {
                    current_line += 1;
                    total_lines += 1;
                }
                None => {
                    return Err(ToolError::InvalidInput(format!(
                        "Start line {} exceeds file length {}",
                        start, current_line
                    )));
                }
            }
        }

        // Read requested lines
        while lines_read < max_lines {
            match lines.next_line().await.map_err(ToolError::Io)? {
                Some(line) => {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                    lines_read += 1;
                    total_lines += 1;
                }
                None => break,
            }
        }

        // Count remaining lines if we haven't read the whole file
        if line_count.is_none() && lines_read >= MAX_LINES {
            truncated = true;
            // Count remaining lines without loading them into memory
            while lines.next_line().await.map_err(ToolError::Io)?.is_some() {
                total_lines += 1;
            }
        } else {
            // Count any remaining lines
            while lines.next_line().await.map_err(ToolError::Io)?.is_some() {
                total_lines += 1;
            }
        }

        let warning = if truncated {
            Some(format!(
                "Output truncated at {} lines. File has {} total lines. Use start_line and line_count for pagination.",
                MAX_LINES, total_lines
            ))
        } else if is_large_file && line_count.is_none() {
            Some(format!(
                "Large file ({} lines). Consider using start_line and line_count for better performance.",
                total_lines
            ))
        } else {
            None
        };

        Ok((output, total_lines, warning))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_read_file() {
        let temp_dir = TempDir::new().unwrap();
        let temp_file_path = temp_dir.path().join("test.txt");
        let mut temp_file = std::fs::File::create(&temp_file_path).unwrap();
        writeln!(temp_file, "Line 1\nLine 2\nLine 3").unwrap();
        temp_file.flush().unwrap();

        let tool = ReadTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id)
            .with_working_directory(temp_dir.path().to_path_buf());

        let input = serde_json::json!({
            "path": temp_file_path.to_str().unwrap()
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Line 1"));
        assert!(result.output.contains("Line 3"));
    }

    #[tokio::test]
    async fn test_read_file_line_range() {
        let temp_dir = TempDir::new().unwrap();
        let temp_file_path = temp_dir.path().join("test.txt");
        let mut temp_file = std::fs::File::create(&temp_file_path).unwrap();
        writeln!(temp_file, "Line 1\nLine 2\nLine 3\nLine 4\nLine 5").unwrap();
        temp_file.flush().unwrap();

        let tool = ReadTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id)
            .with_working_directory(temp_dir.path().to_path_buf());

        let input = serde_json::json!({
            "path": temp_file_path.to_str().unwrap(),
            "start_line": 1,
            "line_count": 2
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Line 2"));
        assert!(result.output.contains("Line 3"));
        assert!(!result.output.contains("Line 1"));
        assert!(!result.output.contains("Line 4"));
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let tool = ReadTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id)
            .with_working_directory(temp_dir.path().to_path_buf());

        let input = serde_json::json!({
            "path": "nonexistent_file.txt"
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("not found"));
    }

    #[test]
    fn test_read_tool_schema() {
        let tool = ReadTool;
        assert_eq!(tool.name(), "read_file");
        assert!(!tool.requires_approval());

        let schema = tool.input_schema();
        assert!(schema.is_object());
    }
}
