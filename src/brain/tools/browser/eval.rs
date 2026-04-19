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

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let script = match input["script"].as_str() {
            Some(c) if !c.is_empty() => c,
            _ => return Ok(ToolResult::error("'script' is required".into())),
        };

        let page = match self.manager.get_or_create_session_page(context.session_id).await {
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
                Ok(ToolResult::success(cap_eval_output(output)))
            }
            Err(e) => Ok(ToolResult::error(format!("JS execution failed: {e}"))),
        }
    }
}

/// Maximum number of bytes from a single `browser_eval` result that
/// reach the LLM. The agent has called eval scripts that return the
/// entire `document.body.outerHTML` of JS-heavy sites (multi-megabyte)
/// which burns the whole context window on one tool result — the
/// result is truncated with a trailing note so the model knows more
/// data exists.
const EVAL_OUTPUT_MAX_BYTES: usize = 50_000;

/// Truncate an eval result to `EVAL_OUTPUT_MAX_BYTES`, appending a
/// byte-count note so the model knows the truncation happened. UTF-8
/// safe: always splits at a char boundary.
///
/// `pub(crate)` so `src/tests/browser_eval_cap_test.rs` can exercise
/// the boundary conditions directly.
pub(crate) fn cap_eval_output(output: String) -> String {
    if output.len() <= EVAL_OUTPUT_MAX_BYTES {
        return output;
    }
    let mut end = EVAL_OUTPUT_MAX_BYTES;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n[truncated — result was {} bytes total, showing first {}]",
        &output[..end],
        output.len(),
        end
    )
}
