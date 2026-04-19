//! browser_content — Get page text content or HTML.

use super::manager::BrowserManager;
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct BrowserContentTool {
    manager: Arc<BrowserManager>,
}

impl BrowserContentTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BrowserContentTool {
    fn name(&self) -> &str {
        "browser_content"
    }

    fn description(&self) -> &str {
        "Get the current page content. Returns full HTML by default, or text-only content \
         of a specific CSS selector. Use 'text_only' to strip HTML tags."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "CSS selector to get content from (default: entire page)"
                },
                "text_only": {
                    "type": "boolean",
                    "description": "Return text content only, no HTML tags (default: false)"
                }
            }
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let selector = input["selector"].as_str();
        let text_only = input["text_only"].as_bool().unwrap_or(false);

        let page = match self.manager.get_or_create_session_page(context.session_id).await {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        let content = if let Some(sel) = selector {
            // Get content of specific element
            let js = if text_only {
                format!(
                    "document.querySelector('{}')?.innerText || '(element not found)'",
                    sel.replace('\'', "\\'")
                )
            } else {
                format!(
                    "document.querySelector('{}')?.innerHTML || '(element not found)'",
                    sel.replace('\'', "\\'")
                )
            };
            match page.evaluate(js.as_str()).await {
                Ok(result) => result
                    .value()
                    .and_then(|v: &serde_json::Value| v.as_str())
                    .unwrap_or("(no result)")
                    .to_string(),
                Err(e) => return Ok(ToolResult::error(format!("Content extraction failed: {e}"))),
            }
        } else if text_only {
            // Full page text
            match page.evaluate("document.body?.innerText || ''").await {
                Ok(result) => result
                    .value()
                    .and_then(|v: &serde_json::Value| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                Err(e) => return Ok(ToolResult::error(format!("Content extraction failed: {e}"))),
            }
        } else {
            // Full page HTML
            match page.content().await {
                Ok(html) => html,
                Err(e) => return Ok(ToolResult::error(format!("Failed to get page HTML: {e}"))),
            }
        };

        Ok(ToolResult::success(content))
    }
}
