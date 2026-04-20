//! `browser_find` — find multiple matching elements and return their
//! stable selectors + text + tag + visibility so the agent can pick
//! one and call `browser_click` against a selector it KNOWS is unique.
//!
//! Previously the agent had to compose `browser_eval` with hand-rolled
//! JS (`Array.from(querySelectorAll...).map(...)`) then parse the
//! returned JSON, then hand back a selector to `browser_click` — with
//! three failure modes: JS syntax errors, non-unique selectors, and
//! stale-ref races between the eval and the click. This tool does the
//! enumeration server-side with a stable indexed selector so the
//! click that follows is deterministic.

use super::manager::BrowserManager;
use crate::brain::tools::error::Result;
use crate::brain::tools::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct BrowserFindTool {
    manager: Arc<BrowserManager>,
}

impl BrowserFindTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BrowserFindTool {
    fn name(&self) -> &str {
        "browser_find"
    }

    fn description(&self) -> &str {
        "Find elements on the current page matching a pattern. Returns a list of \
         matches with stable `selector` values that can be passed back to \
         `browser_click` / `browser_type` without ambiguity. Supports four modes: \
         `css` (CSS selectors, default), `xpath` (XPath expressions), `text` \
         (visible text substring, case-insensitive), `aria` (matches aria-label \
         substring)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The selector / xpath / text / aria-label to match"
                },
                "mode": {
                    "type": "string",
                    "enum": ["css", "xpath", "text", "aria"],
                    "default": "css"
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "minimum": 1,
                    "maximum": 200
                }
            },
            "required": ["pattern"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let pattern = match input["pattern"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return Ok(ToolResult::error("'pattern' is required".into())),
        };
        let mode = input["mode"].as_str().unwrap_or("css");
        let limit = input["limit"]
            .as_u64()
            .map(|l| l.clamp(1, 200) as usize)
            .unwrap_or(20);

        let page = match self
            .manager
            .get_or_create_session_page(context.session_id)
            .await
        {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        // All four modes run server-side JS that enumerates matches
        // and assigns each a `data-opencrabs-match` attribute so the
        // returned selector (`[data-opencrabs-match="N"]`) is stable
        // and unique for the next click/type turn. The attribute is
        // cleared first to avoid leaking state across calls.
        let enumerate_js = build_find_js(mode, &pattern, limit);
        let raw = match page.evaluate(enumerate_js.as_str()).await {
            Ok(r) => r.value().cloned().unwrap_or(Value::Null),
            Err(e) => return Ok(ToolResult::error(format!("browser_find failed: {e}"))),
        };

        let matches = raw.as_array().cloned().unwrap_or_default();
        if matches.is_empty() {
            return Ok(ToolResult::success(format!(
                "No elements matched {mode}:{pattern}"
            )));
        }

        let formatted = format_matches(&matches);
        Ok(ToolResult::success(format!(
            "Found {} match{} for {}:{pattern}\n\n{formatted}",
            matches.len(),
            if matches.len() == 1 { "" } else { "es" },
            mode
        )))
    }
}

/// Build the enumeration script for a given mode. Each script ends by
/// evaluating to an array of `{selector, text, tag, visible}` objects.
///
/// `pub(crate)` so tests can pin the generated JS shape.
pub(crate) fn build_find_js(mode: &str, pattern: &str, limit: usize) -> String {
    // JS-escape the pattern: double quotes only (single quotes are OK
    // inside a double-quoted JS string literal; backslash needs escaping).
    let escaped = pattern.replace('\\', "\\\\").replace('"', "\\\"");
    let walker = match mode {
        "xpath" => format!(
            r#"
            (() => {{
                const it = document.evaluate("{escaped}", document, null,
                    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
                const out = [];
                for (let i = 0; i < it.snapshotLength && i < {limit}; i++)
                    out.push(it.snapshotItem(i));
                return out;
            }})()
            "#
        ),
        "text" => format!(
            r#"
            (() => {{
                const needle = "{escaped}".toLowerCase();
                const walker = document.createTreeWalker(
                    document.body, NodeFilter.SHOW_ELEMENT);
                const out = [];
                let node;
                while ((node = walker.nextNode()) && out.length < {limit}) {{
                    const t = (node.innerText || node.textContent || "").toLowerCase();
                    if (t.includes(needle)) out.push(node);
                }}
                return out;
            }})()
            "#
        ),
        "aria" => format!(
            r#"
            (() => Array.from(
                document.querySelectorAll(
                    '[aria-label*="{escaped}" i]'))
                .slice(0, {limit}))()
            "#
        ),
        _ => format!(
            // CSS default
            r#"
            (() => Array.from(
                document.querySelectorAll("{escaped}"))
                .slice(0, {limit}))()
            "#
        ),
    };

    // Wrap with the "assign stable data-opencrabs-match and serialise"
    // step, shared across all modes.
    format!(
        r#"
        (() => {{
            document.querySelectorAll('[data-opencrabs-match]').forEach(
                el => el.removeAttribute('data-opencrabs-match'));
            const nodes = {walker};
            const out = [];
            for (let i = 0; i < nodes.length; i++) {{
                const el = nodes[i];
                if (!el || !(el instanceof Element)) continue;
                el.setAttribute('data-opencrabs-match', String(i));
                const rect = el.getBoundingClientRect();
                const visible = rect.width > 0 && rect.height > 0
                    && getComputedStyle(el).visibility !== 'hidden'
                    && getComputedStyle(el).display !== 'none';
                out.push({{
                    selector: '[data-opencrabs-match="' + i + '"]',
                    text: (el.innerText || el.textContent || '').trim().slice(0, 200),
                    tag: el.tagName.toLowerCase(),
                    visible: visible,
                }});
            }}
            return out;
        }})()
        "#
    )
}

fn format_matches(matches: &[Value]) -> String {
    let mut out = String::new();
    for (i, m) in matches.iter().enumerate() {
        let sel = m["selector"].as_str().unwrap_or("");
        let tag = m["tag"].as_str().unwrap_or("");
        let text = m["text"].as_str().unwrap_or("");
        let vis = m["visible"].as_bool().unwrap_or(false);
        out.push_str(&format!(
            "  {i}. <{tag}>{vis_marker} {sel}\n     text: {text}\n",
            vis_marker = if vis { "" } else { " (hidden)" }
        ));
    }
    out
}
