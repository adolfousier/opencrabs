//! `browser_find` — find matching elements OR inventory all interactive
//! elements on the current page, returning each with a stable
//! `data-opencrabs-match` selector + text + tag + visibility so the
//! agent can pick one and call `browser_click` against a selector it
//! KNOWS is unique.
//!
//! Two modes share one serialization path:
//! - **Search** (`pattern` supplied): enum matches for css / xpath /
//!   text / aria, as before.
//! - **Inventory** (`pattern` omitted): enumerate ALL visible
//!   interactive elements (button, a[href], input, select, textarea,
//!   [role=button/link/...], [tabindex], summary, contenteditable).
//!   This exists so the agent has a text handle to click the moment it
//!   lands on a page, instead of screenshotting to discover what is
//!   clickable — the screenshot-discovery pattern that produced the
//!   40 "Page is identical" loop failures on Aug 9 alone (#1022).
//!
//! Previously the agent had to compose `browser_eval` with hand-rolled
//! JS (`Array.from(querySelectorAll...).map(...)`) then parse the
//! returned JSON, then hand back a selector to `browser_click` — with
//! three failure modes: JS syntax errors, non-unique selectors, and
//! stale-ref races between the eval and the click. This tool does the
//! enumeration server-side with a stable indexed selector so the
//! click that follows is deterministic.

