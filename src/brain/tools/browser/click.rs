//! browser_click — Click an element by CSS selector.

use super::manager::BrowserManager;
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct BrowserClickTool {
    manager: Arc<BrowserManager>,
}

impl BrowserClickTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BrowserClickTool {
    fn name(&self) -> &str {
        "browser_click"
    }

    fn description(&self) -> &str {
        "Click an element on the page by CSS selector. Returns an automatic screenshot after the click."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "CSS selector of the element to click"
                }
            },
            "required": ["selector"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let selector = match input["selector"].as_str() {
            Some(s) if !s.is_empty() => s,
            _ => return Ok(ToolResult::error("'selector' is required".into())),
        };

        let page = match self
            .manager
            .get_or_create_session_page(context.session_id)
            .await
        {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        let element = match page.find_element(selector).await {
            Ok(el) => el,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Element '{selector}' not found: {e}"
                )));
            }
        };

        if let Err(e) = element.click().await {
            return Ok(ToolResult::error(format!("Click failed: {e}")));
        }

        // Wait for the page to settle after click — navigation, AJAX,
        // hydration. `wait_for_network_almost_idle_with_timeout` resolves
        // once ≤2 requests have been in flight for 500ms, or the timeout
        // elapses (silently). This beats the old fixed 500ms sleep:
        //  - fast SPA updates proceed immediately without wasting 500ms
        //  - slow multi-request hydration gets the time it actually needs
        //  - a stuck page still falls through at the timeout cap
        let _ = page
            .wait_for_network_almost_idle_with_timeout(std::time::Duration::from_secs(3))
            .await;

        let mut result = ToolResult::success(format!("Clicked element: {selector}"));

        // Auto-screenshot: give the model vision of the page after clicking
        self.manager
            .attach_screenshot(context.session_id, &mut result)
            .await;

        Ok(result)
    }
}
