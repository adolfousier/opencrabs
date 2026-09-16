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

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let selector = input["selector"].as_str();

        let page = match self
            .manager
            .get_or_create_session_page(context.session_id)
            .await
        {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        // Pre-flight health check. `page.url()` is a cheap CDP call that
        // round-trips the underlying connection — if it fails the page is
        // not responding (CDP handler task exited, browser process died,
        // tab was closed externally, etc.). Without this the screenshot
        // call below would either hang or return a generic
        // "Screenshot failed: {e}" with no signal whether the page was
        // dead or the capture itself failed.
        if let Err(e) = page.url().await {
            return Ok(ToolResult::error(format!(
                "Screenshot failed: page is not responding (CDP connection may be dead). \
                 Underlying error: {e}. Try `browser_navigate` to reset the page or \
                 `browser_close` if the session is stuck."
            )));
        }

        let bytes = if let Some(sel) = selector {
            // Screenshot a specific element
            let element = match super::manager::resolve_element(&page, sel).await {
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

        // No-op repeat detection: if the bytes hash matches the previous
        // capture for this session, the page hasn't changed since the last
        // screenshot. Send the agent an actionable error instead of a 25KB
        // duplicate image — saves context tokens AND breaks the
        // screenshot-spam pattern at the tool layer (semantic-loop
        // detection in tool_loop.rs handles it from the agent side too,
        // this is the defense-in-depth half).
        //
        // Only applies to full-page screenshots: an element screenshot can
        // legitimately repeat for different selectors against a stable
        // page, so we don't dedupe those.
        //
        // After 3 consecutive identical captures, force-break with a hard
        // error to stop the screenshot loop entirely (#664).
        if selector.is_none() {
            let hash = BrowserManager::hash_screenshot_bytes(&bytes);
            if Some(hash) == self.manager.last_screenshot_hash(context.session_id).await {
                let count = self
                    .manager
                    .increment_identical_screenshot_count(context.session_id)
                    .await;
                if count >= 3 {
                    return Ok(ToolResult::error(format!(
                        "Screenshot loop detected: {count} consecutive identical captures. \
                         The page has not changed. STOP screenshotting and take a different \
                         action: `browser_click` something, `browser_type` into a field, \
                         `browser_navigate` to a new URL, or report the current state and \
                         end the task. Do NOT screenshot again until the page changes."
                    )));
                }
                return Ok(ToolResult::error(
                    "Page is identical to your last screenshot. The previous action \
                     (or no action) produced no visible change. Do not screenshot again — \
                     call `browser_find` with NO pattern to get an indexed inventory of \
                     every visible interactive element you can click/type, then act on one \
                     of those `[data-opencrabs-match=\"N\"]` selectors. Or `browser_navigate` \
                     to a new URL."
                        .to_string(),
                ));
            }
            self.manager
                .set_last_screenshot_hash(context.session_id, hash)
                .await;
            self.manager
                .reset_identical_screenshot_count(context.session_id)
                .await;
        }

        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        Ok(ToolResult::success(super::events::append_line(
            format!("data:image/png;base64,{b64}"),
            self.manager.drain_recent_events(),
        )))
    }
}
