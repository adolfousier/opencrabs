//! browser_navigate — Navigate to a URL and return page info.

use super::manager::BrowserManager;
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct BrowserNavigateTool {
    manager: Arc<BrowserManager>,
}

impl BrowserNavigateTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BrowserNavigateTool {
    fn name(&self) -> &str {
        "browser_navigate"
    }

    fn description(&self) -> &str {
        "Navigate to a URL in the browser. Returns the page title, final URL \
         (after redirects), and an automatic screenshot of the page. \
         Supports both headless and headed (visible) mode."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to navigate to"
                },
                "headless": {
                    "type": "boolean",
                    "description": "Run in headless mode (no visible window). Defaults to true. Set to false to see the browser."
                }
            },
            "required": ["url"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let url = match input["url"].as_str() {
            Some(u) if !u.is_empty() => u,
            _ => return Ok(ToolResult::error("'url' is required".into())),
        };

        // Switch headless/headed mode if requested
        if let Some(headless) = input["headless"].as_bool() {
            self.manager.set_headless(headless).await;
        }

        let page = match self.manager.get_or_create_session_page(context.session_id).await {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        if let Err(e) = page.goto(url).await {
            return Ok(ToolResult::error(format!("Navigation failed: {e}")));
        }

        // Wait for navigation to settle
        let _ = page.wait_for_navigation().await;

        let title = page.get_title().await.ok().flatten().unwrap_or_default();
        let final_url = page.url().await.ok().flatten().unwrap_or_default();

        let mut result = ToolResult::success(format!("Navigated to: {final_url}\nTitle: {title}"));

        // Auto-screenshot: give the model vision of the page after navigation
        self.manager
            .attach_screenshot(context.session_id, &mut result)
            .await;

        Ok(result)
    }
}