use super::manager::BrowserManager;
use super::shadow::deep_helpers_js;
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
        "Find elements on the current page. With a `pattern`, returns matching \
         elements (modes: `css` default, `xpath`, `text` substring, `aria`). \
         WITHOUT a `pattern`, returns an inventory of ALL visible interactive \
         elements on the page (buttons, links, inputs, etc.), each with a \
         stable indexed selector ready for `browser_click`. Use the no-pattern \
         inventory when you have just landed and do not yet know what to \
         click — prefer it over `browser_screenshot` for discovery. \
         `css`, `text`, `aria` and the inventory all search INSIDE open shadow \
         roots (marked `[shadow]` in the output); `xpath` cannot cross a shadow \
         boundary — that is an XPath spec limit, not ours, so use `css`/`text` \
         for web-component pages."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Optional. Omit to inventory ALL visible interactive \
                                    elements on the page. With a value, matches that \
                                    selector / xpath / text / aria-label."
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
            }
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        // `pattern` is now optional: absent (or empty) means "inventory all
        // visible interactive elements". A non-empty value keeps the original
        // css/xpath/text/aria search behavior.
        let pattern = input["pattern"].as_str().filter(|p| !p.is_empty());
        let mode = input["mode"].as_str().unwrap_or("css");
        // Inventory benefits from a larger default cap (more elements are
        // interesting when you have no pattern), search keeps the old 20.
        let limit = input["limit"]
            .as_u64()
            .map(|l| l.clamp(1, 200) as usize)
            .unwrap_or(if pattern.is_some() { 20 } else { 50 });

        let page = match self
            .manager
            .get_or_create_session_page(context.session_id)
            .await
        {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(format!("Browser error: {e}"))),
        };

        // All enumeration runs server-side JS that collects a node array,
        // then assigns each a `data-opencrabs-match` attribute so the
        // returned selector (`[data-opencrabs-match="N"]`) is stable
        // and unique for the next click/type turn. The attribute is
        // cleared first to avoid leaking state across calls. Both the
        // search and inventory scripts share that serialization step
        // (`wrap_with_index`); only the node-collection expression differs.
        let enumerate_js = match pattern {
            Some(p) => build_find_js(mode, p, limit),
            None => build_inventory_js(limit),
        };
        let label = match pattern {
            Some(p) => format!("{mode}:{p}"),
            None => "interactive inventory".to_string(),
        };
        let raw = match page.evaluate(enumerate_js.as_str()).await {
            Ok(r) => r.value().cloned().unwrap_or(Value::Null),
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "browser_find failed ({label}): {e}"
                )));
            }
        };

        // Inventory mode wraps its result as {items, collapsed, occluded};
        // search mode carries the same shape with both counters at 0
        // (#1191, #1187).
        let collapsed = usize::try_from(raw["collapsed"].as_u64().unwrap_or(0)).unwrap_or(0);
        let occluded = usize::try_from(raw["occluded"].as_u64().unwrap_or(0)).unwrap_or(0);
        let matches = raw["items"].as_array().cloned().unwrap_or_default();
        if matches.is_empty() {
            return Ok(ToolResult::success(super::events::append_line(
                match pattern {
                    Some(p) => format!("No elements matched {mode}:{p}"),
                    None => "No visible interactive elements found on this page.".to_string(),
                },
                self.manager.drain_recent_events(),
            )));
        }

        let formatted = format_matches(&matches);
        let count = matches.len();

        // OOPIF inventory pass (#1190): cross-origin iframes are
        // site-isolated; the enumeration above only saw the main frame.
        // Run the same JS per frame page and namespace every element's
        // index with the frame label (`f2:14`), so the model can both
        // see frame membership and click/type through the prefix.
        let mut frame_sections: Vec<String> = Vec::new();
        let mut frame_total = 0usize;
        match self.manager.oopif_pages(context.session_id).await {
            Ok(frames) if !frames.is_empty() => {
                for (label, url, fpage) in frames {
                    let fraw = match fpage.evaluate(enumerate_js.as_str()).await {
                        Ok(r) => r.value().cloned().unwrap_or(Value::Null),
                        Err(e) => {
                            frame_sections.push(format!("[{label} {url}] inventory failed: {e}"));
                            continue;
                        }
                    };
                    let fitems = fraw["items"].as_array().cloned().unwrap_or_default();
                    if fitems.is_empty() {
                        continue;
                    }
                    // Namespace each selector: `[data-opencrabs-match="3"]`
                    // → `f2:[data-opencrabs-match="3"]`.
                    let namespaced: Vec<Value> = fitems
                        .into_iter()
                        .map(|mut it| {
                            if let Some(s) = it["selector"].as_str() {
                                it["selector"] = Value::String(format!("{label}:{s}"));
                            }
                            it
                        })
                        .collect();
                    frame_total += namespaced.len();
                    let fformatted = format_matches(&namespaced);
                    frame_sections.push(format!("[{label} {url}]\n{fformatted}"));
                }
            }
            _ => {}
        }
        let frames_note = if frame_sections.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nCross-origin frames ({frame_total} element{}):\n{}",
                if frame_total == 1 { "" } else { "s" },
                frame_sections.join("\n\n")
            )
        };

        Ok(ToolResult::success(super::events::append_line(
            match pattern {
                Some(p) => format!(
                    "Found {count} match{} for {mode}:{p}\n\n{formatted}{frames_note}",
                    if count == 1 { "" } else { "es" },
                ),
                None => {
                    let body = format!(
                        "{}\n\n{formatted}{frames_note}",
                        inventory_header(count, collapsed, occluded),
                    );
                    // If we hit the cap there may be more elements we did not
                    // show. Tell the model explicitly so it does not assume the
                    // list is exhaustive (mirrors the read_file truncation note).
                    if count >= limit {
                        format!(
                            "{body}\n\n(Inventory capped at {limit} visible elements. \
                             Narrow with a `pattern`/`mode` to see beyond this list.)"
                        )
                    } else {
                        body
                    }
                }
            },
            self.manager.drain_recent_events(),
        )))
    }
}

/// Wrap a node-collection expression (an IIFE returning an array of
/// Elements) with the shared "clear stale match attributes, stamp a
/// stable per-index `data-opencrabs-match`, serialize to
/// `{selector, text, tag, visible}`" step. Both `build_find_js` and
/// `build_inventory_js` route through here so the selectors the model
/// passes back to `browser_click` are deterministic and identical in
/// shape regardless of how the nodes were collected.
fn wrap_with_index(nodes_expr: &str) -> String {
    // The composed-tree helpers are spliced into THIS IIFE's scope, so
    // the nested node-collection IIFE below closes over them and every
    // mode gets shadow-DOM reach without a second wrapper.
    let helpers = deep_helpers_js();
    format!(
        r#"
        (() => {{{helpers}
            // Deep clear: stamping inside a shadow root while clearing
            // only `document` would leave stale stamps behind, so a
            // `[data-opencrabs-match="3"]` handed back on the next turn
            // could resolve to a node from a previous page state.
            __ocClearStamps();
            const nodes = {nodes_expr};
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
                    // Tells the model the element lives behind a shadow
                    // boundary. Additive: the selector shape is unchanged
                    // and still resolves through browser_click/type/act.
                    shadow: __ocInShadow(el),
                }});
            }}
            // Inventory mode attaches `collapsed` to the nodes array (a
            // JS-array property survives in-process, unlike through JSON);
            // search mode leaves it undefined → 0. Surfaced so the header
            // can say how many nested duplicates were folded (#1191).
            return {{
                items: out,
                collapsed: typeof nodes.collapsed === 'number' ? nodes.collapsed : 0,
                occluded: typeof nodes.occluded === 'number' ? nodes.occluded : 0,
            }};
        }})()
        "#
    )
}

