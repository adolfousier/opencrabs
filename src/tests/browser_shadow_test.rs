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
