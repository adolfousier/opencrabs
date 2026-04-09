//! browser_type — Type text into an element or the focused element.

use super::manager::BrowserManager;
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct BrowserTypeTool {
    manager: Arc<BrowserManager>,
}

impl BrowserTypeTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BrowserTypeTool {
    fn name(&self) -> &str {
        "browser_type"
    }

    fn description(&self) -> &str {
        "Type text into a focused element or an element found by CSS selector. Returns an automatic screenshot after typing."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Text to type"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector of the input element (optional — types into focused element if omitted)"
                }
            },
            "required": ["text"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let text = match input["text"].as_str() {
            Some(t) if !t.is_empty() => t,
            _ => return Ok(ToolResult::error("'text' is required".into())),
        };
        let selector = input["selector"].as_str();

        let page = match self.manager.get_or_create_page(None).await {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        if let Some(sel) = selector {
            let element = match page.find_element(sel).await {
                Ok(el) => el,
                Err(e) => return Ok(ToolResult::error(format!("Element '{sel}' not found: {e}"))),
            };

            // Click to focus, then type
            if let Err(e) = element.click().await {
                return Ok(ToolResult::error(format!("Failed to focus element: {e}")));
            }

            if let Err(e) = element.type_str(text).await {
                return Ok(ToolResult::error(format!("Typing failed: {e}")));
            }

            let mut result = ToolResult::success(format!("Typed '{}' into {}", text, sel));

            // Auto-screenshot: give the model vision after typing
            if let Some(img) = self.manager.take_screenshot().await {
                result.images.push(img);
            }

            Ok(result)
        } else {
            // Type into the currently focused element via JS
            let js = format!(
                "document.activeElement && document.activeElement.value !== undefined ? \
                 (document.activeElement.value += '{}', true) : false",
                text.replace('\\', "\\\\").replace('\'', "\\'")
            );
            match page.evaluate(js.as_str()).await {
                Ok(result) => {
                    let ok = result
                        .value()
                        .and_then(|v: &serde_json::Value| v.as_bool())
                        .unwrap_or(false);
                    if ok {
                        let mut result =
                            ToolResult::success(format!("Typed '{}' into focused element", text));
                        if let Some(img) = self.manager.take_screenshot().await {
                            result.images.push(img);
                        }
                        Ok(result)
                    } else {
                        Ok(ToolResult::error(
                            "No focused input element found. Use 'selector' to target an element."
                                .into(),
                        ))
                    }
                }
                Err(e) => Ok(ToolResult::error(format!("Typing failed: {e}"))),
            }
        }
    }
}
