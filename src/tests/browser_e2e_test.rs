//! Browser end-to-end integration tests — ALL `#[ignore]`'d so they
//! never run in CI by default. Launches a real headless Chrome and
//! exercises the full CDP path against `https://example.com`.
//!
//! Run manually — SERIALLY. Every test builds its own `BrowserManager`
//! and they all share one Chrome profile directory, so a parallel run
//! makes them fight over `SingletonLock` and most fail to launch at all
//! (with an empty error, which reads like a code bug and is not one):
//!
//!     cargo test --features browser --lib browser_e2e -- --ignored --test-threads=1
//!
//! Each test pins a specific bug fix from the 2026-05-07 browser
//! resilience pass (commits 2d09065e, 7f58c6f9, 85f5a73b, and the
//! browser_close addition). They're marked `#[ignore]` because:
//!   * Chrome launches add ~2-5s per test
//!   * They need network access (example.com)
//!   * They depend on a Chrome/Chromium binary being installed
//!   * CI runners would need a separate browser-test job to avoid
//!     flake noise in the main suite.

#![cfg(feature = "browser")]

use crate::brain::tools::browser::{
    BrowserClickTool, BrowserCloseTool, BrowserContentTool, BrowserEvalTool, BrowserFindTool,
    BrowserManager, BrowserNavigateTool, BrowserScreenshotTool, BrowserTypeTool,
};
use crate::brain::tools::{Tool, ToolExecutionContext};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

const TEST_URL: &str = "https://example.com";

/// Pins the bug-#1 fix (commit 2d09065e): after `browser_navigate`,
/// the auto-screenshot must be a NON-BLANK image. Pre-fix,
/// `wait_for_navigation()` returned on the CDP `load` event before
/// paint, so the screenshot captured a blank/half-rendered page.
#[tokio::test]
#[ignore = "launches real Chrome — opt-in via `cargo test -- --ignored browser_e2e`"]
async fn navigate_then_screenshot_is_not_blank() {
    let mgr = Arc::new(BrowserManager::new(Default::default()));
    let nav = BrowserNavigateTool::new(mgr.clone());
    let ctx = ToolExecutionContext::new(Uuid::new_v4());

    let res = nav
        .execute(serde_json::json!({ "url": TEST_URL }), &ctx)
        .await
        .expect("navigate tool must not panic");
    assert!(
        res.success,
        "navigate to example.com must succeed: {}",
        res.output
    );

    // Post-navigate: the auto-screenshot rides along in result.images.
    assert!(
        !res.images.is_empty(),
        "navigate must attach an auto-screenshot to the result"
    );
    // images is Vec<(media_type, base64_data)>; index .1 is the
    // base64-encoded PNG. Heuristic: a real screenshot of example.com
    // is at least 4 KB after PNG compression. A blank/single-color
    // capture from the pre-fix race would be well under 1 KB.
    let (_mime, b64) = &res.images[0];
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64.as_bytes())
        .expect("auto-screenshot must be valid base64");
    assert!(
        bytes.len() > 4096,
        "auto-screenshot is suspiciously small ({} bytes) — \
         likely captured a blank page (regression of the 2d09065e fix)",
        bytes.len()
    );

    // Cleanup so the Chrome process doesn't leak between tests.
    let close = BrowserCloseTool::new(mgr);
    let _ = close.execute(serde_json::json!({}), &ctx).await;
}

