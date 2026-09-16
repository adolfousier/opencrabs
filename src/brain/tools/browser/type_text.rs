//! browser_type — Type text into an element or the focused element.

use super::manager::{BrowserManager, split_frame_selector};
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{
    Tool, ToolCapability, ToolExecutionContext, ToolHints, ToolResult,
};
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
        "Type text into an input found by CSS selector (or the focused element if no selector). \
         REPLACES the field's value (does not append) and dispatches input/change events so \
         framework-controlled forms (React/Vue/Svelte, e.g. NextAuth) update their internal \
         state — then a normal browser_click on the submit button works. Returns a screenshot \
         after typing."
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
        let text = match input["text"].as_str() {
            Some(t) if !t.is_empty() => t,
            _ => return Ok(ToolResult::error("'text' is required".into())),
        };
        let selector_input = input["selector"].as_str();

        let page = match self
            .manager
            .get_or_create_session_page(context.session_id)
            .await
        {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        // Frame-routed selectors (#1190): `f2:14` — resolve the owning
        // OOPIF page and strip the prefix (same contract as click).
        let (page, selector): (chromiumoxide::Page, Option<String>) =
            if let Some(sel) = selector_input {
                if let Some((label, rest)) = split_frame_selector(sel) {
                    match self
                        .manager
                        .oopif_page_by_label(context.session_id, &label)
                        .await
                    {
                        Ok(Some(p)) => (p, Some(rest)),
                        Ok(None) => {
                            return Ok(ToolResult::error(format!(
                                "Frame '{label}' not found. The frame may have navigated away — \
                                 re-run `browser_find` to get a fresh inventory."
                            )));
                        }
                        Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
                    }
                } else {
                    (page, Some(sel.to_string()))
                }
            } else {
                (page, None)
            };
        let selector = selector.as_deref();

        // JSON-encode both values so any quotes/backslashes/newlines in the
        // text or selector are injected into the script safely.
        let sel_js = selector
            .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "null".into()))
            .unwrap_or_else(|| "null".into());
        let val_js = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());

        // React/Vue/Svelte controlled inputs ignore a plain `el.value = x`
        // assignment — the framework overrides the value setter, so its
        // internal state never updates and a later submit sees an empty
        // field. We must: (1) REPLACE the value via the *native* setter so
        // the framework's value tracker registers the change, then (2)
        // dispatch bubbling `input` + `change` events so onChange fires.
        // `+=` (the old behaviour) also appended to placeholder/stale text,
        // producing the "placeholder + credentials" garbage. This replaces.
        // `__ocQueryOne` instead of `document.querySelector`: an input
        // rendered inside an open shadow root is invisible to the latter,
        // so a selector `browser_find` just handed back would resolve to
        // nothing here. Outcome strings are unchanged.
        let js = super::shadow::with_deep_helpers(&format!(
            r#"
  var sel = {sel_js};
  var val = {val_js};
  var el = sel ? __ocQueryOne(sel) : document.activeElement;
  if (!el) return "no_element";
  var editable = el.isContentEditable;
  if (el.value === undefined && !editable) return "not_input";
  try {{ el.focus(); }} catch (e) {{}}
  if (editable) {{
    el.textContent = val;
    el.dispatchEvent(new InputEvent("input", {{bubbles:true}}));
    el.dispatchEvent(new Event("change", {{bubbles:true}}));
    return "ok";
  }}
  var proto = el.tagName === "TEXTAREA"
    ? window.HTMLTextAreaElement.prototype
    : window.HTMLInputElement.prototype;
  var desc = Object.getOwnPropertyDescriptor(proto, "value");
  if (desc && desc.set) {{ desc.set.call(el, val); }} else {{ el.value = val; }}
  el.dispatchEvent(new Event("input", {{bubbles:true}}));
  el.dispatchEvent(new Event("change", {{bubbles:true}}));
  return "ok";"#
        ));

        let outcome = match page.evaluate(js.as_str()).await {
            Ok(r) => r
                .value()
                .and_then(|v: &serde_json::Value| v.as_str().map(String::from))
                .unwrap_or_default(),
            Err(e) => return Ok(ToolResult::error(format!("Typing failed: {e}"))),
        };

        let target = selector
            .map(|s| s.to_string())
            .unwrap_or_else(|| "focused element".into());
        match outcome.as_str() {
            "ok" => {
                // Reset consecutive-identical screenshot counter — typing is a page change
                self.manager
                    .reset_identical_screenshot_count(context.session_id)
                    .await;
                let mut result = ToolResult::success(super::events::append_line(
                    format!("Typed into {target}"),
                    self.manager.drain_recent_events(),
                ));
                // Auto-screenshot: give the model vision after typing.
                self.manager
                    .attach_screenshot(context.session_id, &mut result)
                    .await;
                Ok(result)
            }
            "no_element" => Ok(ToolResult::error(if selector.is_some() {
                format!("Element '{target}' not found")
            } else {
                "No focused element. Pass a 'selector' to target an input.".into()
            })),
            "not_input" => Ok(ToolResult::error(format!(
                "'{target}' is not an editable input/textarea. Pick the input element itself."
            ))),
            _ => Ok(ToolResult::error("Typing failed: unexpected result".into())),
        }
    }
}
