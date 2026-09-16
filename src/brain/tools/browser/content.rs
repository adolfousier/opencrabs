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

        let page = match self
            .manager
            .get_or_create_session_page(context.session_id)
            .await
        {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        let content = if let Some(sel) = selector {
            // Get content of specific element. `__ocQueryOne` so a
            // selector pointing inside an open shadow root resolves here
            // exactly as it does in browser_click / browser_type; the
            // selector is JSON-encoded so quotes and backslashes in it
            // cannot break out of the literal.
            let sel_js = serde_json::to_string(sel).unwrap_or_else(|_| "null".into());
            let prop = if text_only { "innerText" } else { "innerHTML" };
            let js = super::shadow::with_deep_helpers(&format!(
                "const el = __ocQueryOne({sel_js});
                 return el ? (el.{prop} || '') : '(element not found)';"
            ));
            match page.evaluate(js.as_str()).await {
                Ok(result) => result
                    .value()
                    .and_then(|v: &serde_json::Value| v.as_str())
                    .unwrap_or("(no result)")
                    .to_string(),
                Err(e) => return Ok(ToolResult::error(format!("Content extraction failed: {e}"))),
            }
        } else if text_only {
            // Full page text. `innerText` is computed per node tree and
            // stops at a shadow boundary, so `document.body.innerText`
            // alone silently omits everything rendered inside a custom
            // element (measured, not assumed — pinned by
            // `body_inner_text_includes_shadow_text` in the e2e fixture).
            //
            // Unlike the full-page HTML path below, joining per-tree text
            // is not an output-sizing problem: the result is bounded by
            // what is actually rendered on screen, which is what the
            // caller asked for. Each root contributes its own tree only
            // (a ShadowRoot's text does not include nested shadow trees),
            // so nothing is counted twice.
            // RAW string: the JS below contains a `\n` escape that a normal
            // Rust literal would collapse into a real newline, producing
            // `join('<LF>')` — a JS SyntaxError, and the eval fails with an
            // empty message that reads like a browser problem.
            let js = super::shadow::with_deep_helpers(
                r#"const parts = [];
                 for (const r of __ocRoots()) {
                     const base = r === document ? document.body : r;
                     if (!base) continue;
                     if (base.innerText !== undefined) {
                         parts.push(base.innerText);
                         continue;
                     }
                     for (const c of base.children) {
                         parts.push(c.innerText || c.textContent || '');
                     }
                 }
                 return parts.filter(p => p && p.trim()).join('\n');"#,
            );
            match page.evaluate(js.as_str()).await {
                Ok(result) => result
                    .value()
                    .and_then(|v: &serde_json::Value| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                Err(e) => return Ok(ToolResult::error(format!("Content extraction failed: {e}"))),
            }
        } else {
            // Full page HTML. Deliberately NOT chromey's
            // `outer_html_full` (which serializes the composed tree
            // including shadow roots): this path applies no output cap
            // at all, so dumping every shadow tree would be an
            // output-sizing change wearing a shadow-DOM costume. It
            // needs a cap first. Selector-scoped extraction above does
            // pierce, which is the case the agent actually reaches for.
            match page.content().await {
                Ok(html) => html,
                Err(e) => return Ok(ToolResult::error(format!("Failed to get page HTML: {e}"))),
            }
        };

        Ok(ToolResult::success(super::events::append_line(
            content,
            self.manager.drain_recent_events(),
        )))
    }
}