/// Pins the bug-#2 fix (commit 7f58c6f9): two concurrent screenshot
/// calls in the same session must complete within a reasonable
/// budget. Pre-fix, the manager mutex was held across the awaited
/// CDP screenshot call, so a second concurrent call queued behind
/// the first one's full network round-trip — and worse, any task
/// trying to acquire the same mutex during the screenshot deadlocked.
#[tokio::test]
#[ignore = "launches real Chrome — opt-in via `cargo test -- --ignored browser_e2e`"]
async fn concurrent_screenshots_do_not_deadlock() {
    let mgr = Arc::new(BrowserManager::new(Default::default()));
    let nav = BrowserNavigateTool::new(mgr.clone());
    let shot = Arc::new(BrowserScreenshotTool::new(mgr.clone()));
    let ctx = ToolExecutionContext::new(Uuid::new_v4());

    nav.execute(serde_json::json!({ "url": TEST_URL }), &ctx)
        .await
        .expect("seed navigate must succeed");

    // Fire two screenshot calls in parallel. They share the manager
    // and same session — a regression of the lock-held-across-await
    // bug would either deadlock or serialize, exceeding the timeout.
    let shot_a = shot.clone();
    let ctx_a = ctx.clone();
    let shot_b = shot.clone();
    let ctx_b = ctx.clone();

    let combined = tokio::time::timeout(
        Duration::from_secs(15),
        futures::future::join(
            async move { shot_a.execute(serde_json::json!({}), &ctx_a).await },
            async move { shot_b.execute(serde_json::json!({}), &ctx_b).await },
        ),
    )
    .await;

    let (a, b) = combined.expect(
        "concurrent screenshots took >15s — likely deadlocked behind \
         the manager mutex (regression of the 7f58c6f9 fix)",
    );
    assert!(a.unwrap().success);
    assert!(b.unwrap().success);

    let close = BrowserCloseTool::new(mgr);
    let _ = close.execute(serde_json::json!({}), &ctx).await;
}

/// Pins the bug-#4 fix (this commit): browser_close on an open
/// session removes the page so a subsequent action gets a fresh
/// page rather than reusing the stale one. We can't directly
/// observe "fresh vs stale" without invoking another navigate, but
/// we CAN observe that close-then-list reports the session is gone.
#[tokio::test]
#[ignore = "launches real Chrome — opt-in via `cargo test -- --ignored browser_e2e`"]
async fn close_actually_removes_session_page() {
    let mgr = Arc::new(BrowserManager::new(Default::default()));
    let nav = BrowserNavigateTool::new(mgr.clone());
    let close = BrowserCloseTool::new(mgr.clone());
    let ctx = ToolExecutionContext::new(Uuid::new_v4());

    nav.execute(serde_json::json!({ "url": TEST_URL }), &ctx)
        .await
        .expect("navigate must succeed");

    let key = BrowserManager::page_name_for_session(ctx.session_id);
    assert!(
        mgr.list_pages().await.contains(&key),
        "after navigate, session must have an open page"
    );

    let close_res = close.execute(serde_json::json!({}), &ctx).await.unwrap();
    assert!(
        close_res.success,
        "browser_close must succeed on an open session"
    );
    assert!(
        !mgr.list_pages().await.contains(&key),
        "after browser_close, the session's page must be gone from the manager"
    );

    // Idempotent: second close on the same session must still report success.
    let second = close.execute(serde_json::json!({}), &ctx).await.unwrap();
    assert!(
        second.success,
        "browser_close must be idempotent — second call should not error"
    );
}

/// Tests that when `cdp_endpoint` is configured, the browser manager
/// connects to an existing Chromium instance instead of launching a new one.
/// This is the fix for issue #189 — multiple profiles sharing a single
/// browser to save memory.
///
/// To run this test manually:
/// 1. Start a headless Chromium with CDP enabled:
///    chromium --remote-debugging-port=9222 --headless --no-sandbox
/// 2. Run: cargo test --features browser --lib browser_cdp_endpoint -- --ignored
#[tokio::test]
#[ignore = "requires external Chromium with CDP enabled on port 9222"]
async fn cdp_endpoint_connects_to_existing_browser() {
    use crate::config::BrowserConfig;

    // Configure the manager to connect to an existing CDP endpoint
    let config = BrowserConfig {
        cdp_endpoint: Some("ws://localhost:9222".to_string()),
    };
    let mgr = Arc::new(BrowserManager::new(config));
    let nav = BrowserNavigateTool::new(mgr.clone());
    let ctx = ToolExecutionContext::new(Uuid::new_v4());

    // Navigate should succeed by connecting to the existing browser
    let res = nav
        .execute(serde_json::json!({ "url": TEST_URL }), &ctx)
        .await
        .expect("navigate tool must not panic");
    assert!(
        res.success,
        "navigate must succeed when connecting to existing CDP endpoint: {}",
        res.output
    );

    // Cleanup
    let close = BrowserCloseTool::new(mgr);
    let _ = close.execute(serde_json::json!({}), &ctx).await;
}

// ---------- shadow DOM: enumeration + resolution round trip ----------

