//! Tests for `web_search`: the DuckDuckGo User-Agent rotation pool (#525) and
//! the pure result parser / formatter. The retry loop itself is network I/O and
//! is exercised in the field; these pin the rotation intent and the parsing.

use crate::brain::tools::web_search::{DDG_USER_AGENTS, format_results, parse_lite_results};

#[test]
fn ua_pool_has_multiple_distinct_agents() {
    // Rotation needs more than one UA; collapsing back to a single hardcoded UA
    // would reintroduce the 403 failures this fixes (#525).
    assert!(DDG_USER_AGENTS.len() >= 3, "need a pool to rotate through");
    let mut seen = std::collections::HashSet::new();
    for ua in DDG_USER_AGENTS {
        assert!(ua.starts_with("Mozilla/5.0"), "realistic UA expected: {ua}");
        assert!(seen.insert(*ua), "UAs must be distinct: {ua}");
    }
}

#[test]
fn parse_lite_extracts_result_links() {
    let html = r#"
        <a class="result-link" href="https://example.com/a">First result</a>
        <a class="result-link" href="https://rust-lang.org">Rust</a>
        <a class="result-link" href="ftp://skip.me">not http</a>
        <a class="result-link" href="https://empty.com"></a>
    "#;
    let results = parse_lite_results(html, 5);
    assert_eq!(
        results.len(),
        2,
        "non-http and empty-title links are dropped"
    );
    assert_eq!(results[0].title, "First result");
    assert_eq!(results[0].url, "https://example.com/a");
    assert_eq!(results[1].url, "https://rust-lang.org");
}

#[test]
fn parse_lite_respects_max_results() {
    let html = r#"
        <a class="result-link" href="https://a.com">A</a>
        <a class="result-link" href="https://b.com">B</a>
        <a class="result-link" href="https://c.com">C</a>
    "#;
    assert_eq!(parse_lite_results(html, 2).len(), 2);
}

#[test]
fn format_results_renders_list_and_empty() {
    let results = parse_lite_results(
        r#"<a class="result-link" href="https://example.com">Ex</a>"#,
        5,
    );
    let out = format_results("rust async", &results);
    assert!(out.contains("Search results for: \"rust async\""));
    assert!(out.contains("1. Ex"));
    assert!(out.contains("https://example.com"));

    let empty = format_results("nope", &[]);
    assert!(empty.contains("No results found"));
}

// ---- URL dedup across engine sections (#1731) ----

use crate::brain::tools::web_search::dedup_sections;

/// Brave/Serper-style rendered section: header + numbered entries with
/// `URL:` lines (this is what both engines' outputs look like).
fn brave_style(query: &str, entries: &[(&str, &str)]) -> String {
    let mut out = format!("Search results for: \"{query}\"\n\n");
    for (i, (title, url)) in entries.iter().enumerate() {
        out.push_str(&format!("{}. {title}\n   URL: {url}\n\n", i + 1));
    }
    out
}

/// DuckDuckGo-style rendered section: entries with `🔗 ` URL lines.
fn ddg_style(query: &str, entries: &[(&str, &str)]) -> String {
    let mut out = format!("🔍 Search results for: \"{query}\"\n\n");
    for (i, (title, url)) in entries.iter().enumerate() {
        out.push_str(&format!("{}. {title}\n   🔗 {url}\n\n", i + 1));
    }
    out
}

#[test]
fn dedup_drops_cross_engine_duplicate_urls_keeping_first() {
    let ddg = ddg_style("rust", &[("Rust Home", "https://www.rust-lang.org")]);
    let brave = brave_style(
        "rust",
        &[
            ("Rust Home", "https://www.rust-lang.org/"), // dup of DDG (trailing slash)
            ("Rust Book", "https://doc.rust-lang.org/book/"),
        ],
    );
    let merged = dedup_sections(vec![ddg, brave]);
    assert!(merged.contains("Rust Book"), "unique entry must survive");
    // Exact dup URL appears once; the kept doc.rust-lang.org hit is a
    // DIFFERENT page, so counting the bare "rust-lang.org" substring would
    // collide across both (it did, in the first draft of this assertion).
    assert_eq!(
        merged.matches("https://www.rust-lang.org").count(),
        1,
        "duplicate URL must collapse to its first occurrence, got: {merged}"
    );
    assert!(
        merged.contains("doc.rust-lang.org"),
        "distinct page must survive"
    );
}

#[test]
fn dedup_is_case_insensitive_on_urls() {
    let a = brave_style("q", &[("Title", "https://Example.com/Page")]);
    let b = brave_style("q", &[("Clone", "https://example.com/page")]);
    let merged = dedup_sections(vec![a, b]);
    // Count case-insensitively: the surviving first occurrence keeps its
    // original casing (`Example.com`), the clone is gone.
    assert_eq!(
        merged.to_lowercase().matches("example.com").count(),
        1,
        "case-variant duplicate must collapse, got: {merged}"
    );
}

#[test]
fn dedup_keeps_all_unique_entries_in_engine_order() {
    let a = brave_style("q", &[("A1", "https://a.com/1"), ("A2", "https://a.com/2")]);
    let b = ddg_style("q", &[("B1", "https://b.com/1")]);
    let merged = dedup_sections(vec![a, b]);
    assert!(merged.contains("A1") && merged.contains("A2") && merged.contains("B1"));
    // Engine order preserved: A section fully before B section.
    let a_pos = merged.find("A1").unwrap();
    let b_pos = merged.find("B1").unwrap();
    assert!(a_pos < b_pos);
}

#[test]
fn dedup_drops_section_reduced_to_header_only() {
    // Distinct queries so the dropped section's header is identifiable:
    // same-query headers would collide with the surviving section's.
    let a = brave_style("first query", &[("Only", "https://only.com")]);
    let b = brave_style("dup query", &[("Only dup", "https://only.com")]);
    let merged = dedup_sections(vec![a, b]);
    assert!(
        !merged.contains("dup query"),
        "header-only section must be dropped whole, got: {merged}"
    );
    assert!(
        merged.contains("first query") && merged.contains("https://only.com"),
        "the surviving section stays intact, got: {merged}"
    );
}

#[test]
fn dedup_passes_through_lines_without_recognizable_urls() {
    let exa_like =
        "Exa results for neural search\n\n- Doc one explains everything\n- Doc two continues\n";
    let merged = dedup_sections(vec![exa_like.to_string()]);
    assert_eq!(merged, exa_like);
}

#[test]
fn dedup_empty_input_yields_empty_output() {
    assert_eq!(dedup_sections(vec![]), "");
}

#[test]
fn dedup_duplicate_within_one_section_also_dropped() {
    let a = brave_style(
        "q",
        &[
            ("First", "https://dup.com"),
            ("Second", "https://uniq.com"),
            ("Third dup", "https://dup.com/"),
        ],
    );
    let merged = dedup_sections(vec![a]);
    assert!(merged.contains("First") && merged.contains("Second"));
    assert!(!merged.contains("Third dup"));
}
