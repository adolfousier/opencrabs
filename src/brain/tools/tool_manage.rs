//! Tool Manage — meta-tool for runtime tool management.
//!
//! Allows the agent to list, add, remove, enable, disable, and reload
//! dynamic tools defined in `~/.opencrabs/tools.toml`.

use super::ToolRegistry;
use super::dynamic::{DynamicToolDef, DynamicToolLoader, ExecutorType, ParamDef};
use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Meta-tool the agent uses to manage dynamic tools at runtime.
pub struct ToolManageTool {
    registry: Arc<ToolRegistry>,
    tools_path: PathBuf,
}

impl ToolManageTool {
    pub fn new(registry: Arc<ToolRegistry>, tools_path: PathBuf) -> Self {
        Self {
            registry,
            tools_path,
        }
    }
}

#[async_trait]
impl Tool for ToolManageTool {
    fn name(&self) -> &str {
        "tool_manage"
    }

    fn description(&self) -> &str {
        "Manage dynamic tools at runtime. Add new HTTP or shell tools, list/remove/enable/disable \
         existing ones, or reload from disk. Dynamic tools appear in the tool list immediately \
         without restart. Use this to extend your own capabilities on the fly."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "add", "remove", "enable", "disable", "reload"],
                    "description": "Action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Tool name (required for add/remove/enable/disable)"
                },
                "description": {
                    "type": "string",
                    "description": "Tool description shown to the LLM (required for add)"
                },
                "executor": {
                    "type": "string",
                    "enum": ["http", "shell"],
                    "description": "Executor type (required for add)"
                },
                "method": {
                    "type": "string",
                    "description": "HTTP method (for http executor)"
                },
                "url": {
                    "type": "string",
                    "description": "URL with optional {{param}} placeholders (for http executor)"
                },
                "headers": {
                    "type": "object",
                    "description": "Static headers (for http executor)",
                    "additionalProperties": { "type": "string" }
                },
                "command": {
                    "type": "string",
                    "description": "Shell command with optional {{param}} placeholders (for shell executor)"
                },
                "params": {
                    "type": "array",
                    "description": "Parameter definitions",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "type": { "type": "string", "default": "string" },
                            "description": { "type": "string" },
                            "required": { "type": "boolean", "default": true },
                            "default": { "type": "string" }
                        },
                        "required": ["name"]
                    }
                },
                "requires_approval": {
                    "type": "boolean",
                    "description": "Whether tool requires approval (default: true)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds for http executor (default: 30)"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = input["action"].as_str().unwrap_or("").to_string();

        match action.as_str() {
            "list" => self.handle_list(),
            "add" => self.handle_add(&input),
            "remove" => self.handle_remove(&input),
            "enable" => self.handle_set_enabled(&input, true),
            "disable" => self.handle_set_enabled(&input, false),
            "reload" => self.handle_reload(),
            _ => Ok(ToolResult::error(format!(
                "Unknown action: '{action}'. Use: list, add, remove, enable, disable, reload"
            ))),
        }
    }
}

impl ToolManageTool {
    fn handle_list(&self) -> Result<ToolResult> {
        let defs = DynamicToolLoader::list_tools_detailed(&self.tools_path);
        if defs.is_empty() {
            return Ok(ToolResult::success(
                "No dynamic tools defined. Use 'add' to create one.".to_string(),
            ));
        }

        let mut output = format!("Dynamic tools ({}):\n\n", defs.len());
        for def in &defs {
            let status = if def.enabled { "enabled" } else { "disabled" };
            let executor = match def.executor {
                ExecutorType::Http => "http",
                ExecutorType::Shell => "shell",
            };
            output.push_str(&format!(
                "  {} [{}] ({})\n    {}\n",
                def.name, status, executor, def.description
            ));
            if !def.params.is_empty() {
                output.push_str("    params: ");
                let param_strs: Vec<String> = def
                    .params
                    .iter()
                    .map(|p| {
                        if p.required {
                            format!("{}*", p.name)
                        } else {
                            p.name.clone()
                        }
                    })
                    .collect();
                output.push_str(&param_strs.join(", "));
                output.push('\n');
            }
        }
        Ok(ToolResult::success(output))
    }

