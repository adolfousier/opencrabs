//! Slash Command Tool
//!
//! Lets the agent invoke any slash command programmatically — both built-in
//! (/cd, /compact, /rebuild) and user-defined commands from commands.toml.
//! New commands added via `config_manager add_command` are automatically available.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

pub struct SlashCommandTool;

#[async_trait]
impl Tool for SlashCommandTool {
    fn name(&self) -> &str {
        "slash_command"
    }

    fn description(&self) -> &str {
        "Execute any OpenCrabs slash command. Works for built-in commands (/cd, /compact, \
         /rebuild, /approve, etc.) and user-defined commands from commands.toml. \
         Use this when the user asks you to run a command or when you need to trigger \
         a slash command internally. New user-defined commands are available immediately."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The slash command to execute (e.g. '/cd', '/compact', '/deploy'). Must start with '/'."
                },
                "args": {
                    "type": "string",
                    "description": "Optional arguments for the command (e.g. a directory path for /cd)"
                }
            },
            "required": ["command"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::WriteFiles]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let args = input.get("args").and_then(|v| v.as_str()).unwrap_or("");

        if !command.starts_with('/') {
            return Ok(ToolResult::error(format!(
                "Command must start with '/'. Got: '{}'",
                command
            )));
        }

        match command {
            "/cd" => self.handle_cd(args, context),
            "/compact" => Ok(ToolResult::success(
                "Compaction requested. Summarize the current conversation for continuity, \
                 then the system will trim context automatically."
                    .into(),
            )),
            "/rebuild" => self.handle_rebuild(),
            "/approve" => self.handle_approve(args),
            // TUI-only commands — agent can't open UI dialogs
            "/models" | "/sessions" | "/help" | "/settings" | "/onboard" | "/usage" => {
                Ok(ToolResult::success(format!(
                    "{} is a TUI-only command (opens an interactive dialog). \
                     Tell the user to type {} in the input box.",
                    command, command
                )))
            }
            "/whisper" => Ok(ToolResult::success(
                "WhisperCrabs is a TUI-triggered command. Tell the user to type /whisper \
                 in the input box to launch the floating voice-to-text tool."
                    .into(),
            )),
            _ => self.handle_user_command(command, args),
        }
    }
}

impl SlashCommandTool {
    fn handle_cd(&self, args: &str, context: &ToolExecutionContext) -> Result<ToolResult> {
        let path_str = args.trim();
        if path_str.is_empty() {
            return Ok(ToolResult::error(
                "No directory specified. Usage: slash_command /cd with args='/path/to/dir'".into(),
            ));
        }

        let path = std::path::PathBuf::from(path_str);
        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "Path does not exist: {}",
                path_str
            )));
        }
        if !path.is_dir() {
            return Ok(ToolResult::error(format!(
                "Path is not a directory: {}",
                path_str
            )));
        }

        let canonical = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Failed to resolve path: {}", e))),
        };

        // Update runtime working directory
        if let Some(ref shared_wd) = context.shared_working_directory {
            *shared_wd.write().expect("working_directory lock poisoned") = canonical.clone();
        }

        // Persist to config.toml
        if let Err(e) = crate::config::Config::write_key(
            "agent",
            "working_directory",
            &canonical.to_string_lossy(),
        ) {
            return Ok(ToolResult::error(format!(
                "Runtime updated but failed to persist to config.toml: {}",
                e
            )));
        }

        Ok(ToolResult::success(format!(
            "Working directory changed to: {}",
            canonical.display()
        )))
    }

    fn handle_rebuild(&self) -> Result<ToolResult> {
        // Detect source and report — actual build should use the rebuild tool
        match crate::brain::SelfUpdater::auto_detect() {
            Ok(updater) => Ok(ToolResult::success(format!(
                "Source detected at: {}. Use the `rebuild` tool to build and restart, \
                 or tell the user to type /rebuild.",
                updater.project_root().display()
            ))),
            Err(e) => Ok(ToolResult::error(format!(
                "Cannot detect project source: {}",
                e
            ))),
        }
    }

    fn handle_approve(&self, args: &str) -> Result<ToolResult> {
        let policy = args.trim();
        if policy.is_empty() {
            // Read current policy
            return match crate::config::Config::load() {
                Ok(cfg) => Ok(ToolResult::success(format!(
                    "Current approval policy: {}",
                    cfg.agent.approval_policy
                ))),
                Err(e) => Ok(ToolResult::error(format!("Failed to read config: {}", e))),
            };
        }

        // Set policy
        match policy {
            "approve-only" | "auto-session" | "auto-always" => {
                match crate::config::Config::write_key("agent", "approval_policy", policy) {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Approval policy set to: {}",
                        policy
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to write config: {}", e))),
                }
            }
            _ => Ok(ToolResult::error(format!(
                "Invalid policy: '{}'. Valid: approve-only, auto-session, auto-always",
                policy
            ))),
        }
    }

    fn handle_user_command(&self, command: &str, _args: &str) -> Result<ToolResult> {
        let brain_path = crate::brain::BrainLoader::resolve_path();
        let loader = crate::brain::CommandLoader::from_brain_path(&brain_path);
        let commands = loader.load();

        if let Some(cmd) = commands.iter().find(|c| c.name == command) {
            match cmd.action.as_str() {
                "system" => Ok(ToolResult::success(format!(
                    "[System message] {}",
                    cmd.prompt
                ))),
                _ => {
                    // "prompt" action — return the prompt for the agent to execute
                    Ok(ToolResult::success(format!(
                        "User command '{}' ({}): {}",
                        cmd.name, cmd.description, cmd.prompt
                    )))
                }
            }
        } else {
            // List available commands for context
            let available: Vec<String> = commands.iter().map(|c| c.name.clone()).collect();
            let builtin = [
                "/cd",
                "/compact",
                "/rebuild",
                "/approve",
                "/models",
                "/sessions",
                "/help",
                "/onboard",
                "/usage",
                "/whisper",
                "/settings",
            ];
            Ok(ToolResult::error(format!(
                "Unknown command: '{}'. Built-in: {}. User-defined: {}",
                command,
                builtin.join(", "),
                if available.is_empty() {
                    "(none)".to_string()
                } else {
                    available.join(", ")
                }
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_metadata() {
        let tool = SlashCommandTool;
        assert_eq!(tool.name(), "slash_command");
        assert!(tool.requires_approval());
    }

    #[tokio::test]
    async fn test_missing_slash() {
        let tool = SlashCommandTool;
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(serde_json::json!({"command": "cd"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("must start with '/'"));
    }

    #[tokio::test]
    async fn test_tui_only_command() {
        let tool = SlashCommandTool;
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(serde_json::json!({"command": "/models"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("TUI-only"));
    }

    #[tokio::test]
    async fn test_cd_no_args() {
        let tool = SlashCommandTool;
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(serde_json::json!({"command": "/cd"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("No directory"));
    }

    #[tokio::test]
    async fn test_unknown_command() {
        let tool = SlashCommandTool;
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(serde_json::json!({"command": "/nonexistent"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Unknown command"));
    }
}