/// Fixture page for the shadow-DOM tests. Four hosts, each isolating one
/// behaviour the module has to get right:
///   a. `open-host`   — an open root holding a button and an input.
///   b. `nested-host` — an open root two levels deep.
///   c. `closed-host` — a CLOSED root: resolvable over CDP, never
///      enumerable from JS (`el.shadowRoot` is null for one by spec).
///   d. `overlay-host` — a shadow-inner button under a
///      `pointer-events:none` overlay, which is what proves the
///      `elementFromPoint` retarget fix: `document.elementFromPoint`
///      would return the HOST for it and the old hit-test would drop it
///      as occluded.
const SHADOW_FIXTURE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>shadow-fixture</title>
<style>
  open-host, nested-host, closed-host, overlay-host { display:block; height:60px; }
</style></head>
<body>
<div id="light"><button id="light-btn">Light Button</button></div>
<open-host></open-host>
<nested-host></nested-host>
<closed-host></closed-host>
<overlay-host></overlay-host>
<script>
customElements.define('open-host', class extends HTMLElement {
  connectedCallback() {
    const r = this.attachShadow({mode:'open'});
    r.innerHTML = '<button id="shadow-btn">Shadow Button</button>'
      + '<input id="shadow-input" type="text">';
    r.getElementById('shadow-btn').addEventListener('click',
      () => { window.__clicked = 'shadow-btn'; });
  }
});
customElements.define('inner-host', class extends HTMLElement {
  connectedCallback() {
    const r = this.attachShadow({mode:'open'});
    r.innerHTML = '<button id="deep-btn">Deep Button</button>';
  }
});
customElements.define('nested-host', class extends HTMLElement {
  connectedCallback() {
    this.attachShadow({mode:'open'}).innerHTML = '<div><inner-host></inner-host></div>';
  }
});
customElements.define('closed-host', class extends HTMLElement {
  connectedCallback() {
    const r = this.attachShadow({mode:'closed'});
    r.innerHTML = '<button id="closed-btn">Closed Button</button>';
    r.getElementById('closed-btn').addEventListener('click',
      () => { window.__clicked = 'closed-btn'; });
  }
});
customElements.define('overlay-host', class extends HTMLElement {
  connectedCallback() {
    this.attachShadow({mode:'open'}).innerHTML =
      '<div style="position:relative;width:160px;height:40px">'
      + '<button id="under-btn" style="position:absolute;inset:0">Under Overlay</button>'
      + '<div style="position:absolute;inset:0;background:rgba(0,0,0,.2);'
      + 'pointer-events:none"></div></div>';
  }
});
</script></body></html>"#;

/// Write the fixture to a temp file and return its `file://` URL plus
/// the guard that keeps it alive for the test's lifetime.
fn shadow_fixture_url() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir for shadow fixture");
    let path = dir.path().join("shadow.html");
    std::fs::write(&path, SHADOW_FIXTURE).expect("write shadow fixture");
    let url = format!("file://{}", path.display());
    (dir, url)
}

/// The enumeration half: `browser_find` must surface elements rendered
/// inside open shadow roots (including a root nested two levels deep),
/// mark them `[shadow]`, and NOT report the overlaid one as occluded.
///
/// Pre-fix, `document.querySelectorAll` never entered a shadow root, so
/// the inventory on this page returned exactly one element: the light
/// DOM button.
#[tokio::test]
#[ignore = "launches real Chrome — opt-in via `cargo test -- --ignored browser_e2e`"]
async fn inventory_surfaces_open_shadow_roots() {
    let mgr = Arc::new(BrowserManager::new(Default::default()));
    let nav = BrowserNavigateTool::new(mgr.clone());
    let find = BrowserFindTool::new(mgr.clone());
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    let (_dir, url) = shadow_fixture_url();

    nav.execute(serde_json::json!({ "url": url }), &ctx)
        .await
        .expect("navigate to the fixture must not panic");

    let res = find
        .execute(serde_json::json!({}), &ctx)
        .await
        .expect("browser_find must not panic");
    assert!(res.success, "inventory must succeed: {}", res.output);
    let out = res.output;

    assert!(out.contains("Light Button"), "light DOM regression: {out}");
    assert!(
        out.contains("Shadow Button"),
        "open shadow root must be enumerated: {out}"
    );
    assert!(
        out.contains("Deep Button"),
        "shadow root nested two levels deep must be enumerated: {out}"
    );
    assert!(
        out.contains("[shadow]"),
        "shadow-resident elements must be marked: {out}"
    );
    assert!(
        out.contains("Under Overlay"),
        "a shadow element under a pointer-events:none overlay must NOT be \
         reported occluded — that is the elementFromPoint retarget fix: {out}"
    );
    // A closed root is invisible to JS by spec, so it must never appear
    // in the inventory even though browser_click can still resolve it.
    assert!(
        !out.contains("Closed Button"),
        "closed shadow roots are documented as NOT enumerable: {out}"
    );

    let close = BrowserCloseTool::new(mgr);
    let _ = close.execute(serde_json::json!({}), &ctx).await;
}

