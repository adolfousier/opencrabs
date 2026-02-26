//! Bash/Shell Command Execution Tool
//!
//! Allows executing shell commands in the system.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

/// Bash execution tool
pub struct BashTool;

#[derive(Debug, Deserialize, Serialize)]
struct BashInput {
    /// Command to execute
    command: String,

    /// Optional working directory (overrides context)
    #[serde(skip_serializing_if = "Option::is_none")]
    working_dir: Option<String>,

    /// Optional timeout in seconds (overrides context default)
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_secs: Option<u64>,
}

/// Check if a bash command is safe for read-only mode (Plan mode)
fn is_read_only_command(command: &str) -> bool {
    let cmd_lower = command.trim().to_lowercase();

    // Check for output redirection (dangerous in read-only mode)
    if cmd_lower.contains('>') || cmd_lower.contains(">>") {
        return false;
    }

    // Check for dangerous pipe patterns (piping to tee, writing to files)
    if cmd_lower.contains("| tee") || cmd_lower.contains("|tee") {
        return false;
    }

    // Check for command substitution (can hide dangerous commands)
    if cmd_lower.contains("$(") || cmd_lower.contains("`") {
        return false;
    }

    // Check for subshell execution
    if cmd_lower.contains("bash ") || cmd_lower.contains("sh ") || cmd_lower.contains("eval ") {
        return false;
    }

    // Get the first command (before pipes or &&)
    let first_cmd = cmd_lower
        .split('|')
        .next()
        .unwrap_or(&cmd_lower)
        .split("&&")
        .next()
        .unwrap_or(&cmd_lower)
        .split(';')
        .next()
        .unwrap_or(&cmd_lower)
        .trim();

    // Get the command name (first word) - this is what we'll validate
    let cmd_name = first_cmd.split_whitespace().next().unwrap_or("");

    // List of safe read-only single commands (exact command name match)
    let safe_single_commands = [
        "ls", "cat", "head", "tail", "less", "more", "grep", "find", "tree", "file", "pwd",
        "whoami", "hostname", "date", "echo", "which", "type", "env", "printenv", "df", "du", "wc",
        "curl", "wget", "rg", "fd", "bat", "exa", "eza",
    ];

    // List of safe git subcommands (read-only)
    let safe_git_subcommands = [
        "status",
        "log",
        "diff",
        "branch",
        "show",
        "remote",
        "tag",
        "describe",
        "rev-parse",
        "config",
        "ls-files",
        "ls-tree",
        "shortlog",
        "blame",
        "reflog",
    ];

    // List of safe cargo subcommands (read-only)
    let safe_cargo_subcommands = [
        "version",
        "check",
        "clippy",
        "fmt",
        "test",
        "build",
        "doc",
        "tree",
        "metadata",
        "verify-project",
    ];

    // Check if command is in safe single commands list (exact match)
    if safe_single_commands.contains(&cmd_name) {
        return true;
    }

    // Check for git commands with safe subcommands
    if cmd_name == "git" {
        let parts: Vec<&str> = first_cmd.split_whitespace().collect();
        if parts.len() >= 2 {
            let subcommand = parts[1];
            // Check if the git subcommand is in our safe list
            return safe_git_subcommands.contains(&subcommand);
        }
        // Bare "git" command is safe (just shows help)
        return true;
    }

    // Check for cargo commands with safe subcommands
    if cmd_name == "cargo" {
        let parts: Vec<&str> = first_cmd.split_whitespace().collect();
        if parts.len() >= 2 {
            let subcommand = parts[1];
            // Check if the cargo subcommand is in our safe list
            return safe_cargo_subcommands.contains(&subcommand);
        }
        // Bare "cargo" command is safe (just shows help)
        return true;
    }

    false
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command. Returns stdout, stderr, and exit code. Use carefully as this can modify system state."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Optional: Working directory for command execution"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional: Timeout in seconds (default 120, max 600). Use higher values for builds."
                }
            },
            "required": ["command"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![
            ToolCapability::ExecuteShell,
            ToolCapability::SystemModification,
            ToolCapability::Network,
        ]
    }

    fn requires_approval(&self) -> bool {
        true // Shell execution always requires approval
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input: BashInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;

        if input.command.trim().is_empty() {
            return Err(ToolError::InvalidInput(
                "Command cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: BashInput = serde_json::from_value(input)?;

        // Check if in read-only mode and validate command safety
        if context.read_only_mode && !is_read_only_command(&input.command) {
            return Ok(ToolResult::error(format!(
                "Command '{}' is not allowed in Plan mode (read-only). \
                 Only safe read-only commands are permitted (git status, ls, cat, grep, etc.). \
                 Please approve the plan and switch to execution mode (Ctrl+A) to run write commands.",
                input.command
            )));
        }

        // Determine working directory
        let working_dir = if let Some(ref dir) = input.working_dir {
            std::path::PathBuf::from(dir)
        } else {
            context.working_directory.clone()
        };

        // Verify working directory exists
        if !working_dir.exists() {
            return Ok(ToolResult::error(format!(
                "Working directory does not exist: {}",
                working_dir.display()
            )));
        }

        // Prepare command for the current platform
        let (shell, shell_arg) = if cfg!(target_os = "windows") {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        // Determine timeout: use input override if provided, else context default, cap at 600s
        let effective_timeout = input.timeout_secs.unwrap_or(context.timeout_secs).min(600);

        // Detect sudo commands and request password via callback
        let is_sudo = input.command.trim_start().starts_with("sudo ");
        let sudo_password = if is_sudo {
            if let Some(ref callback) = context.sudo_callback {
                match callback(input.command.clone()).await {
                    Ok(Some(password)) => Some(password),
                    Ok(None) => return Ok(ToolResult::error("Sudo cancelled by user".to_string())),
                    Err(e) => return Ok(ToolResult::error(format!("Sudo prompt failed: {}", e))),
                }
            } else {
                None // No callback — run normally (will fail if password needed)
            }
        } else {
            None
        };

        // Execute command with timeout — use piped stdin for sudo password
        let output = if let Some(password) = sudo_password {
            // Rewrite command to read password from stdin via -S flag
            let sudo_cmd = if input.command.trim_start().starts_with("sudo -S ") {
                input.command.clone()
            } else {
                input.command.replacen("sudo ", "sudo -S ", 1)
            };

            let command_future = async {
                let mut child = Command::new(shell)
                    .arg(shell_arg)
                    .arg(&sudo_cmd)
                    .current_dir(&working_dir)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()?;

                // Write password to stdin and close it
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(format!("{}\n", password).as_bytes()).await;
                    drop(stdin);
                }

                child.wait_with_output().await
            };

            match timeout(Duration::from_secs(effective_timeout), command_future).await {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    return Ok(ToolResult::error(format!(
                        "Command execution failed: {}",
                        e
                    )));
                }
                Err(_) => {
                    return Err(ToolError::Timeout(effective_timeout));
                }
            }
        } else {
            // Normal execution (no sudo password needed)
            let command_future = Command::new(shell)
                .arg(shell_arg)
                .arg(&input.command)
                .current_dir(&working_dir)
                .output();

            match timeout(Duration::from_secs(effective_timeout), command_future).await {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    return Ok(ToolResult::error(format!(
                        "Command execution failed: {}",
                        e
                    )));
                }
                Err(_) => {
                    return Err(ToolError::Timeout(effective_timeout));
                }
            }
        };

        // Convert output to strings
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        // Build output message
        let mut result_text = String::new();

        if !stdout.is_empty() {
            result_text.push_str("STDOUT:\n");
            result_text.push_str(&stdout);
        }

        if !stderr.is_empty() {
            if !result_text.is_empty() {
                result_text.push_str("\n\n");
            }
            result_text.push_str("STDERR:\n");
            result_text.push_str(&stderr);
        }

        if result_text.is_empty() {
            result_text = "(no output)".to_string();
        }

        let success = output.status.success();

        let result = if success {
            ToolResult::success(result_text)
        } else {
            ToolResult {
                success: false,
                output: result_text,
                error: Some(format!("Command exited with code {}", exit_code)),
                metadata: std::collections::HashMap::new(),
            }
        };

        Ok(result
            .with_metadata("exit_code".to_string(), exit_code.to_string())
            .with_metadata("working_dir".to_string(), working_dir.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_bash_simple_command() {
        let tool = BashTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id).with_auto_approve(true);

        let command = if cfg!(target_os = "windows") {
            "echo Hello"
        } else {
            "echo 'Hello'"
        };

        let input = serde_json::json!({
            "command": command
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Hello"));
    }

    #[tokio::test]
    async fn test_bash_with_exit_code() {
        let tool = BashTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id).with_auto_approve(true);

        let command = "exit 1";

        let input = serde_json::json!({
            "command": command
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.metadata.get("exit_code"), Some(&"1".to_string()));
    }

    #[tokio::test]
    async fn test_bash_invalid_command() {
        let tool = BashTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id).with_auto_approve(true);

        let input = serde_json::json!({
            "command": "nonexistent_command_12345"
        });

        let result = tool.execute(input, &context).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    #[cfg(not(target_os = "windows"))] // Skip on Windows due to cmd.exe limitations
    async fn test_bash_timeout() {
        let tool = BashTool;
        let session_id = Uuid::new_v4();
        let context = ToolExecutionContext::new(session_id)
            .with_auto_approve(true)
            .with_timeout(1); // 1 second timeout

        let input = serde_json::json!({
            "command": "sleep 5"
        });

        let result = tool.execute(input, &context).await;
        assert!(result.is_err(), "Expected timeout error, got: {:?}", result);
        assert!(matches!(result.unwrap_err(), ToolError::Timeout(_)));
    }

    #[test]
    fn test_bash_tool_schema() {
        let tool = BashTool;
        assert_eq!(tool.name(), "bash");
        assert!(tool.requires_approval());

        let capabilities = tool.capabilities();
        assert!(capabilities.contains(&ToolCapability::ExecuteShell));
        assert!(capabilities.contains(&ToolCapability::SystemModification));
    }

    #[test]
    fn test_validate_empty_command() {
        let tool = BashTool;
        let input = serde_json::json!({
            "command": ""
        });

        let result = tool.validate_input(&input);
        assert!(result.is_err());
    }
}
