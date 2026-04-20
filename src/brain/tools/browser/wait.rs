//! browser_wait — Wait for a CSS selector to appear or a fixed delay.

use super::manager::BrowserManager;
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub struct BrowserWaitTool {
    manager: Arc<BrowserManager>,
}

impl BrowserWaitTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BrowserWaitTool {
    fn name(&self) -> &str {
        "browser_wait"
    }

    fn description(&self) -> &str {
        "Wait for a CSS selector to appear on the page, or wait a fixed number of seconds. \
         Polls every 200ms up to the timeout."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "CSS selector to wait for"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Max seconds to wait (default: 10)"
                },
                "delay_secs": {
                    "type": "integer",
                    "description": "Fixed delay in seconds (if no selector, just wait this long)"
                }
            }
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let selector = input["selector"].as_str();
        let timeout_secs = input["timeout_secs"].as_u64().unwrap_or(10);
        let delay_secs = input["delay_secs"].as_u64();

        // Fixed delay mode
        if let Some(secs) = delay_secs
            && selector.is_none()
        {
            let secs = secs.min(30);
            tokio::time::sleep(Duration::from_secs(secs)).await;
            return Ok(ToolResult::success(format!("Waited {secs} seconds")));
        }

        let sel = match selector {
            Some(s) if !s.is_empty() => s,
            _ => {
                return Ok(ToolResult::error(
                    "'selector' or 'delay_secs' required".into(),
                ));
            }
        };

        let page = match self
            .manager
            .get_or_create_session_page(context.session_id)
            .await
        {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            match page.find_element(sel).await {
                Ok(_) => {
                    return Ok(ToolResult::success(format!(
                        "Element '{sel}' found on page"
                    )));
                }
                Err(_) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Ok(ToolResult::error(format!(
                            "Timeout: '{sel}' not found after {timeout_secs}s"
                        )));
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }
}