/// The resolution half — the round trip that makes enumeration worth
/// anything. A `[data-opencrabs-match="N"]` selector handed back by
/// `browser_find` for a shadow-resident element must click and type.
/// Fixing enumeration alone would produce a selector that lists fine and
/// then fails, which is strictly worse than not listing it.
#[tokio::test]
#[ignore = "launches real Chrome — opt-in via `cargo test -- --ignored browser_e2e`"]
async fn found_shadow_selector_clicks_and_types() {
    let mgr = Arc::new(BrowserManager::new(Default::default()));
    let nav = BrowserNavigateTool::new(mgr.clone());
    let find = BrowserFindTool::new(mgr.clone());
    let click = BrowserClickTool::new(mgr.clone());
    let typer = BrowserTypeTool::new(mgr.clone());
    let eval = BrowserEvalTool::new(mgr.clone());
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    let (_dir, url) = shadow_fixture_url();

    nav.execute(serde_json::json!({ "url": url }), &ctx)
        .await
        .expect("navigate must not panic");

    // Resolve the shadow button through find, then click THAT selector.
    let found = find
        .execute(serde_json::json!({ "pattern": "#shadow-btn" }), &ctx)
        .await
        .expect("find must not panic");
    assert!(
        found.success && found.output.contains("Shadow Button"),
        "css search must pierce open roots: {}",
        found.output
    );
    let selector = extract_match_selector(&found.output)
        .expect("find must hand back an indexed selector for the shadow button");

    let clicked = click
        .execute(serde_json::json!({ "selector": selector }), &ctx)
        .await
        .expect("click must not panic");
    assert!(
        clicked.success,
        "a selector from browser_find must resolve in browser_click: {}",
        clicked.output
    );
    let seen = eval
        .execute(
            serde_json::json!({ "script": "window.__clicked || ''" }),
            &ctx,
        )
        .await
        .expect("eval must not panic");
    assert!(
        seen.output.contains("shadow-btn"),
        "the click must have reached the shadow button: {}",
        seen.output
    );

    // Typing into a shadow-resident input goes through __ocQueryOne.
    let typed = typer
        .execute(
            serde_json::json!({ "selector": "#shadow-input", "text": "hello-shadow" }),
            &ctx,
        )
        .await
        .expect("type must not panic");
    assert!(typed.success, "type into shadow input: {}", typed.output);
    let value = eval
        .execute(
            serde_json::json!({
                "script": "document.querySelector('open-host').shadowRoot\
                           .getElementById('shadow-input').value"
            }),
            &ctx,
        )
        .await
        .expect("eval must not panic");
    assert!(
        value.output.contains("hello-shadow"),
        "browser_type must have filled the shadow input: {}",
        value.output
    );

    let close = BrowserCloseTool::new(mgr);
    let _ = close.execute(serde_json::json!({}), &ctx).await;
}

