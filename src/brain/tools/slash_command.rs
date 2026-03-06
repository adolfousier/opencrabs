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
        "Execute any OpenCrabs slash command. Built-in: /help, /models (view/switch), \
         /usage (session stats), /doctor (health check), /sessions (list), \
         /approve (get/set policy), /cd (change dir), /compact, /rebuild. \
         Also executes user-defined commands from commands.toml. \
         /models with args='model-name' switches the active model."
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
            "/evolve" => Ok(ToolResult::success(
                "Use the `evolve` tool to check for and install the latest release. \
                 It downloads the pre-built binary from GitHub and hot-restarts."
                    .into(),
            )),
            "/approve" => self.handle_approve(args),
            "/help" => self.handle_help(),
            "/models" => self.handle_models(args),
            "/usage" => self.handle_usage(context).await,
            "/doctor" => self.handle_doctor().await,
            "/sessions" => self.handle_sessions(context).await,
            "/settings" => Ok(ToolResult::success(
                "Settings is a TUI screen (press S). Use config_manager read_config \
                 to view settings programmatically."
                    .into(),
            )),
            "/stop" => Ok(ToolResult::success(
                "Use the cancel mechanism to stop the current operation. \
                 On channels, users type /stop. On TUI, press Escape twice."
                    .into(),
            )),
            "/onboard" => Ok(ToolResult::success(
                "Onboarding wizard is a TUI-only interactive screen. \
                 However, you can read and modify all settings via config_manager \
                 (read_config, write_config) and manage API keys directly."
                    .into(),
            )),
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

    fn handle_help(&self) -> Result<ToolResult> {
        Ok(ToolResult::success(
            "Available commands:\n\
             /help     — Show this list\n\
             /models   — Show current provider/model + available models (args: model name to switch)\n\
             /usage    — Session token & cost stats\n\
             /stop     — Abort current operation (channels: /stop, TUI: Esc×2)\n\
             /doctor   — Run connection health check on all providers/channels\n\
             /sessions — List all sessions with stats\n\
             /approve  — Get or set approval policy (args: approve-only|auto-session|auto-always)\n\
             /cd       — Change working directory (args: path)\n\
             /compact  — Compact context (summarize + trim)\n\
             /rebuild  — Build from source & hot-restart\n\
             /evolve   — Download latest release & hot-restart\n\
             /whisper  — Voice-to-text (TUI only)\n\
             /onboard  — Setup wizard (TUI only, use config_manager for programmatic changes)\n\n\
             You can also use config_manager to read/write any config setting directly."
                .into(),
        ))
    }

    fn handle_models(&self, args: &str) -> Result<ToolResult> {
        let config = match crate::config::Config::load() {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to load config: {}", e))),
        };

        let model_arg = args.trim();

        // If a model name was provided, switch to it
        if !model_arg.is_empty() {
            // Detect active provider section
            let provider = &config.providers;
            let section = if provider.anthropic.as_ref().is_some_and(|p| p.enabled) {
                "providers.anthropic"
            } else if provider.openai.as_ref().is_some_and(|p| p.enabled) {
                "providers.openai"
            } else if provider.gemini.as_ref().is_some_and(|p| p.enabled) {
                "providers.gemini"
            } else if provider.openrouter.as_ref().is_some_and(|p| p.enabled) {
                "providers.openrouter"
            } else if provider.minimax.as_ref().is_some_and(|p| p.enabled) {
                "providers.minimax"
            } else {
                // Check custom providers
                let custom = provider
                    .custom
                    .as_ref()
                    .and_then(|m| m.iter().find(|(_, p)| p.enabled))
                    .map(|(name, _)| name.clone());
                if let Some(ref name) = custom {
                    return match crate::config::Config::write_key(
                        &format!("providers.custom.{}", name),
                        "default_model",
                        model_arg,
                    ) {
                        Ok(()) => Ok(ToolResult::success(format!(
                            "Model switched to '{}' on custom provider '{}'.",
                            model_arg, name
                        ))),
                        Err(e) => Ok(ToolResult::error(format!("Failed to write config: {}", e))),
                    };
                }
                return Ok(ToolResult::error(
                    "No active provider found. Configure one via config_manager or /onboard."
                        .into(),
                ));
            };

            return match crate::config::Config::write_key(section, "default_model", model_arg) {
                Ok(()) => Ok(ToolResult::success(format!(
                    "Model switched to '{}'. Config updated at [{section}].default_model. \
                     The change takes effect on the next request.",
                    model_arg
                ))),
                Err(e) => Ok(ToolResult::error(format!("Failed to write config: {}", e))),
            };
        }

        // No args — return current provider/model info
        let mut lines = Vec::new();

        let providers_info = [
            ("anthropic", &config.providers.anthropic),
            ("openai", &config.providers.openai),
            ("gemini", &config.providers.gemini),
            ("openrouter", &config.providers.openrouter),
            ("minimax", &config.providers.minimax),
        ];

        for (name, provider_opt) in &providers_info {
            if let Some(provider) = provider_opt {
                let status = if provider.enabled {
                    "active"
                } else {
                    "disabled"
                };
                let model = provider.default_model.as_deref().unwrap_or("(not set)");
                let models_list = if provider.models.is_empty() {
                    String::new()
                } else {
                    format!("\n    Available: {}", provider.models.join(", "))
                };
                lines.push(format!(
                    "  {} [{}]: model={}{}",
                    name, status, model, models_list
                ));
            }
        }

        // Custom providers
        if let Some(ref custom) = config.providers.custom {
            for (name, provider) in custom {
                let status = if provider.enabled {
                    "active"
                } else {
                    "disabled"
                };
                let model = provider.default_model.as_deref().unwrap_or("(not set)");
                let models_list = if provider.models.is_empty() {
                    String::new()
                } else {
                    format!("\n    Available: {}", provider.models.join(", "))
                };
                lines.push(format!(
                    "  custom/{} [{}]: model={}{}",
                    name, status, model, models_list
                ));
            }
        }

        if lines.is_empty() {
            lines.push("  No providers configured.".to_string());
        }

        Ok(ToolResult::success(format!(
            "Providers:\n{}\n\n\
             To switch model: use slash_command /models with args='<model-name>'\n\
             To change provider: use config_manager write_config on the provider section.",
            lines.join("\n")
        )))
    }

    async fn handle_usage(&self, context: &ToolExecutionContext) -> Result<ToolResult> {
        let svc_ctx = match &context.service_context {
            Some(ctx) => ctx.clone(),
            None => {
                return Ok(ToolResult::error(
                    "Service context not available — cannot query session data.".into(),
                ));
            }
        };

        let session_svc = crate::services::SessionService::new(svc_ctx);
        let session_id = context.session_id;

        let mut lines = vec!["Usage Stats".to_string(), String::new()];

        // Current session
        match session_svc.get_session(session_id).await {
            Ok(Some(session)) => {
                let name = session.title.as_deref().unwrap_or("Current Session");
                let model = session.model.as_deref().unwrap_or("(unknown)");
                lines.push(format!("Current Session: {}", name));
                lines.push(format!("  Model: {}", model));
                lines.push(format!("  Tokens: {}", session.token_count));
                lines.push(format!("  Cost: ${:.4}", session.total_cost));
            }
            _ => {
                lines.push("Current Session: (not found)".to_string());
            }
        }

        // All-time stats
        lines.push(String::new());
        {
            use crate::db::repository::UsageLedgerRepository;
            let ledger = UsageLedgerRepository::new(session_svc.pool());
            let ledger_stats = ledger.stats_by_model().await.unwrap_or_default();

            let all_tokens: i64 = ledger_stats.iter().map(|s| s.total_tokens).sum();
            let all_cost: f64 = ledger_stats.iter().map(|s| s.total_cost).sum();

            let total_sessions = session_svc
                .list_sessions(crate::db::repository::SessionListOptions::default())
                .await
                .map(|s| s.len())
                .unwrap_or(0);

            lines.push(format!(
                "All-Time: {} sessions, {} tokens, ${:.4}",
                total_sessions, all_tokens, all_cost
            ));

            for stats in ledger_stats.iter().take(10) {
                lines.push(format!(
                    "  {} — {} tokens, ${:.4}",
                    stats.model, stats.total_tokens, stats.total_cost
                ));
            }
        }

        Ok(ToolResult::success(lines.join("\n")))
    }

    async fn handle_doctor(&self) -> Result<ToolResult> {
        let config = match crate::config::Config::load() {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to load config: {}", e))),
        };

        let mut lines = vec!["Health Check".to_string(), String::new()];

        // Check keys.toml validity
        let keys_path = crate::config::keys_path();
        if keys_path.exists() {
            match std::fs::read_to_string(&keys_path) {
                Ok(content) => match toml::from_str::<toml::Value>(&content) {
                    Ok(_) => lines.push("keys.toml — OK".to_string()),
                    Err(e) => lines.push(format!("keys.toml — PARSE ERROR: {e}")),
                },
                Err(e) => lines.push(format!("keys.toml — READ ERROR: {e}")),
            }
        } else {
            lines.push("keys.toml — NOT FOUND".to_string());
        }
        lines.push(String::new());

        // Check providers
        let providers = [
            ("anthropic", &config.providers.anthropic),
            ("openai", &config.providers.openai),
            ("gemini", &config.providers.gemini),
            ("openrouter", &config.providers.openrouter),
            ("minimax", &config.providers.minimax),
        ];

        lines.push("Providers:".to_string());
        for (name, provider_opt) in &providers {
            if let Some(provider) = provider_opt
                && provider.enabled
            {
                let has_key = provider.api_key.as_ref().is_some_and(|k| !k.is_empty());
                let model = provider.default_model.as_deref().unwrap_or("(not set)");
                let status = if has_key { "OK" } else { "MISSING API KEY" };
                lines.push(format!("  {} — {} (model: {})", name, status, model));
            }
        }

        if let Some(ref custom) = config.providers.custom {
            for (name, provider) in custom {
                if provider.enabled {
                    let has_key = provider.api_key.as_ref().is_some_and(|k| !k.is_empty());
                    let model = provider.default_model.as_deref().unwrap_or("(not set)");
                    let status = if has_key { "OK" } else { "MISSING API KEY" };
                    lines.push(format!("  custom/{} — {} (model: {})", name, status, model));
                }
            }
        }

        // Check channels
        lines.push(String::new());
        lines.push("Channels:".to_string());
        let ch = &config.channels;
        if ch.telegram.enabled {
            lines.push("  telegram — enabled".to_string());
        }
        if ch.discord.enabled {
            lines.push("  discord — enabled".to_string());
        }
        if ch.slack.enabled {
            lines.push("  slack — enabled".to_string());
        }
        if ch.whatsapp.enabled {
            lines.push("  whatsapp — enabled".to_string());
        }
        if ch.trello.enabled {
            lines.push("  trello — enabled".to_string());
        }

        // Voice config
        lines.push(String::new());
        lines.push(format!(
            "Voice: STT={}, TTS={}",
            config.voice.stt_enabled, config.voice.tts_enabled
        ));

        // Approval policy
        lines.push(format!("Approval: {}", config.agent.approval_policy));

        Ok(ToolResult::success(lines.join("\n")))
    }

    async fn handle_sessions(&self, context: &ToolExecutionContext) -> Result<ToolResult> {
        let svc_ctx = match &context.service_context {
            Some(ctx) => ctx.clone(),
            None => {
                return Ok(ToolResult::error(
                    "Service context not available — cannot query sessions.".into(),
                ));
            }
        };

        let session_svc = crate::services::SessionService::new(svc_ctx);
        match session_svc
            .list_sessions(crate::db::repository::SessionListOptions::default())
            .await
        {
            Ok(sessions) => {
                if sessions.is_empty() {
                    return Ok(ToolResult::success("No sessions found.".into()));
                }

                let current_id = context.session_id;
                let mut lines = vec![format!("{} session(s):\n", sessions.len())];
                for s in sessions.iter().take(20) {
                    let title = s.title.as_deref().unwrap_or("(untitled)");
                    let model = s.model.as_deref().unwrap_or("?");
                    let marker = if s.id == current_id {
                        " ← current"
                    } else {
                        ""
                    };
                    lines.push(format!(
                        "  {} [{}] — {} tokens, ${:.4}{}",
                        title, model, s.token_count, s.total_cost, marker
                    ));
                }
                if sessions.len() > 20 {
                    lines.push(format!("  ... and {} more", sessions.len() - 20));
                }
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to list sessions: {}", e))),
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
                "/evolve",
                "/approve",
                "/models",
                "/sessions",
                "/help",
                "/usage",
                "/doctor",
                "/stop",
                "/settings",
                "/onboard",
                "/whisper",
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
    async fn test_models_returns_provider_info() {
        let tool = SlashCommandTool;
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(serde_json::json!({"command": "/models"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("Providers"));
    }

    #[tokio::test]
    async fn test_help_returns_commands() {
        let tool = SlashCommandTool;
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(serde_json::json!({"command": "/help"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("/models"));
        assert!(result.output.contains("/usage"));
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
