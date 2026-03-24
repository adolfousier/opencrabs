//! browser_eval — Execute JavaScript in page context.

use super::manager::BrowserManager;
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct BrowserEvalTool {
    manager: Arc<BrowserManager>,
}

impl BrowserEvalTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BrowserEvalTool {
    fn name(&self) -> &str {
        "browser_eval"
    }

    fn description(&self) -> &str {
        "Execute JavaScript code in the browser page context and return the result. \
         Useful for extracting data, manipulating the DOM, or running complex interactions."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "JavaScript code to execute. Can be an expression or a function body."
                }
            },
            "required": ["script"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network, ToolCapability::ExecuteShell]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let script = match input["script"].as_str() {
            Some(c) if !c.is_empty() => c,
            _ => return Ok(ToolResult::error("'script' is required".into())),
        };

        let page = match self.manager.get_or_create_page(None).await {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        match page.evaluate(script).await {
            Ok(result) => {
                let value: Value = result.value().cloned().unwrap_or(Value::Null);
                let output = match &value {
                    Value::String(s) => s.clone(),
                    Value::Null => "(undefined)".to_string(),
                    other => serde_json::to_string_pretty(other).unwrap_or_default(),
                };
                Ok(ToolResult::success(output))
            }
            Err(e) => Ok(ToolResult::error(format!("JS execution failed: {e}"))),
        }
    }
}