/// Build the enumeration script for a given search mode. Each script ends by
/// evaluating to an array of Elements, which `wrap_with_index` then indexes
/// and serializes.
///
/// `pub(crate)` so tests can pin the generated JS shape.
pub(crate) fn build_find_js(mode: &str, pattern: &str, limit: usize) -> String {
    // JS-escape the pattern: double quotes only (single quotes are OK
    // inside a double-quoted JS string literal; backslash needs escaping).
    let escaped = pattern.replace('\\', "\\\\").replace('"', "\\\"");
    let walker = match mode {
        // XPath stays main-document only: the spec has no notion of a
        // shadow boundary, so `document.evaluate` cannot cross one no
        // matter how we call it. The tool description points at
        // css/text/aria for web-component pages.
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
                const out = [];
                for (const node of __ocWalk()) {{
                    if (out.length >= {limit}) break;
                    const t = (node.innerText || node.textContent || "").toLowerCase();
                    if (t.includes(needle)) out.push(node);
                }}
                return out;
            }})()
            "#
        ),
        "aria" => format!(
            r#"
            (() => __ocQueryAll('[aria-label*="{escaped}" i]', {limit}))()
            "#
        ),
        _ => format!(
            // CSS default
            r#"
            (() => __ocQueryAll("{escaped}", {limit}))()
            "#
        ),
    };

    wrap_with_index(&walker)
}

