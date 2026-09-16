//! Tests for the Discord auto-thread title generator (`thread_title`).
//!
//! Extracted from an inline `#[cfg(test)]` block that landed with PR #1608;
//! project policy (CONTRIBUTING.md) requires all tests under `src/tests/`.

use crate::channels::discord::handler::thread_title;

#[test]
fn keeps_short_prompt_whole() {
    assert_eq!(
        thread_title("dame un status del sistema"),
        "🧵 dame un status del sistema"
    );
}

#[test]
fn drops_leading_bang_and_collapses_whitespace() {
    assert_eq!(
        thread_title("!  busca   la\núltima release"),
        "🧵 busca la última release"
    );
}

#[test]
fn cuts_on_word_boundary_with_ellipsis() {
    let long = "este prompt es largo a propósito para probar el corte en el borde de cada palabra sin romper ninguna";
    let t = thread_title(long);
    assert!(t.starts_with("🧵 "));
    assert!(t.ends_with('…'));
    assert!(t.chars().count() <= 67); // marker + space + 64 + ellipsis
    assert!(!t.contains("romper")); // cut happened before the overflow word
}

#[test]
fn hard_caps_single_giant_word() {
    let giant = "x".repeat(200);
    let t = thread_title(&giant);
    assert_eq!(t.chars().count(), 67);
}
