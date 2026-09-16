//! Tests for the `browser_find` inventory-mode JS builder
//! (`build_inventory_js`), used when the agent calls `browser_find`
//! with no `pattern` to enumerate every visible interactive element
//! on the page (#1022). We pin the shape of the JS we send into the
//! page so the selectors the model passes back to `browser_click` are
//! deterministic and identical in shape to the search-mode results.
//!
//! We can't run the JS (that requires a real page / V8), so these
//! tests verify we emit the right selector union, pre-filter to
//! visible elements, respect the limit, and reuse the shared
//! `{selector, text, tag, visible}` serialization.

#![cfg(feature = "browser")]

use crate::brain::tools::browser::{build_inventory_js, inventory_header};

#[test]
fn inventory_targets_interactive_element_union() {
    // The inventory must enumerate the standard "interactive" set, not
    // every element on the page. Each of these MUST appear so a click
    // target is never silently dropped.
    let js = build_inventory_js(50);
    assert!(js.contains("a[href]"), "links");
    assert!(js.contains("button"), "buttons");
    assert!(
        js.contains("input:not([type=\"hidden\"])"),
        "visible inputs"
    );
    assert!(js.contains("select"), "selects");
    assert!(js.contains("textarea"), "textareas");
    assert!(js.contains("summary"), "disclosure summaries");
    assert!(js.contains("[role=\"button\"]"), "ARIA buttons");
    assert!(js.contains("[role=\"link\"]"), "ARIA links");
    assert!(js.contains("[role=\"checkbox\"]"), "ARIA checkboxes");
    assert!(js.contains("[role=\"tab\"]"), "ARIA tabs");
    assert!(js.contains("[role=\"menuitem\"]"), "ARIA menu items");
    assert!(js.contains("[role=\"option\"]"), "ARIA options");
    assert!(js.contains("[contenteditable=\"true\"]"), "contenteditable");
    // tabindex excludes -1 (not focusable) but includes the bare attribute.
    assert!(js.contains("[tabindex]:not([tabindex=\"-1\"])"), "tabindex");
}

#[test]
fn inventory_pre_filters_to_visible_elements() {
    // Off-screen / hidden elements waste the index and can never be
    // clicked, so the inventory must drop them at collection time,
    // BEFORE indexing. Mirrors the visibility check in the shared
    // serializer but applied earlier to keep the index dense.
    let js = build_inventory_js(50);
    assert!(js.contains("getBoundingClientRect()"));
    assert!(js.contains("rect.width > 0"));
    assert!(js.contains("rect.height > 0"));
    assert!(js.contains("getComputedStyle(el).visibility !== 'hidden'"));
    assert!(js.contains("getComputedStyle(el).display !== 'none'"));
}

#[test]
fn inventory_respects_the_limit() {
    // The cap bounds the collection so a page with 800 buttons does not
    // flood context. The limit is enforced inside the collection loop,
    // so the index never exceeds it even though the serializer also
    // walks the full node array.
    let js = build_inventory_js(40);
    assert!(js.contains("visible.length >= 40"));
    assert!(js.contains("break"));
}

