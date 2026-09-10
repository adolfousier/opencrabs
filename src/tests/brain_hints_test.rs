//! Brain hint selection.
//!
//! Moved out of `src/brain/hints.rs`: tests live under `src/tests/`,
//! never inline beside the logic they exercise (#1076).

use crate::brain::hints::*;
use crate::memory::MemoryResult;

fn result(path: &str, snippet: &str, rank: f64) -> MemoryResult {
    MemoryResult {
        path: path.to_string(),
        snippet: snippet.to_string(),
        rank,
        corpus: "",
    }
}

#[test]
fn empty_results_yield_none() {
    assert_eq!(format_hints(&[]), None);
}

#[test]
fn all_below_threshold_yield_none() {
    let results = vec![
        result("TOOLS.md", "telegram_send routing", 0.05),
        result("AGENTS.md", "github rules", 0.01),
    ];
    assert_eq!(format_hints(&results), None);
}

#[test]
fn relevant_results_yield_block() {
    let results = vec![result("TOOLS.md", "telegram_send is canonical", 0.9)];
    let block = format_hints(&results).expect("expected a hint block");
    assert!(block.contains("relevant brain notes"));
    assert!(block.contains("TOOLS.md"));
    assert!(block.contains("telegram_send is canonical"));
}

#[test]
fn low_rank_results_are_filtered() {
    let results = vec![
        result("TOOLS.md", "good match", 0.8),
        result("AGENTS.md", "noise", 0.02),
    ];
    let block = format_hints(&results).expect("expected a hint block");
    assert!(block.contains("TOOLS.md"));
    assert!(!block.contains("AGENTS.md"));
}

#[test]
fn caps_at_max_snippets() {
    let results = vec![
        result("TOOLS.md", "one", 0.9),
        result("AGENTS.md", "two", 0.8),
        result("CODE.md", "three", 0.7),
    ];
    let block = format_hints(&results).expect("expected a hint block");
    assert!(block.contains("TOOLS.md"));
    assert!(block.contains("AGENTS.md"));
    assert!(!block.contains("CODE.md"));
}

#[test]
fn truncates_long_snippets() {
    let long = "y".repeat(600);
    let results = vec![result("TOOLS.md", &long, 0.9)];
    let block = format_hints(&results).expect("expected a hint block");
    let ys: usize = block.chars().filter(|c| *c == 'y').count();
    assert_eq!(ys, SNIPPET_CHARS);
}

#[test]
fn caps_whole_block() {
    let results = vec![
        result("TOOLS.md", &"a".repeat(500), 0.9),
        result("AGENTS.md", &"b".repeat(500), 0.8),
    ];
    let block = format_hints(&results).expect("expected a hint block");
    // Whole-block cap: MAX_HINT_CHARS plus the trailing ellipsis.
    assert!(block.chars().count() <= MAX_HINT_CHARS + 1);
}

#[test]
fn uses_file_name_not_full_path() {
    let results = vec![result("/home/u/.opencrabs/TOOLS.md", "routing", 0.9)];
    let block = format_hints(&results).expect("expected a hint block");
    assert!(block.contains("TOOLS.md"));
    assert!(!block.contains("/home/u"));
}
