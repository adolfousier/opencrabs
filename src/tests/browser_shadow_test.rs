//! Tests for the composed-tree JS helpers in
//! `src/brain/tools/browser/shadow.rs`.
//!
//! We cannot run the JS here (that needs a real page / V8 — that is what
//! the `#[ignore]`d fixture in `browser_e2e_test.rs` is for), so these
//! pin the emitted source: every helper name the rest of the module
//! calls, the open-root guard, the iteration caps, and the cycle guard.
//! A rename that silently breaks a caller fails here instead of at
//! runtime on someone's page.

#![cfg(feature = "browser")]

use crate::brain::tools::browser::{MAX_ROOTS, MAX_WALK_NODES, deep_helpers_js, with_deep_helpers};

#[test]
fn preamble_defines_every_helper_the_module_calls() {
    let js = deep_helpers_js();
    for name in [
        "__ocRoots",
        "__ocQueryAll",
        "__ocQueryOne",
        "__ocWalk",
        "__ocComposedContains",
        "__ocHitTest",
        "__ocClearStamps",
        "__ocInShadow",
    ] {
        assert!(
            js.contains(&format!("const {name} =")),
            "preamble must define {name}"
        );
    }
}

#[test]
fn root_collection_only_enters_open_shadow_roots() {
    let js = deep_helpers_js();
    // `el.shadowRoot` is null for a CLOSED root by spec — that is the
    // open-root guard, and it is the reason closed roots are documented
    // as resolvable (CDP) but never enumerable (JS).
    assert!(js.contains("const sr = h.shadowRoot;"));
    assert!(js.contains("if (sr && !seen.has(sr))"));
}

#[test]
fn root_collection_is_cycle_guarded_and_capped() {
    let js = deep_helpers_js();
    // Cycle guard: a root is queued at most once.
    assert!(js.contains("const seen = new Set(roots);"));
    assert!(js.contains("seen.add(sr); roots.push(sr);"));
    // Cap: pathological pages cannot grow the queue without bound.
    assert!(js.contains(&format!("const __OC_MAX_ROOTS = {MAX_ROOTS};")));
    assert!(js.contains("if (roots.length >= __OC_MAX_ROOTS) break;"));
}

#[test]
fn walk_is_iteration_capped() {
    let js = deep_helpers_js();
    assert!(js.contains(&format!("const __OC_MAX_NODES = {MAX_WALK_NODES};")));
    assert!(js.contains("if (out.length >= __OC_MAX_NODES) return out;"));
}

#[test]
fn walk_covers_body_and_every_shadow_root() {
    let js = deep_helpers_js();
    // The document contributes `document.body`; every other root walks
    // itself, so nothing inside a shadow tree is skipped.
    assert!(js.contains("const base = r === document ? document.body : r;"));
    assert!(js.contains("createTreeWalker(base, NodeFilter.SHOW_ELEMENT)"));
}

#[test]
fn query_all_spans_roots_and_respects_limit() {
    let js = deep_helpers_js();
    assert!(js.contains("for (const r of __ocRoots())"));
    assert!(js.contains("r.querySelectorAll(sel)"));
    assert!(js.contains("if (out.length >= limit) return out;"));
}

#[test]
fn composed_contains_hops_out_through_the_host() {
    let js = deep_helpers_js();
    // `Node.contains` stops at a shadow boundary; climbing via the
    // root's `host` is what makes ancestry work across one.
    assert!(js.contains("n.getRootNode().host"));
    assert!(js.contains("guard++ < __OC_MAX_ROOTS"));
}

#[test]
fn hit_test_is_scoped_to_the_elements_own_root() {
    let js = deep_helpers_js();
    // The whole point: NOT `document.elementFromPoint`, which retargets
    // to the shadow host and would flag every pierced element occluded.
    assert!(js.contains("const root = el.getRootNode();"));
    assert!(js.contains("from.elementFromPoint(x, y)"));
    assert!(!js.contains("document.elementFromPoint"));
}

