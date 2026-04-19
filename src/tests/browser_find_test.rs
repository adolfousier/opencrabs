//! Tests for the `browser_find` JS builder. Each test pins the shape
//! of the JS we send into the page — assigning a stable
//! `data-opencrabs-match` attribute and returning an array of
//! `{selector, text, tag, visible}` objects — so the selectors the
//! model passes back to browser_click are deterministic.
//!
//! We can't run the JS (that requires a real page / V8), so these
//! tests verify we emit the right script shape for each mode and
//! that user-supplied patterns are correctly escaped.

use crate::brain::tools::browser::build_find_js;

#[test]
fn css_mode_uses_query_selector_all() {
    let js = build_find_js("css", "button.primary", 20);
    assert!(js.contains(r#"querySelectorAll("button.primary")"#));
    assert!(js.contains("slice(0, 20)"));
}

#[test]
fn xpath_mode_uses_document_evaluate() {
    let js = build_find_js("xpath", "//button[@type='submit']", 5);
    assert!(js.contains("document.evaluate("));
    assert!(js.contains("XPathResult.ORDERED_NODE_SNAPSHOT_TYPE"));
    assert!(js.contains("//button[@type='submit']"));
    assert!(js.contains("i < 5"));
}

#[test]
fn text_mode_walks_dom_for_substring() {
    let js = build_find_js("text", "Sign in", 10);
    assert!(js.contains("createTreeWalker"));
    assert!(js.contains("SHOW_ELEMENT"));
    // Pattern gets lowercased server-side for case-insensitive match
    assert!(js.contains(r#""Sign in".toLowerCase()"#));
}

#[test]
fn aria_mode_uses_attribute_selector() {
    let js = build_find_js("aria", "Close dialog", 3);
    assert!(js.contains(r#"[aria-label*="Close dialog" i]"#));
}

#[test]
fn unknown_mode_defaults_to_css() {
    let js = build_find_js("nonsense", ".btn", 5);
    assert!(
        js.contains(r#"querySelectorAll(".btn")"#),
        "unknown mode must fall back to CSS selector path"
    );
}

#[test]
fn escapes_double_quotes_in_pattern() {
    // A pattern containing a double quote would otherwise close the JS
    // string literal early and inject arbitrary code into the page
    // context. This guard is a security boundary.
    let js = build_find_js("css", "div[data-foo=\"bar\"]", 5);
    // The inner double quotes must be backslash-escaped so the
    // outer querySelectorAll(" … ") stays balanced.
    assert!(js.contains(r#"div[data-foo=\"bar\"]"#));
    // And no raw unescaped `"bar"` substring sneaks through breaking
    // the outer literal.
    assert!(
        !js.contains(r#"querySelectorAll("div[data-foo="bar"]")"#),
        "unescaped inner double-quotes would break the JS string literal"
    );
}

#[test]
fn escapes_backslashes() {
    // Backslash is the JS escape introducer — leaving one raw lets the
    // user's pattern bend subsequent chars into escape sequences.
    let js = build_find_js("css", r"div\.class", 5);
    assert!(js.contains(r"div\\.class"));
}

#[test]
fn clears_previous_match_attributes_before_re_enumerating() {
    // Every call starts by stripping any `data-opencrabs-match`
    // attributes left over from the previous call. Without this,
    // stale indices from call N would coexist with fresh indices
    // from call N+1 on different elements and the returned selector
    // would be ambiguous.
    let js = build_find_js("css", ".btn", 5);
    assert!(js.contains("[data-opencrabs-match]"));
    assert!(js.contains("removeAttribute('data-opencrabs-match')"));
}

#[test]
fn returns_object_with_stable_selector_per_match() {
    // The payload shape the model sees must be:
    //   { selector, text, tag, visible }
    // Selector uses the attribute we just assigned so it's unique
    // and survives subsequent DOM churn (within the same turn).
    let js = build_find_js("css", "button", 5);
    assert!(js.contains(r#"selector: '[data-opencrabs-match="' + i + '"]'"#));
    assert!(js.contains("text:"));
    assert!(js.contains("tag:"));
    assert!(js.contains("visible:"));
}
