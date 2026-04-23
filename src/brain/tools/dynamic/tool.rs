//! Dynamic tool types and executor.
//!
//! `DynamicToolDef` is the TOML-serializable definition.
//! `DynamicTool` wraps a definition and implements the `Tool` trait.

use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Executor type for a dynamic tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorType {
    Http,
    Shell,
}

/// Parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDef {
    pub name: String,
    #[serde(rename = "type", default = "default_string_type")]
    pub param_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub default: Option<Value>,
}

/// A single dynamic tool definition as parsed from tools.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicToolDef {
    pub name: String,
    pub description: String,
    pub executor: ExecutorType,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub requires_approval: bool,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub params: Vec<ParamDef>,
}

fn default_true() -> bool {
    true
}
fn default_timeout() -> u64 {
    30
}
fn default_string_type() -> String {
    "string".to_string()
}

impl DynamicToolDef {
    pub fn input_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for param in &self.params {
            let mut prop = serde_json::Map::new();
            prop.insert("type".into(), Value::String(param.param_type.clone()));
            if !param.description.is_empty() {
                prop.insert(
                    "description".into(),
                    Value::String(param.description.clone()),
                );
            }
            if let Some(ref default) = param.default {
                prop.insert("default".into(), default.clone());
            }
            properties.insert(param.name.clone(), Value::Object(prop));
            if param.required {
                required.push(Value::String(param.name.clone()));
            }
        }
        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required
        })
    }

    pub fn render_template(template: &str, params: &Value) -> String {
        let mut result = template.to_string();
        if let Some(obj) = params.as_object() {
            for (key, value) in obj {
                let placeholder = format!("{{{{{}}}}}", key);
                let replacement = match value {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                result = result.replace(&placeholder, &replacement);
            }
        }
        result
    }
}

/// Top-level tools.toml structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DynamicToolsConfig {
    #[serde(default)]
    pub tools: Vec<DynamicToolDef>,
}

/// Runtime tool wrapping a TOML definition.
pub struct DynamicTool {
    def: DynamicToolDef,
}

impl DynamicTool {
    pub fn new(def: DynamicToolDef) -> Self {
        Self { def }
    }

    fn extract_params(&self, input: &Value) -> Value {
        let mut out = serde_json::Map::new();
        let obj = input.as_object();
        for p in &self.def.params {
            let val = obj
                .and_then(|o| o.get(&p.name))
                .cloned()
                .or_else(|| p.default.clone());
            if let Some(v) = val {
                out.insert(p.name.clone(), v);
            }
        }
        Value::Object(out)
    }

    async fn execute_http(&self, params: &Value) -> Result<ToolResult> {
        let url = match &self.def.url {
            Some(u) => DynamicToolDef::render_template(u, params),
            None => return Ok(ToolResult::error("HTTP tool missing 'url' field".into())),
        };
        let method = self.def.method.as_deref().unwrap_or("GET").to_uppercase();
        let client = reqwest::Client::new();
        let mut req = match method.as_str() {
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "PATCH" => client.patch(&url),
            "DELETE" => client.delete(&url),
            _ => client.get(&url),
        };
        for (k, v) in &self.def.headers {
            let rendered = DynamicToolDef::render_template(v, params);
            req = req.header(k.as_str(), rendered);
        }
        let timeout = std::time::Duration::from_secs(self.def.timeout_secs);
        match req.timeout(timeout).send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    Ok(ToolResult::success(body))
                } else {
                    Ok(ToolResult::error(format!(
                        "HTTP {} {}: {}",
                        status.as_u16(),
                        status.canonical_reason().unwrap_or(""),
                        body
                    )))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("HTTP request failed: {e}"))),
        }
    }

    async fn execute_shell(
        &self,
        params: &Value,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let cmd = match &self.def.command {
            Some(c) => DynamicToolDef::render_template(c, params),
            None => {
                return Ok(ToolResult::error(
                    "Shell tool missing 'command' field".into(),
                ));
            }
        };
        // Detach stdin from the parent TTY so mouse-capture bytes don't
        // leak into captured stdout (same TUI-bleed issue as bash.rs).
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .current_dir(&context.working_directory)
            .stdin(std::process::Stdio::null())
            .output()
            .await;
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if out.status.success() {
                    let mut result = stdout;
                    if !stderr.is_empty() {
                        result.push_str("\n[stderr] ");
                        result.push_str(&stderr);
                    }
                    Ok(ToolResult::success(result))
                } else {
                    Ok(ToolResult::error(format!(
                        "Exit code {}: {}{}",
                        out.status.code().unwrap_or(-1),
                        stdout,
                        if stderr.is_empty() {
                            String::new()
                        } else {
                            format!("\n[stderr] {stderr}")
                        }
                    )))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to spawn shell: {e}"))),
        }
    }
}

