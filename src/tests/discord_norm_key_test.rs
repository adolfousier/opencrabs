//! Tests for the Discord final-response dedup key (`norm_key`).
//!
//! Extracted from an inline `#[cfg(test)]` block that landed with PR #1603;
//! project policy (CONTRIBUTING.md) requires all tests under `src/tests/`.

use crate::channels::discord::handler::norm_key;

#[test]
fn identical_bodies_match() {
    let body = "Listo, las dos cosas:\n\n1. Provider registry queda apagado\n2. Cron creado";
    assert_eq!(norm_key(body), norm_key(body));
}

#[test]
fn whitespace_only_differences_match() {
    let a = "Answer text\n\nwith paragraphs\n";
    let b = "Answer text with paragraphs";
    assert_eq!(norm_key(a), norm_key(b));
}

#[test]
fn different_bodies_do_not_match() {
    // Narration must NOT dedup against the final answer even if they
    // share some words — the whole point of the guard.
    let narration = "On it — checking the logs now";
    let answer = "On it — the logs say the daemon is healthy";
    assert_ne!(norm_key(narration), norm_key(answer));
}

#[test]
fn empty_and_whitespace_are_equivalent() {
    assert_eq!(norm_key(""), norm_key("   \n\t  "));
}
