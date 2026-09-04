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

use crate::memory::COLLECTION_EXTERNAL;
use crate::memory::db::Store;
use crate::memory::search::{MIN_STRUCTURAL_RESULTS, search_symbol_graph};
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
            .insert_call_edge(&caller, "target", file, (n - i) as i64)
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
    let results = search_symbol_graph(&store, "calls", "target", 5).unwrap();
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
    let results = search_symbol_graph(&store, "calls", "target", 50).unwrap();
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
    let store = Store::open(temp.path().join("callees.db")).unwrap();
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
    let store = Store::open(temp.path().join("symbols.db")).unwrap();
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
    let results = search_symbol_graph(&store, "calls", "target", 5).unwrap();
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