#[test]
fn stamp_cleanup_is_deep() {
    let js = deep_helpers_js();
    let clear = js
        .split("const __ocClearStamps")
        .nth(1)
        .expect("preamble defines __ocClearStamps");
    // Clearing only `document` would leave stamps rotting inside shadow
    // trees, so a stale `[data-opencrabs-match="3"]` could resolve to a
    // node from a previous page state.
    assert!(clear.contains("for (const r of __ocRoots())"));
    assert!(clear.contains("removeAttribute('data-opencrabs-match')"));
}

#[test]
fn helpers_declare_no_globals() {
    let js = deep_helpers_js();
    // Page pollution is detectable; every helper is a scoped `const`.
    assert!(!js.contains("window.__oc"));
    assert!(!js.contains("globalThis.__oc"));
}

#[test]
fn wrapper_puts_helpers_in_scope_before_the_body() {
    let js = with_deep_helpers("return __ocQueryOne('button');");
    assert!(js.starts_with("(() => {"));
    assert!(js.ends_with("})()"));
    let helpers_at = js.find("__ocQueryAll").expect("helpers are spliced in");
    let body_at = js
        .find("return __ocQueryOne('button');")
        .expect("body is spliced in");
    assert!(
        helpers_at < body_at,
        "helpers must be declared before the body that calls them"
    );
}

// ---------- source-scan sentinels for the CDP resolution path ----------
//
// `resolve_element` is async and needs a live `Page`, so its ordering
// cannot be exercised without a browser (the e2e fixture does that,
// `#[ignore]`d). What CAN be checked cheaply here is the property that
// actually breaks the round trip when someone regresses it: a selector
// path that goes straight to `page.find_element` again, or a fallback
// that stops trying plain first.

fn browser_src(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/brain/tools/browser")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn no_selector_path_bypasses_the_shared_resolver() {
    // `Page::find_element` issues `DOM.querySelector` rooted at the
    // document node, which does not cross a shadow boundary. Every
    // caller must go through `resolve_element` instead, or a selector
    // `browser_find` just handed back will enumerate and then fail to
    // click — strictly worse than never enumerating it.
    for file in ["click.rs", "wait.rs", "screenshot.rs", "act.rs"] {
        let src = browser_src(file);
        assert!(
            !src.contains("page.find_element("),
            "{file} calls page.find_element directly — use \
             manager::resolve_element so shadow roots stay reachable"
        );
        assert!(
            src.contains("resolve_element("),
            "{file} must resolve selectors through manager::resolve_element"
        );
    }
}

#[test]
fn resolver_tries_the_light_dom_before_piercing() {
    // Ordering is the whole no-regression argument: a page with no
    // shadow DOM must pay ZERO extra CDP round-trips, so the pierced
    // lookup may only run after the plain one has already failed.
    let src = browser_src("manager.rs");
    let body = src
        .split("pub(crate) async fn resolve_element")
        .nth(1)
        .expect("manager.rs defines resolve_element");
    let plain = body
        .find("page.find_element(selector)")
        .expect("resolver must try the plain lookup");
    let pierced = body
        .find("find_elements_pierced(selector)")
        .expect("resolver must fall back to a pierced lookup");
    assert!(
        plain < pierced,
        "plain lookup must come first — piercing on every call would add \
         a DOM.getDocument round-trip to pages that have no shadow DOM"
    );
    // `find_element_pierced` is `els.pop()` upstream — it returns the
    // LAST match where plain `find_element` returns the first. Taking
    // the first keeps both branches answering the same question.
    assert!(
        !body.contains("find_element_pierced("),
        "use find_elements_pierced + first match, not find_element_pierced \
         (which pops the LAST match and disagrees with find_element)"
    );
}

#[test]
fn full_page_content_is_deliberately_not_pierced() {
    // Non-goal, written down rather than silently skipped: the
    // no-selector `browser_content` path applies no output cap, so
    // serializing every shadow tree would be an output-sizing change
    // wearing a shadow-DOM costume.
    let src = browser_src("content.rs");
    assert!(src.contains("page.content().await"));
    assert!(
        !src.contains("outer_html_full"),
        "full-page content must stay uncapped-safe until an output cap exists"
    );
    assert!(
        src.contains("no output cap"),
        "the reason must stay documented at the call site"
    );
}