#[async_trait]
impl Tool for DynamicTool {
    fn name(&self) -> &str {
        &self.def.name
    }
    fn description(&self) -> &str {
        &self.def.description
    }
    fn input_schema(&self) -> Value {
        self.def.input_schema()
    }
    fn capabilities(&self) -> Vec<ToolCapability> {
        match self.def.executor {
            ExecutorType::Http => vec![ToolCapability::Network],
            ExecutorType::Shell => vec![ToolCapability::ExecuteShell],
        }
    }
    fn requires_approval(&self) -> bool {
        self.def.requires_approval
    }
    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let params = self.extract_params(&input);
        tracing::info!(
            "Executing dynamic tool '{}' ({:?})",
            self.def.name,
            self.def.executor
        );
        match self.def.executor {
            ExecutorType::Http => self.execute_http(&params).await,
            ExecutorType::Shell => self.execute_shell(&params, context).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_shell(name: &str, cmd: &str, params: Vec<ParamDef>) -> DynamicTool {
        DynamicTool::new(DynamicToolDef {
            name: name.into(),
            description: format!("Test: {name}"),
            executor: ExecutorType::Shell,
            enabled: true,
            requires_approval: false,
            method: None,
            url: None,
            headers: HashMap::new(),
            timeout_secs: 10,
            command: Some(cmd.into()),
            params,
        })
    }

    fn ctx() -> ToolExecutionContext {
        ToolExecutionContext::new(Uuid::new_v4())
    }

    #[test]
    fn test_name() {
        assert_eq!(make_shell("t", "echo", vec![]).name(), "t");
    }

    #[test]
    fn test_capabilities() {
        assert_eq!(
            make_shell("s", "echo", vec![]).capabilities(),
            vec![ToolCapability::ExecuteShell]
        );
    }

    #[test]
    fn test_input_schema() {
        let tool = make_shell(
            "echo",
            "echo {{msg}}",
            vec![ParamDef {
                name: "msg".into(),
                param_type: "string".into(),
                description: "Msg".into(),
                required: true,
                default: None,
            }],
        );
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "msg");
    }

    #[test]
    fn test_extract_params_with_defaults() {
        let tool = make_shell(
            "echo",
            "echo {{msg}} {{count}}",
            vec![
                ParamDef {
                    name: "msg".into(),
                    param_type: "string".into(),
                    description: "".into(),
                    required: true,
                    default: None,
                },
                ParamDef {
                    name: "count".into(),
                    param_type: "integer".into(),
                    description: "".into(),
                    required: false,
                    default: Some(serde_json::json!(3)),
                },
            ],
        );
        let params = tool.extract_params(&serde_json::json!({"msg": "hello"}));
        assert_eq!(params["msg"], "hello");
        assert_eq!(params["count"], 3);
    }

    #[test]
    fn test_template_rendering() {
        let result = DynamicToolDef::render_template(
            "deploy {{branch}} x{{count}}",
            &serde_json::json!({"branch": "main", "count": 3}),
        );
        assert_eq!(result, "deploy main x3");
    }

    #[test]
    fn test_parse_toml() {
        let config: DynamicToolsConfig = toml::from_str(
            r#"
[[tools]]
name = "check"
description = "Check health"
executor = "http"
method = "GET"
url = "https://example.com/health"
"#,
        )
        .unwrap();
        assert_eq!(config.tools.len(), 1);
        assert_eq!(config.tools[0].executor, ExecutorType::Http);
    }

    #[test]
    fn test_roundtrip_toml() {
        let config = DynamicToolsConfig {
            tools: vec![DynamicToolDef {
                name: "ping".into(),
                description: "Ping".into(),
                executor: ExecutorType::Shell,
                enabled: true,
                requires_approval: false,
                method: None,
                url: None,
                headers: HashMap::new(),
                timeout_secs: 30,
                command: Some("ping -c 1 {{host}}".into()),
                params: vec![ParamDef {
                    name: "host".into(),
                    param_type: "string".into(),
                    description: "".into(),
                    required: true,
                    default: None,
                }],
            }],
        };
        let content = toml::to_string_pretty(&config).unwrap();
        let loaded: DynamicToolsConfig = toml::from_str(&content).unwrap();
        assert_eq!(loaded.tools[0].name, "ping");
    }

    #[tokio::test]
    async fn test_execute_shell_echo() {
        let tool = make_shell("echo_test", "echo hello", vec![]);
        let result = tool.execute(serde_json::json!({}), &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_shell_failure() {
        let result = make_shell("fail", "exit 42", vec![])
            .execute(serde_json::json!({}), &ctx())
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_missing_command() {
        let t = DynamicTool::new(DynamicToolDef {
            name: "b".into(),
            description: "".into(),
            executor: ExecutorType::Shell,
            enabled: true,
            requires_approval: false,
            method: None,
            url: None,
            headers: HashMap::new(),
            timeout_secs: 5,
            command: None,
            params: vec![],
        });
        let result = t.execute(serde_json::json!({}), &ctx()).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_missing_url() {
        let t = DynamicTool::new(DynamicToolDef {
            name: "h".into(),
            description: "".into(),
            executor: ExecutorType::Http,
            enabled: true,
            requires_approval: false,
            method: None,
            url: None,
            headers: HashMap::new(),
            timeout_secs: 5,
            command: None,
            params: vec![],
        });
        let result = t.execute(serde_json::json!({}), &ctx()).await.unwrap();
        assert!(!result.success);
    }
}
