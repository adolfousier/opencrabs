//! memory_search external-tier parity pins (#89).
//!
//! Structural graph listings used to truncate at the schema-default n=5 in
//! arbitrary (unsorted) order, scope=all merged corpora unlabeled, and
//! external hits echoed absolute paths. These tests pin the fixes:
//! deterministic ordering in the query layer, the structural-only n floor,
//! and the symmetric corpus-tag contract.
//!
//! Graph-tier throughout — the whole file is gated on `code-graph` like the
//! query layer it exercises.

#![cfg(feature = "code-graph")]

use crate::brain::tools::memory_search::MemorySearchTool;
use crate::brain::tools::r#trait::Tool;
use crate::memory::COLLECTION_EXTERNAL;
use crate::memory::db::Store;
use crate::memory::search::{MIN_STRUCTURAL_RESULTS, detect_structural_query, search_symbol_graph};
use tempfile::TempDir;

/// 8 distinct callers of one symbol, inserted in scrambled order.
fn store_with_n_callers(n: usize) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("parity.db");
    let store = Store::open(&db_path).unwrap();
    store.ensure_symbol_tables().unwrap();
    // Scrambled: names reverse-ordered, lines shuffled, two files.
    for i in 0..n {
        let caller = format!("caller_{:02}", n - i);
        let file = if i % 2 == 0 { "b.rs" } else { "a.rs" };
        store
            .insert_call_edge(&caller, "target", file, n - i)
            .unwrap();
    }
    (temp, store)
}

/// Structural listings must NOT truncate at the schema-default n=5 (#89):
/// every hit is a rank-1.0 one-liner, so the full set IS the answer. The
/// effective cap floors at MIN_STRUCTURAL_RESULTS (25).
#[test]
fn structural_listing_returns_full_set_above_search_default() {
    let (_temp, store) = store_with_n_callers(8);
    let results = search_symbol_graph(&store, "calls", "target", 5, 0).unwrap();
    assert_eq!(
        results.len(),
        8,
        "n=5 must not truncate a structural listing; floor is {MIN} (got {})",
        results.len(),
        MIN = MIN_STRUCTURAL_RESULTS
    );
}

/// The floor is a floor, not a hard cap: results beyond 25 still return when
/// the caller asks for more.
#[test]
fn structural_floor_does_not_cap_higher_requests() {
    let (_temp, store) = store_with_n_callers(8);
    let results = search_symbol_graph(&store, "calls", "target", 50, 0).unwrap();
    assert_eq!(results.len(), 8, "a request above the floor must not clamp");
}

/// Caller listings come back deterministically ordered by
/// (caller_symbol, file_path, call_line) — db-layer ORDER BY (#89).
#[test]
fn caller_listings_are_deterministically_ordered() {
    let (_temp, store) = store_with_n_callers(6);
    let callers = store.query_callers_of("target").unwrap();

    let mut expected: Vec<(String, String, usize)> = callers.clone();
    expected.sort();
    assert_eq!(
        callers, expected,
        "query_callers_of must return rows sorted by (caller, file, line)"
    );

    // And the order is stable across repeated queries.
    let again = store.query_callers_of("target").unwrap();
    assert_eq!(callers, again, "repeated queries must not reshuffle");
}

/// Callee listings get the same deterministic treatment.
#[test]
fn callee_listings_are_deterministically_ordered() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(&temp.path().join("callees.db")).unwrap();
    store.ensure_symbol_tables().unwrap();
    for (callee, file, line) in [
        ("zeta", "a.rs", 3),
        ("alpha", "b.rs", 1),
        ("alpha", "a.rs", 9),
        ("alpha", "a.rs", 2),
    ] {
        store
            .insert_call_edge("caller_fn", callee, file, line)
            .unwrap();
    }
    let callees = store.query_callees_of("caller_fn").unwrap();
    let got: Vec<(String, String, usize)> = callees;
    assert_eq!(
        got,
        vec![
            ("alpha".into(), "a.rs".into(), 2),
            ("alpha".into(), "a.rs".into(), 9),
            ("alpha".into(), "b.rs".into(), 1),
            ("zeta".into(), "a.rs".into(), 3),
        ]
    );
}

/// Symbol definitions keep tests-last ordering and are now fully
/// deterministic (file_path, start_line tiebreakers) (#89).
#[test]
fn symbol_definitions_are_deterministically_ordered() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(&temp.path().join("symbols.db")).unwrap();
    store.ensure_symbol_tables().unwrap();
    for (file, line) in [
        ("src/lib.rs", 20),
        ("src/lib.rs", 5),
        ("tests/it.rs", 1),
        ("src/main.rs", 2),
    ] {
        store
            .insert_symbol("Widget", "struct", file, line, line + 1)
            .unwrap();
    }
    let defs = store.query_symbols_by_name("Widget").unwrap();
    let files: Vec<&str> = defs.iter().map(|(_, f, _, _)| f.as_str()).collect();
    assert_eq!(
        files,
        vec!["src/lib.rs", "src/main.rs", "src/lib.rs", "tests/it.rs"],
        "expected non-test files first sorted by path/line, tests last — got {files:?}"
    );
}