/// CLOSED roots: `resolve_element`'s pierced fallback runs over
/// `DOM.getDocument { pierce: true }`, which CDP populates with closed
/// roots too — JS cannot see them at all. This test is the receipt for
/// that claim (plan step 8b); if it fails, the closed-root line comes
/// out of the docs rather than staying as an unverified promise.
#[tokio::test]
#[ignore = "launches real Chrome — opt-in via `cargo test -- --ignored browser_e2e`"]
async fn closed_shadow_root_resolves_over_cdp() {
    let mgr = Arc::new(BrowserManager::new(Default::default()));
    let nav = BrowserNavigateTool::new(mgr.clone());
    let click = BrowserClickTool::new(mgr.clone());
    let eval = BrowserEvalTool::new(mgr.clone());
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    let (_dir, url) = shadow_fixture_url();

    nav.execute(serde_json::json!({ "url": url }), &ctx)
        .await
        .expect("navigate must not panic");

    // No indexed selector exists for it — a closed root is not
    // enumerable — so the plain CSS selector is the only handle, and it
    // can only resolve through the CDP pierced fallback.
    let clicked = click
        .execute(serde_json::json!({ "selector": "#closed-btn" }), &ctx)
        .await
        .expect("click must not panic");
    assert!(
        clicked.success,
        "closed shadow root must resolve via find_elements_pierced: {}",
        clicked.output
    );
    let seen = eval
        .execute(
            serde_json::json!({ "script": "window.__clicked || ''" }),
            &ctx,
        )
        .await
        .expect("eval must not panic");
    assert!(
        seen.output.contains("closed-btn"),
        "the click must have reached the closed-root button: {}",
        seen.output
    );

    let close = BrowserCloseTool::new(mgr);
    let _ = close.execute(serde_json::json!({}), &ctx).await;
}

/// Plan step 8a: does `document.body.innerText` already include text
/// rendered inside a shadow root? It decides whether `text=` mode was
/// partly working by accident and whether `browser_content --text_only`
/// needs a change. Answered by the page, not by reasoning — the
/// assertion records whichever answer Chrome gives so a future change in
/// behaviour shows up as a failure here instead of a silent surprise.
#[tokio::test]
#[ignore = "launches real Chrome — opt-in via `cargo test -- --ignored browser_e2e`"]
async fn body_inner_text_includes_shadow_text() {
    let mgr = Arc::new(BrowserManager::new(Default::default()));
    let nav = BrowserNavigateTool::new(mgr.clone());
    let eval = BrowserEvalTool::new(mgr.clone());
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    let (_dir, url) = shadow_fixture_url();

    nav.execute(serde_json::json!({ "url": url }), &ctx)
        .await
        .expect("navigate must not panic");

    let res = eval
        .execute(
            serde_json::json!({
                "script": "String(document.body.innerText.includes('Shadow Button'))"
            }),
            &ctx,
        )
        .await
        .expect("eval must not panic");
    // MEASURED, not reasoned: Chrome returns FALSE here. The intuition
    // that `innerText` is defined over rendered text, so shadow content
    // would come through, is wrong — `innerText` is computed per node
    // tree and stops at a shadow boundary like everything else.
    //
    // Two consequences, both acted on:
    //   * `text=` search genuinely needed the composed walk. It was not
    //     partly working by accident.
    //   * full-page `browser_content --text_only` was blind to shadow
    //     text, so it now joins per-tree text across the composed tree.
    // If a future Chrome changes this, THIS assertion flips first.
    assert!(
        res.output.contains("false"),
        "document.body.innerText was measured NOT to include shadow-rendered \
         text; a `true` here means Chrome changed and browser_content's \
         full-page text path can be simplified. got: {}",
        res.output
    );

    // ...and therefore `browser_content --text_only` must NOT rely on it.
    // This is the assertion that proves the composed-tree join actually
    // recovers what plain innerText drops.
    let content = BrowserContentTool::new(mgr.clone());
    let text = content
        .execute(serde_json::json!({ "text_only": true }), &ctx)
        .await
        .expect("content must not panic");
    assert!(text.success, "content must succeed: {}", text.output);
    assert!(
        text.output.contains("Light Button"),
        "light DOM text regression: {}",
        text.output
    );
    assert!(
        text.output.contains("Shadow Button") && text.output.contains("Deep Button"),
        "full-page text_only must span the composed tree, including a root \
         nested two levels deep: {}",
        text.output
    );

    let close = BrowserCloseTool::new(mgr);
    let _ = close.execute(serde_json::json!({}), &ctx).await;
}

/// Pull the first `[data-opencrabs-match="N"]` selector out of a
/// `browser_find` report.
fn extract_match_selector(output: &str) -> Option<String> {
    let start = output.find("[data-opencrabs-match=\"")?;
    let rest = &output[start..];
    let end = rest.find("\"]")? + 2;
    Some(rest[..end].to_string())
}