    fn handle_add(&self, input: &Value) -> Result<ToolResult> {
        let name = match input["name"].as_str() {
            Some(n) if !n.is_empty() => n,
            _ => return Ok(ToolResult::error("'name' is required for add".to_string())),
        };
        let description = match input["description"].as_str() {
            Some(d) if !d.is_empty() => d,
            _ => {
                return Ok(ToolResult::error(
                    "'description' is required for add".to_string(),
                ));
            }
        };
        let executor = match input["executor"].as_str() {
            Some("http") => ExecutorType::Http,
            Some("shell") => ExecutorType::Shell,
            _ => {
                return Ok(ToolResult::error(
                    "'executor' is required: http or shell".to_string(),
                ));
            }
        };

        // Parse params
        let params = if let Some(arr) = input["params"].as_array() {
            arr.iter()
                .filter_map(|p| {
                    let pname = p["name"].as_str()?;
                    Some(ParamDef {
                        name: pname.to_string(),
                        param_type: p["type"].as_str().unwrap_or("string").to_string(),
                        description: p["description"].as_str().unwrap_or("").to_string(),
                        required: p["required"].as_bool().unwrap_or(true),
                        default: if p["default"].is_null() {
                            None
                        } else {
                            Some(p["default"].clone())
                        },
                    })
                })
                .collect()
        } else {
            Vec::new()
        };

        let def = DynamicToolDef {
            name: name.to_string(),
            description: description.to_string(),
            executor,
            method: input["method"].as_str().map(|s| s.to_string()),
            url: input["url"].as_str().map(|s| s.to_string()),
            headers: input["headers"]
                .as_object()
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default(),
            command: input["command"].as_str().map(|s| s.to_string()),
            params,
            timeout_secs: input["timeout_secs"].as_u64().unwrap_or(30),
            requires_approval: input["requires_approval"].as_bool().unwrap_or(true),
            enabled: true,
        };

        match DynamicToolLoader::add_tool(&self.tools_path, def, &self.registry) {
            Ok(()) => Ok(ToolResult::success(format!(
                "Dynamic tool '{name}' added and registered. It's now available in your tool list."
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to add tool: {e}"))),
        }
    }

    fn handle_remove(&self, input: &Value) -> Result<ToolResult> {
        let name = match input["name"].as_str() {
            Some(n) if !n.is_empty() => n,
            _ => {
                return Ok(ToolResult::error(
                    "'name' is required for remove".to_string(),
                ));
            }
        };

        match DynamicToolLoader::remove_tool(&self.tools_path, name, &self.registry) {
            Ok(true) => Ok(ToolResult::success(format!(
                "Dynamic tool '{name}' removed and unregistered."
            ))),
            Ok(false) => Ok(ToolResult::error(format!(
                "Tool '{name}' not found in dynamic tools."
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to remove tool: {e}"))),
        }
    }

    fn handle_set_enabled(&self, input: &Value, enabled: bool) -> Result<ToolResult> {
        let name = match input["name"].as_str() {
            Some(n) if !n.is_empty() => n,
            _ => {
                return Ok(ToolResult::error(
                    "'name' is required for enable/disable".to_string(),
                ));
            }
        };
        let action_word = if enabled { "enabled" } else { "disabled" };

        match DynamicToolLoader::set_enabled(&self.tools_path, name, enabled, &self.registry) {
            Ok(true) => Ok(ToolResult::success(format!(
                "Dynamic tool '{name}' {action_word}."
            ))),
            Ok(false) => Ok(ToolResult::error(format!(
                "Tool '{name}' not found in dynamic tools."
            ))),
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to {action_word} tool: {e}"
            ))),
        }
    }

    fn handle_reload(&self) -> Result<ToolResult> {
        let count = DynamicToolLoader::reload(&self.tools_path, &self.registry);
        Ok(ToolResult::success(format!(
            "Reloaded {count} dynamic tool(s) from {}",
            self.tools_path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::TempDir;
    use uuid::Uuid;

    fn setup() -> (Arc<ToolRegistry>, PathBuf, ToolManageTool) {
        let dir = TempDir::new().unwrap();
        let tools_path = dir.into_path().join("tools.toml");
        let registry = Arc::new(ToolRegistry::new());
        let tool = ToolManageTool::new(registry.clone(), tools_path.clone());
        (registry, tools_path, tool)
    }

    fn ctx() -> ToolExecutionContext {
        ToolExecutionContext::new(Uuid::new_v4()).with_auto_approve(true)
    }

    #[tokio::test]
    async fn test_list_empty() {
        let (_reg, _path, tool) = setup();
        let result = tool
            .execute(serde_json::json!({"action": "list"}), &ctx())
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("No dynamic tools"));
    }

    #[tokio::test]
    async fn test_add_shell_tool() {
        let (reg, _path, tool) = setup();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "add",
                    "name": "my_echo",
                    "description": "Echo a message",
                    "executor": "shell",
                    "command": "echo {{msg}}",
                    "requires_approval": false,
                    "params": [{"name": "msg", "type": "string", "required": true}]
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.success, "add failed: {:?}", result.error);
        assert!(reg.has_tool("my_echo"));
    }

    #[tokio::test]
    async fn test_add_then_list() {
        let (_reg, _path, tool) = setup();
        tool.execute(
            serde_json::json!({
                "action": "add",
                "name": "test_tool",
                "description": "A test tool",
                "executor": "shell",
                "command": "echo test"
            }),
            &ctx(),
        )
        .await
        .unwrap();

        let result = tool
            .execute(serde_json::json!({"action": "list"}), &ctx())
            .await
            .unwrap();
        assert!(result.output.contains("test_tool"));
        assert!(result.output.contains("enabled"));
    }

    #[tokio::test]
    async fn test_remove_tool() {
        let (reg, _path, tool) = setup();
        // Add first
        tool.execute(
            serde_json::json!({
                "action": "add",
                "name": "removable",
                "description": "Will be removed",
                "executor": "shell",
                "command": "echo bye"
            }),
            &ctx(),
        )
        .await
        .unwrap();
        assert!(reg.has_tool("removable"));

        // Remove
        let result = tool
            .execute(
                serde_json::json!({"action": "remove", "name": "removable"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(!reg.has_tool("removable"));
    }

    #[tokio::test]
    async fn test_disable_enable() {
        let (reg, _path, tool) = setup();
        tool.execute(
            serde_json::json!({
                "action": "add",
                "name": "toggleable",
                "description": "Can be toggled",
                "executor": "shell",
                "command": "echo hi"
            }),
            &ctx(),
        )
        .await
        .unwrap();
        assert!(reg.has_tool("toggleable"));

        // Disable
        let result = tool
            .execute(
                serde_json::json!({"action": "disable", "name": "toggleable"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(!reg.has_tool("toggleable"));

        // Enable
        let result = tool
            .execute(
                serde_json::json!({"action": "enable", "name": "toggleable"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(reg.has_tool("toggleable"));
    }

    #[tokio::test]
    async fn test_reload() {
        let (reg, path, tool) = setup();
        // Write a tools.toml directly
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[[tools]]
name = "from_disk"
description = "Loaded from disk"
executor = "shell"
command = "echo disk"
"#
        )
        .unwrap();

        let result = tool
            .execute(serde_json::json!({"action": "reload"}), &ctx())
            .await
            .unwrap();
        assert!(result.success);
        assert!(reg.has_tool("from_disk"));
    }

    #[tokio::test]
    async fn test_add_missing_name() {
        let (_reg, _path, tool) = setup();
        let result = tool
            .execute(
                serde_json::json!({"action": "add", "executor": "shell"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_add_missing_executor() {
        let (_reg, _path, tool) = setup();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "add",
                    "name": "no_exec",
                    "description": "Missing executor"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let (_reg, _path, tool) = setup();
        let result = tool
            .execute(serde_json::json!({"action": "destroy"}), &ctx())
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_add_http_tool() {
        let (reg, _path, tool) = setup();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "add",
                    "name": "health_check",
                    "description": "Check server health",
                    "executor": "http",
                    "method": "GET",
                    "url": "https://example.com/health",
                    "timeout_secs": 10,
                    "headers": {"Authorization": "Bearer {{token}}"},
                    "params": [{"name": "token", "type": "string", "required": true}]
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.success, "add http failed: {:?}", result.error);
        assert!(reg.has_tool("health_check"));

        // Verify it shows up in tool definitions
        let defs = reg.get_tool_definitions();
        let hc = defs.iter().find(|t| t.name == "health_check").unwrap();
        assert!(hc.description.contains("health"));
    }

    #[tokio::test]
    async fn test_remove_nonexistent() {
        let (_reg, _path, tool) = setup();
        let result = tool
            .execute(
                serde_json::json!({"action": "remove", "name": "ghost"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.success);
    }
}
