//! browser_click — Click an element by CSS selector.

use super::manager::{BrowserManager, split_frame_selector};
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{
    Tool, ToolCapability, ToolExecutionContext, ToolHints, ToolResult,
};
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
        "Click an element on the page. Accepts three selector shapes:\n\
         - CSS selector (default): `button.primary`, `#submit`, `[data-id=\"x\"]`\n\
         - Text contains: `text=Sign in` finds the first visible element whose innerText contains the substring (case-insensitive). Use when you don't know the CSS but can see the label.\n\
         - XPath: `xpath=//button[contains(., 'Submit')]` for precise structural queries.\n\
         Returns an automatic screenshot after the click."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "selector": {
                    "type": "string",
                    "description": "Element to click. Prefix-based: `text=Label` for text match, `xpath=//...` for XPath, anything else is treated as CSS."
                }
            },
            "required": ["selector"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: true,
        }
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let selector_input = match input["selector"].as_str() {
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

        // Frame-routed selectors (#1190): `f2:14` or `f2:[data-...]` —
        // the model copied a namespaced index from a cross-origin
        // iframe's inventory. Resolve the owning frame page and strip
        // the prefix before the normal CSS path runs. Plain selectors
        // keep working on the main page exactly as before.
        let (page, selector): (chromiumoxide::Page, String) =
            if let Some((label, rest)) = split_frame_selector(selector_input) {
                match self
                    .manager
                    .oopif_page_by_label(context.session_id, &label)
                    .await
                {
                    Ok(Some(p)) => (p, rest),
                    Ok(None) => {
                        return Ok(ToolResult::error(format!(
                            "Frame '{label}' not found. The frame may have navigated away — \
                             re-run `browser_find` to get a fresh inventory."
                        )));
                    }
                    Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
                }
            } else {
                (page, selector_input.to_string())
            };

        // `text=...` and `xpath=...` selectors are Playwright-style and not
        // valid CSS — translate them to a JS evaluator that finds the first
        // visible match and clicks it in-page. Returns true on success, the
        // string "not_found" when no element matches, or "not_clickable" if
        // the match exists but has no click handler / is not visible.
        if let Some(text) = selector.strip_prefix("text=") {
            let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
            let js = format!(
                r#"
                (() => {{
                    const needle = "{escaped}".toLowerCase();
                    const walker = document.createTreeWalker(
                        document.body, NodeFilter.SHOW_ELEMENT);
                    let node;
                    while ((node = walker.nextNode())) {{
                        const t = (node.innerText || node.textContent || "").toLowerCase();
                        if (!t.includes(needle)) continue;
                        const r = node.getBoundingClientRect();
                        if (r.width === 0 || r.height === 0) continue;
                        node.scrollIntoView({{block: "center"}});
                        node.click();
                        return "ok";
                    }}
                    return "not_found";
                }})()
                "#
            );
            match page.evaluate(js.as_str()).await {
                Ok(r) => {
                    let result = r.value().and_then(|v| v.as_str().map(String::from));
                    match result.as_deref() {
                        Some("ok") => {
                            let _ = page
                                .wait_for_network_almost_idle_with_timeout(
                                    std::time::Duration::from_secs(3),
                                )
                                .await;
                            // Reset consecutive-identical screenshot counter — click is a page change
                            self.manager
                                .reset_identical_screenshot_count(context.session_id)
                                .await;
                            let mut tr = ToolResult::success(super::events::append_line(
                                format!("Clicked: {selector}"),
                                self.manager.drain_recent_events(),
                            ));
                            self.manager
                                .attach_screenshot(context.session_id, &mut tr)
                                .await;
                            return Ok(tr);
                        }
                        _ => {
                            return Ok(ToolResult::error(format!(
                                "No visible element matched text '{text}'. \
                                 Use `browser_find` with mode=\"text\" pattern=\"{text}\" \
                                 to enumerate candidates, then click by the returned \
                                 `[data-opencrabs-match=\"N\"]` selector."
                            )));
                        }
                    }
                }
                Err(e) => {
                    return Ok(ToolResult::error(format!("text-click eval failed: {e}")));
                }
            }
        }

        if let Some(xpath) = selector.strip_prefix("xpath=") {
            let escaped = xpath.replace('\\', "\\\\").replace('"', "\\\"");
            let js = format!(
                r#"
                (() => {{
                    const it = document.evaluate("{escaped}", document, null,
                        XPathResult.FIRST_ORDERED_NODE_TYPE, null);
                    const node = it.singleNodeValue;
                    if (!node) return "not_found";
                    const r = node.getBoundingClientRect();
                    if (r.width === 0 || r.height === 0) return "not_visible";
                    node.scrollIntoView({{block: "center"}});
                    node.click();
                    return "ok";
                }})()
                "#
            );
            match page.evaluate(js.as_str()).await {
                Ok(r) => {
                    let result = r.value().and_then(|v| v.as_str().map(String::from));
                    match result.as_deref() {
                        Some("ok") => {
                            let _ = page
                                .wait_for_network_almost_idle_with_timeout(
                                    std::time::Duration::from_secs(3),
                                )
                                .await;
                            // Reset consecutive-identical screenshot counter — click is a page change
                            self.manager
                                .reset_identical_screenshot_count(context.session_id)
                                .await;
                            let mut tr = ToolResult::success(super::events::append_line(
                                format!("Clicked: {selector}"),
                                self.manager.drain_recent_events(),
                            ));
                            self.manager
                                .attach_screenshot(context.session_id, &mut tr)
                                .await;
                            return Ok(tr);
                        }
                        Some("not_visible") => {
                            return Ok(ToolResult::error(format!(
                                "XPath '{xpath}' matched but element is not visible."
                            )));
                        }
                        _ => {
                            return Ok(ToolResult::error(format!(
                                "XPath '{xpath}' matched no nodes."
                            )));
                        }
                    }
                }
                Err(e) => {
                    return Ok(ToolResult::error(format!("xpath-click eval failed: {e}")));
                }
            }
        }

        let element = match super::manager::resolve_element(&page, &selector).await {
            Ok(el) => el,
            Err(e) => {
                // Surface the recovery path inline so the agent doesn't
                // spend several iterations guessing alternate CSS. The
                // logs from 2026-05-23 09:04 show ~25 wasted iterations
                // after a single failed CSS click before the agent
                // figured out browser_find on its own.
                let hint = if selector.contains(' ') || selector.contains('|') {
                    " Looks like a label or descriptive string rather than \
                     CSS — try `text=...` (e.g. `text=Sign in`) instead."
                } else {
                    " Try `browser_find` with mode=\"text\"/\"aria\"/\"role\" \
                     to locate the element, then click by the returned \
                     `[data-opencrabs-match=\"N\"]` selector. Or use \
                     `text=Label` directly if you know the visible text."
                };
                return Ok(ToolResult::error(format!(
                    "Element '{selector}' not found: {e}.{hint}"
                )));
            }
        };

        if let Err(e) = element.click().await {
            return Ok(ToolResult::error(format!("Click failed: {e}")));
        }

        // Reset consecutive-identical screenshot counter — click is a page change
        self.manager
            .reset_identical_screenshot_count(context.session_id)
            .await;

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
