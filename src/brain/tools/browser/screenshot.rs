//! browser_screenshot — Capture a screenshot of the current page.

use super::manager::BrowserManager;
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use base64::Engine;
use serde_json::Value;
use std::sync::Arc;

pub struct BrowserScreenshotTool {
    manager: Arc<BrowserManager>,
}

impl BrowserScreenshotTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BrowserScreenshotTool {
    fn name(&self) -> &str {
        "browser_screenshot"
    }

    fn description(&self) -> &str {
        "Capture a screenshot of the current page. Returns base64-encoded PNG. \
         Optionally screenshot a specific element by CSS selector."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "CSS selector of element to screenshot (default: full page)"
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

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let selector = input["selector"].as_str();

        let page = match self.manager.get_or_create_page(None).await {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        let bytes = if let Some(sel) = selector {
            // Screenshot a specific element
            let element = match page.find_element(sel).await {
                Ok(el) => el,
                Err(e) => return Ok(ToolResult::error(format!("Element '{sel}' not found: {e}"))),
            };
            match element
                .screenshot(
                    chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
                )
                .await
            {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::error(format!("Element screenshot failed: {e}"))),
            }
        } else {
            // Full page screenshot
            match page
                .screenshot(
                    chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams::builder()
                        .format(
                            chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
                        )
                        .build(),
                )
                .await
            {
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::error(format!("Screenshot failed: {e}"))),
            }
        };

        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        Ok(ToolResult::success(format!("data:image/png;base64,{b64}")))
    }
}