/// Structural results carry the external corpus so scope=all can tag them (#89).
#[test]
fn structural_results_stamp_external_corpus() {
    let (_temp, store) = store_with_n_callers(2);
    let results = search_symbol_graph(&store, "calls", "target", 5, 0).unwrap();
    assert!(!results.is_empty());
    assert!(
        results.iter().all(|r| r.corpus == COLLECTION_EXTERNAL),
        "structural hits belong to the external corpus"
    );
}

/// The tag law is corpus-symmetric (#89): in scope=all, brain and memory hits
/// get tagged exactly like external ones. A named scope stays untagged, and
/// a result with no corpus (collection-wide search()) stays untagged too.
#[test]
fn corpus_tags_are_symmetric_across_corpora() {
    let tag = crate::brain::tools::memory_search::corpus_tag;
    assert_eq!(tag("brain", true), "[brain] ");
    assert_eq!(tag("memory", true), "[memory] ");
    assert_eq!(tag("external", true), "[external] ");
    // Named scope: no provenance ambiguity, no tag.
    assert_eq!(tag("brain", false), "");
    assert_eq!(tag("external", false), "");
    // Collection-wide search() stamps an empty corpus.
    assert_eq!(tag("", true), "");
    assert_eq!(tag("", false), "");
}

/// "impact of X" routes to the impact chain; plain "who calls X" must NOT.
#[test]
fn impact_queries_route_to_the_impact_chain() {
    assert_eq!(
        detect_structural_query("impact of validate_input"),
        Some(("impact".into(), "validate_input".into()))
    );
    assert_eq!(
        detect_structural_query("what breaks if I change process_message"),
        Some(("impact".into(), "process_message".into()))
    );
    assert_eq!(
        detect_structural_query("who calls validate_input transitively"),
        Some(("impact".into(), "validate_input".into()))
    );
    // Plain depth-0 queries stay where they were.
    assert_eq!(
        detect_structural_query("who calls validate_input"),
        Some(("calls".into(), "validate_input".into()))
    );
}

/// Chain A→B→C: impact of C reaches A at depth 2, each caller reported once
/// at its shallowest depth, output ordered by (depth, caller).
#[test]
fn impact_chain_walks_two_hops() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(&temp.path().join("impact.db")).unwrap();
    store.ensure_symbol_tables().unwrap();
    // a.rs line numbers scrambled to prove ordering, not insertion order.
    store.insert_call_edge("mid", "leaf", "a.rs", 42).unwrap();
    store.insert_call_edge("top", "mid", "a.rs", 7).unwrap();
    store.insert_call_edge("mid2", "leaf", "b.rs", 2).unwrap();

    let rows = store.query_transitive_callers("leaf", 2).unwrap();
    assert_eq!(
        rows,
        vec![
            ("mid".into(), "a.rs".into(), 42, 1),
            ("mid2".into(), "b.rs".into(), 2, 1),
            ("top".into(), "a.rs".into(), 7, 2),
        ],
        "depth-1 callers first, then depth-2, each ordered by (file, line)"
    );

    // Depth 1 answers direct callers only.
    let direct = store.query_transitive_callers("leaf", 1).unwrap();
    assert_eq!(direct.len(), 2, "depth cap must exclude depth-2 callers");
}

/// CYCLE GUARD: corrupt data with A→B→A must terminate at the depth cap,
/// not walk forever or fan out — the query returns bounded, deduplicated
/// rows (the guard exists precisely for this case) (#89).
#[test]
fn impact_chain_survives_a_call_cycle() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(&temp.path().join("cycle.db")).unwrap();
    store.ensure_symbol_tables().unwrap();
    store.insert_call_edge("b_fn", "a_fn", "a.rs", 1).unwrap();
    store.insert_call_edge("a_fn", "b_fn", "a.rs", 2).unwrap();

    let rows = store.query_transitive_callers("a_fn", 2).unwrap();
    // b_fn (depth 1) then a_fn itself (depth 2, via the cycle). Bounded rows,
    // no duplication blowup, no hang.
    assert!(
        rows.len() <= 2,
        "cycle must not fan out beyond one row per symbol per depth: {rows:?}"
    );
    assert!(
        rows.iter().any(|(c, _, _, d)| c == "b_fn" && *d == 1),
        "direct caller b_fn must be present"
    );
}