#[test]
fn inventory_uses_shared_match_index_serializer() {
    // Inventory and search results MUST be serialized identically so the
    // model sees one shape regardless of how the nodes were collected.
    // The shared `wrap_with_index` step clears stale attributes, stamps a
    // stable per-index `data-opencrabs-match`, and returns the same
    // `{selector, text, tag, visible}` tuple as search mode.
    let js = build_inventory_js(50);
    assert!(
        js.contains("removeAttribute('data-opencrabs-match')"),
        "must clear stale match attributes before re-indexing"
    );
    assert!(
        js.contains(r#"selector: '[data-opencrabs-match="' + i + '"]'"#),
        "must return the stable indexed selector shape"
    );
    assert!(js.contains("text:"));
    assert!(js.contains("tag:"));
    assert!(js.contains("visible:"));
}

#[test]
fn inventory_has_no_user_supplied_string_to_escape() {
    // The selector union is a fixed string with no user input, so unlike
    // search mode there is no injection surface. Sanity-check that the
    // union is a single static querySelectorAll argument and contains no
    // format placeholders from the caller.
    let js = build_inventory_js(10);
    // The limit is the only interpolation; the selector union is literal.
    assert!(
        !js.contains(r#""+ "#),
        "no concatenation into the selector string"
    );
    // The union goes through the deep helper, which matches it in the
    // document AND in every open shadow root.
    assert!(js.contains("__ocQueryAll(sel, __OC_MAX_NODES)"));
}

#[test]
fn inventory_scan_budget_is_not_the_display_limit() {
    // The visibility / dedup / occlusion filters reject candidates, so
    // capping the SCAN at `limit` would silently under-fill the
    // inventory. Scan is capped by the walk budget; `limit` still bounds
    // what is indexed.
    let js = build_inventory_js(5);
    assert!(js.contains("__ocQueryAll(sel, __OC_MAX_NODES)"));
    assert!(js.contains("visible.length >= 5"));
}

// ---------- #1191: nested-duplicate dedup + collapse count ----------

#[test]
fn inventory_dedups_generic_wrappers_contained_in_accepted_elements() {
    // A generic click candidate (a/button/role=button/link/tabindex span)
    // whose rect sits fully inside an ALREADY-accepted element's rect is
    // the same visual target — it must be skipped (collapsed), not given
    // its own index. The button+icon+span page yields ONE index for the
    // button, not three.
    let js = build_inventory_js(50);
    assert!(js.contains("const accepted = [];"), "containment tracker");
    assert!(
        js.contains("const dup = accepted.some(a =>"),
        "containment check against already-accepted elements"
    );
    // ±1px tolerance so sub-pixel layout rounding cannot split a real
    // wrapper from its container.
    assert!(js.contains("a.rect.left - 1 <= rect.left"), "left tolerance");
    assert!(js.contains("rect.right <= a.rect.right + 1"), "right tolerance");
    assert!(js.contains("a.rect.top - 1 <= rect.top"), "top tolerance");
    assert!(
        js.contains("rect.bottom <= a.rect.bottom + 1"),
        "bottom tolerance"
    );
    assert!(js.contains("collapsed++"), "collapse counter incremented");
}

#[test]
fn inventory_collapse_requires_composed_ancestry() {
    // Rect containment alone stops being sufficient once enumeration
    // pierces: a shadow-inner control can sit inside an unrelated
    // light-DOM wrapper's rect without being its descendant, and
    // collapsing it would throw away the only handle on it.
    let js = build_inventory_js(50);
    assert!(
        js.contains("&& __ocComposedContains(a.el, el));"),
        "collapse must require the accepted element to be a composed-tree ancestor"
    );
    assert!(js.contains("accepted.push({el: el, rect: rect});"));
}

#[test]
fn inventory_dedup_preserves_own_semantics_elements() {
    // The containment skip is gated on !own: elements with click semantics
    // of their own survive even when visually nested inside another accepted
    // element — a checkbox wrapped in a label keeps BOTH indices (the label
    // toggles, the checkbox toggles-and-selects; different actions).
    let js = build_inventory_js(50);
    // Own-semantics classification comes BEFORE the containment check.
    let own_pos = js
        .find("const own = el.tagName === 'INPUT'")
        .expect("own-semantics classification present");
    let dup_pos = js
        .find("const dup = accepted.some")
        .expect("containment check present");
    let gate_pos = js.find("if (!own)").expect("gate present");
    assert!(
        own_pos < gate_pos && gate_pos < dup_pos,
        "classification → gate → containment, in that order"
    );
    for token in [
        "'INPUT'",
        "'SELECT'",
        "'TEXTAREA'",
        "'SUMMARY'",
        "'checkbox'",
        "'menuitem'",
        "'tab'",
        "'option'",
        "isContentEditable",
    ] {
        assert!(js.contains(token), "own-semantics set includes {token}");
    }
}

#[test]
fn inventory_attaches_collapse_count_for_header() {
    // The collector stashes the collapsed count on the nodes array so the
    // shared serializer can surface it without a second DOM pass.
    let js = build_inventory_js(50);
    assert!(js.contains("visible.collapsed = collapsed"));
    assert!(js.contains("visible.occluded = occluded"));
    assert!(js.contains("typeof nodes.collapsed === 'number'"));
    assert!(js.contains("typeof nodes.occluded === 'number'"));
    // Occlusion v1 (#1187): center hit-test with viewport clamping,
    // descendant tolerance, and dropped candidates earning no index.
    // Hit-test is scoped to the element's OWN root: plain
    // `document.elementFromPoint` retargets to the shadow host and
    // `Node.contains` stops at the boundary, so together they would
    // report every pierced element as occluded and drop it.
    assert!(js.contains("__ocHitTest(el, cx, cy)"));
    assert!(js.contains("Math.min(Math.max(rect.left + rect.width / 2, 0), vw - 1)"));
    assert!(js.contains("hit !== el && !__ocComposedContains(el, hit)"));
    assert!(js.contains("occluded++; continue;"));
    assert!(js.contains("items: out"));
}

#[test]
fn inventory_header_mentions_collapsed_count_only_when_nonzero() {
    // Zero collapses: header identical to the pre-#1191 wording.
    let plain = inventory_header(12, 0, 0);
    assert!(plain.starts_with("12 visible interactive elements on this page"));
    assert!(!plain.contains("collapsed"), "no collapse note at zero");
    assert!(!plain.contains("occluded"), "no occlusion note at zero");

    // Non-zero: count appears in the header, singular and plural forms.
    let multi = inventory_header(9, 3, 0);
    assert!(multi.contains("(3 nested duplicates collapsed)"), "{multi}");
    let single = inventory_header(5, 1, 0);
    assert!(
        single.contains("(1 nested duplicate collapsed)"),
        "{single}"
    );

    // Selector handoff instruction survives in all variants.
    assert!(plain.contains("[data-opencrabs-match=\"N\"]"));
    assert!(multi.contains("[data-opencrabs-match=\"N\"]"));
}

#[test]
fn inventory_header_mentions_occluded_count_only_when_nonzero() {
    // Occlusion alone.
    let occluded_only = inventory_header(7, 0, 2);
    assert!(
        occluded_only.contains(", 2 hidden (occluded)"),
        "{occluded_only}"
    );
    assert!(!occluded_only.contains("collapsed"));

    // Both counters: collapse note then occlusion note, comma-joined.
    let both = inventory_header(6, 1, 4);
    assert!(both.contains("(1 nested duplicate collapsed)"), "{both}");
    assert!(both.contains(", 4 hidden (occluded)"), "{both}");

    // Singular occluded wording is the same as plural (count carries it).
    let one = inventory_header(3, 0, 1);
    assert!(one.contains(", 1 hidden (occluded)"), "{one}");
}