/// Build an inventory script that enumerates every VISIBLE interactive
/// element on the page (up to `limit`), indexed and serialized exactly
/// like a search result. Used when the agent has no `pattern` — the
/// "I just landed, what can I click?" case that otherwise drives the
/// screenshot-discovery loop (#1022).
///
/// The selector union is a fixed string (no user input), so it needs no
/// escaping. We pre-filter to visible elements inside the collection so
/// the index is not wasted on off-screen / `display:none` nodes.
///
/// `pub(crate)` so tests can pin the generated JS shape.
pub(crate) fn build_inventory_js(limit: usize) -> String {
    let nodes_expr = format!(
        r#"(() => {{
            const sel = 'a[href], button, input:not([type="hidden"]), select, \
textarea, summary, [role="button"], [role="link"], [role="checkbox"], \
[role="tab"], [role="menuitem"], [role="option"], [contenteditable=""], \
[contenteditable="true"], [tabindex]:not([tabindex="-1"])';
            // Deep scan: the union is matched in the document AND in
            // every open shadow root. Capped by the walk budget rather
            // than by `limit`, because the visibility/dup/occlusion
            // filters below reject candidates — capping the scan at
            // `limit` would silently under-fill the inventory.
            const all = __ocQueryAll(sel, __OC_MAX_NODES);
            const visible = [];
            const accepted = [];
            let collapsed = 0;
            let occluded = 0;
            for (const el of all) {{
                if (visible.length >= {limit}) break;
                const rect = el.getBoundingClientRect();
                if (rect.width > 0 && rect.height > 0
                    && getComputedStyle(el).visibility !== 'hidden'
                    && getComputedStyle(el).display !== 'none') {{
                    // Own-semantics elements always keep their index even
                    // when visually nested: a checkbox inside a label, a
                    // tab inside a tablist — each is its own click target.
                    const role = el.getAttribute('role');
                    const own = el.tagName === 'INPUT'
                        || el.tagName === 'SELECT'
                        || el.tagName === 'TEXTAREA'
                        || el.tagName === 'SUMMARY'
                        || el.isContentEditable === true
                        || role === 'checkbox' || role === 'menuitem'
                        || role === 'tab' || role === 'option';
                    if (!own) {{
                        // Collapsible duplicate: a generic click wrapper
                        // (a/button/role=button/link/tabindex span) fully
                        // contained (±1px tolerance) in an already-accepted
                        // element's rect is the SAME visual target — index
                        // the container only (#1191).
                        // Rect containment alone is not enough once we
                        // pierce: a shadow-inner control can sit inside
                        // an unrelated light-DOM wrapper's rect without
                        // being its descendant, and collapsing it would
                        // drop the only handle on it. Require COMPOSED
                        // ancestry so the fold still fires for a real
                        // nested wrapper (including one that is its own
                        // shadow host, where the host IS the click
                        // target) and never for a coincidental overlap.
                        const dup = accepted.some(a =>
                            a.rect.left - 1 <= rect.left
                            && rect.right <= a.rect.right + 1
                            && a.rect.top - 1 <= rect.top
                            && rect.bottom <= a.rect.bottom + 1
                            && __ocComposedContains(a.el, el));
                        if (dup) {{ collapsed++; continue; }}
                    }}
                    // Occlusion v1 (#1187): hit-test the rect center —
                    // when something opaque covers the candidate, the
                    // element returned is neither the candidate nor one
                    // of its descendants. pointer-events:none overlays
                    // pass through elementFromPoint by construction, so
                    // underlying content stays indexed. Centers clamped
                    // into the viewport; fully off-screen candidates
                    // (clamped center still outside) skip the test
                    // rather than being dropped on a technicality.
                    const vw = document.documentElement.clientWidth;
                    const vh = document.documentElement.clientHeight;
                    const cx = Math.min(Math.max(rect.left + rect.width / 2, 0), vw - 1);
                    const cy = Math.min(Math.max(rect.top + rect.height / 2, 0), vh - 1);
                    const inViewport = rect.right >= 0 && rect.bottom >= 0
                        && rect.left <= vw && rect.top <= vh;
                    if (inViewport) {{
                        // `document.elementFromPoint` retargets to the
                        // shadow host, and `Node.contains` stops at the
                        // boundary — together they would report EVERY
                        // pierced element as occluded. Hit-test inside
                        // the element's own root and compare ancestry
                        // along the composed tree instead.
                        const hit = __ocHitTest(el, cx, cy);
                        if (hit && hit !== el && !__ocComposedContains(el, hit)) {{
                            occluded++; continue;
                        }}
                    }}
                    visible.push(el);
                    accepted.push({{el: el, rect: rect}});
                }}
            }}
            visible.collapsed = collapsed;
            visible.occluded = occluded;
            return visible;
        }})()"#
    );
    wrap_with_index(&nodes_expr)
}

/// Inventory header line (#1191, #1187): count, optional collapse note
/// when nested duplicates were folded, optional occlusion note when
/// candidates were hidden under opaque overlays, and the selector
/// handoff instruction. Pure so tests can pin the wording without a
/// live page.
pub(crate) fn inventory_header(count: usize, collapsed: usize, occluded: usize) -> String {
    let mut s = format!(
        "{count} visible interactive element{} on this page",
        if count == 1 { "" } else { "s" },
    );
    if collapsed > 0 {
        s.push_str(&format!(
            " ({collapsed} nested duplicate{} collapsed)",
            if collapsed == 1 { "" } else { "s" },
        ));
    }
    if occluded > 0 {
        s.push_str(&format!(", {occluded} hidden (occluded)",));
    }
    s.push_str(
        " (indexed — pass the `[data-opencrabs-match=\"N\"]` \
         selector to `browser_click`):",
    );
    s
}

fn format_matches(matches: &[Value]) -> String {
    let mut out = String::new();
    for (i, m) in matches.iter().enumerate() {
        let sel = m["selector"].as_str().unwrap_or("");
        let tag = m["tag"].as_str().unwrap_or("");
        let text = m["text"].as_str().unwrap_or("");
        let vis = m["visible"].as_bool().unwrap_or(false);
        // `[shadow]` tells the model the element lives behind a shadow
        // boundary, so it can reason about why a hand-written CSS
        // selector for it would not resolve while the indexed one does.
        let shadow = m["shadow"].as_bool().unwrap_or(false);
        out.push_str(&format!(
            "  {i}. <{tag}>{vis_marker}{shadow_marker} {sel}\n     text: {text}\n",
            vis_marker = if vis { "" } else { " (hidden)" },
            shadow_marker = if shadow { " [shadow]" } else { "" }
        ));
    }
    out
}