/// The rendered impact chain indents by depth.
#[test]
fn impact_snippets_indent_by_depth() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(&temp.path().join("indent.db")).unwrap();
    store.ensure_symbol_tables().unwrap();
    store.insert_call_edge("mid", "leaf", "a.rs", 42).unwrap();
    store.insert_call_edge("top", "mid", "a.rs", 7).unwrap();

    let results = search_symbol_graph(&store, "impact", "leaf", 5, 0).unwrap();
    let snippets: Vec<&str> = results.iter().map(|r| r.snippet.as_str()).collect();
    assert_eq!(
        snippets,
        vec![
            "mid calls leaf at line 42 (depth 1)",
            "  top calls leaf at line 7 (depth 2)",
        ]
    );
}

/// TRUNCATION VISIBILITY (#89): a 30-caller symbol listed with the floor n=25
/// must end with a marker counting the 5 hidden callers — never a silent cut.
#[test]
fn truncated_listing_reports_the_hidden_remainder() {
    let (_temp, store) = store_with_n_callers(30);
    let results = search_symbol_graph(&store, "calls", "target", 5, 0).unwrap();
    // 25 shown (floor) + 1 marker line.
    assert_eq!(results.len(), 26);
    let last = results.last().unwrap();
    assert!(
        last.path.is_empty(),
        "the marker rides an empty-path result"
    );
    assert_eq!(
        last.snippet,
        "… and 5 more callers of target (re-query with higher n or offset)"
    );
    // No marker when nothing was cut.
    let (_temp2, store2) = store_with_n_callers(8);
    let full = search_symbol_graph(&store2, "calls", "target", 5, 0).unwrap();
    assert_eq!(full.len(), 8, "a fully-shown listing gets no marker");
}

/// OFFSET PAGINATION (#89): offset=25 with n=25 returns the NEXT sorted page
/// — no overlap with page 1, in the same deterministic order.
#[test]
fn offset_returns_the_next_sorted_page_without_overlap() {
    let (_temp, store) = store_with_n_callers(30);
    let page1 = search_symbol_graph(&store, "calls", "target", 5, 0).unwrap();
    let page2 = search_symbol_graph(&store, "calls", "target", 5, 25).unwrap();

    // Page 1 = 25 hits + marker; page 2 = the remaining 5, no marker.
    assert_eq!(page1.len(), 26);
    assert_eq!(page2.len(), 5);
    assert!(
        page2.iter().all(|r| !r.path.is_empty()),
        "exhausted listing must not carry a marker"
    );

    let names1: Vec<String> = page1
        .iter()
        .filter(|r| !r.path.is_empty())
        .map(|r| r.snippet.split_whitespace().next().unwrap().to_string())
        .collect();
    let names2: Vec<String> = page2
        .iter()
        .map(|r| r.snippet.split_whitespace().next().unwrap().to_string())
        .collect();
    assert!(
        names1.iter().all(|n| !names2.contains(n)),
        "pages must not overlap: {names2:?} vs page 1"
    );

    // Page 2 continues the sorted order: its 5 callers are the tail of the
    // full sorted set.
    let all = store.query_callers_of("target").unwrap();
    let tail: Vec<String> = all[25..].iter().map(|(c, _, _)| c.clone()).collect();
    assert_eq!(names2, tail);
}

/// MEGA-SET ROLLUP (#89): above 500 callers the listing collapses to one line
/// per file with its call-site count, plus the total line.
#[test]
fn mega_caller_sets_roll_up_per_file() {
    let (_temp, store) = store_with_n_callers(501);
    let results = search_symbol_graph(&store, "calls", "target", 5, 0).unwrap();

    // Two fixture files (a.rs / b.rs) + the total line — tens of lines, not 501.
    assert_eq!(results.len(), 3);
    // Rollups sorted by file: a.rs first, then b.rs, then the marker.
    assert_eq!(results[0].path, "a.rs");
    assert_eq!(results[0].snippet, "250 call sites of target");
    assert_eq!(results[1].path, "b.rs");
    assert_eq!(results[1].snippet, "251 call sites of target");
    assert_eq!(
        results[2].snippet,
        "… 501 total callers of target (per-file rollup; target a file for detail)"
    );
}

/// The offset contract is declared structural-only: the schema documents
/// offset for structural listings, and ranked paths take no offset at all —
/// their signature (search_core) has no offset parameter to pass.
#[test]
fn offset_is_documented_as_structural_only() {
    let schema = crate::brain::tools::memory_search::MemorySearchTool.input_schema();
    let offset = &schema["properties"]["offset"];
    assert_eq!(offset["default"], 0, "offset defaults to 0");
    let desc = offset["description"].as_str().unwrap().to_lowercase();
    assert!(
        desc.contains("structural"),
        "offset description must scope it to structural listings: {desc}"
    );
    assert!(
        desc.contains("ignore") || desc.contains("not"),
        "offset description must say ranked paths ignore it: {desc}"
    );
}
