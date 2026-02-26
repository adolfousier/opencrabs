//! Code Execution Tool
//!
//! Execute code in various languages within a sandboxed environment.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

/// Code execution tool
pub struct CodeExecTool;

#[derive(Debug, Deserialize, Serialize)]
struct CodeExecInput {
    /// Programming language
    language: String,

    /// Code to execute
    code: String,

    /// Optional: Additional arguments to pass to interpreter
    #[serde(default)]
    args: Vec<String>,

    /// Optional: Timeout in seconds (max 60)
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
}

fn default_timeout() -> u64 {
    30
}

#[async_trait]
impl Tool for CodeExecTool {
    fn name(&self) -> &str {
        "execute_code"
    }

    fn description(&self) -> &str {
        "Execute code in a sandboxed environment. Supports Python, JavaScript (Node.js), Rust, and shell scripts. Returns stdout, stderr, and exit code."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "description": "Programming language",
                    "enum": ["python", "python3", "javascript", "js", "node", "rust", "sh", "bash"]
                },
                "code": {
                    "type": "string",
                    "description": "Code to execute"
                },
                "args": {
                    "type": "array",
                    "description": "Additional arguments to pass to the interpreter",
                    "items": {
                        "type": "string"
                    },
                    "default": []
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Execution timeout in seconds (default: 30, max: 60)",
                    "default": 30,
                    "minimum": 1,
                    "maximum": 60
                }
            },
            "required": ["language", "code"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![
            ToolCapability::ExecuteShell,
            ToolCapability::SystemModification,
            ToolCapability::WriteFiles,
        ]
    }

    fn requires_approval(&self) -> bool {
        true // Code execution requires approval
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input: CodeExecInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;

        if input.code.trim().is_empty() {
            return Err(ToolError::InvalidInput("Code cannot be empty".to_string()));
        }

        if input.timeout_secs == 0 || input.timeout_secs > 60 {
            return Err(ToolError::InvalidInput(
                "Timeout must be between 1 and 60 seconds".to_string(),
            ));
        }

        let valid_languages = [
            "python",
            "python3",
            "javascript",
            "js",
            "node",
            "rust",
            "sh",
            "bash",
        ];
        if !valid_languages.contains(&input.language.as_str()) {
            return Err(ToolError::InvalidInput(format!(
                "Unsupported language: {}. Supported: {}",
                input.language,
                valid_languages.join(", ")
            )));
        }

        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        // Check if in read-only mode (Plan mode)
        if context.read_only_mode {
            return Ok(ToolResult::error(
                "Code execution is not allowed in Plan mode. \
                 Please approve the plan and switch to execution mode (Ctrl+A) to execute code."
                    .to_string(),
            ));
        }

        let input: CodeExecInput = serde_json::from_value(input)?;

        // Determine interpreter and file extension
        let (interpreter, extension, extra_args) = match input.language.as_str() {
            "python" | "python3" => ("python3", "py", vec![]),
            "javascript" | "js" | "node" => ("node", "js", vec![]),
            "rust" => (
                "rustc",
                "rs",
                vec!["--out-dir".to_string(), "/tmp".to_string()],
            ),
            "sh" | "bash" => ("bash", "sh", vec![]),
            _ => {
                return Ok(ToolResult::error(format!(
                    "Unsupported language: {}",
                    input.language
                )));
            }
        };

        // Check if interpreter exists
        let interpreter_check = which::which(interpreter);
        if interpreter_check.is_err() {
            return Ok(ToolResult::error(format!(
                "Interpreter '{}' not found. Please install it first.",
                interpreter
            )));
        }

        // Create temporary file
        let temp_dir = std::env::temp_dir();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| ToolError::Internal(format!("Failed to get system time: {}", e)))?
            .as_nanos();
        let temp_file = temp_dir.join(format!("opencrabs_exec_{}.{}", timestamp, extension));

        // Write code to temp file
        fs::write(&temp_file, &input.code)
            .await
            .map_err(ToolError::Io)?;

        // Prepare command
        let mut cmd = Command::new(interpreter);
        cmd.current_dir(&context.working_directory);

        // Add extra args (like rustc --out-dir)
        for arg in extra_args {
            cmd.arg(arg);
        }

        // Add user-provided args
        for arg in &input.args {
            cmd.arg(arg);
        }

        // Add the temp file path
        cmd.arg(&temp_file);

        // Execute with timeout
        let exec_timeout = Duration::from_secs(input.timeout_secs);
        let output_future = cmd.output();

        let output = match timeout(exec_timeout, output_future).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                // Clean up temp file
                let _ = fs::remove_file(&temp_file).await;
                return Ok(ToolResult::error(format!("Code execution failed: {}", e)));
            }
            Err(_) => {
                // Clean up temp file
                let _ = fs::remove_file(&temp_file).await;
                return Err(ToolError::Timeout(input.timeout_secs));
            }
        };

        // Clean up temp file
        let _ = fs::remove_file(&temp_file).await;

        // For Rust, also clean up the compiled binary
        if input.language == "rust" {
            let binary_name = temp_file.with_extension("");
            let _ = fs::remove_file(&binary_name).await;
        }

        // Convert output to strings
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        // Build output message
        let mut result_text = format!("Language: {}\nExit Code: {}\n\n", input.language, exit_code);

        if !stdout.is_empty() {
            result_text.push_str("STDOUT:\n");
            result_text.push_str(&stdout);
            result_text.push('\n');
        }

        if !stderr.is_empty() {
            if !stdout.is_empty() {
                result_text.push('\n');
            }
            result_text.push_str("STDERR:\n");
            result_text.push_str(&stderr);
        }

        if stdout.is_empty() && stderr.is_empty() {
            result_text.push_str("(no output)");
        }

        let success = output.status.success();
        let mut tool_result = if success {
            ToolResult::success(result_text)
        } else {
            ToolResult::error(result_text)
        };

        tool_result
            .metadata
            .insert("exit_code".to_string(), exit_code.to_string());
        tool_result
            .metadata
            .insert("language".to_string(), input.language);

        Ok(tool_result)
    }
}
