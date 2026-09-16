//! Tests for the Discord message splitter (`split_message`), focusing on
//! fence balance across chunk boundaries.
//!
//! Extracted from an inline `#[cfg(test)]` block that landed with PR #1608;
//! project policy (CONTRIBUTING.md) requires all tests under `src/tests/`.

use crate::channels::discord::handler::split_message;

#[test]
fn split_keeps_short_text_whole() {
    assert_eq!(split_message("hola", 2000), vec!["hola".to_string()]);
}

#[test]
fn split_fenced_grid_never_left_open() {
    // Regression: the fallback break used to land inside a ```text fence
    // (table grids are fenced), spilling raw rows into the next message
    // followed by a phantom opening fence.
    let mut text = String::from("Los 7 planes\n\n```text\n");
    for i in 0..40 {
        text.push_str(&format!(
            "plan-{i:0>3}.json          una descripcion cualquiera\n"
        ));
    }
    text.push_str("```\n\nultimo parrafo\n");
    let chunks = split_message(&text, 700);
    assert!(chunks.len() > 1, "expected the text to split");
    for chunk in &chunks {
        assert_eq!(
            chunk.matches('`').count() % 2,
            0,
            "unbalanced backticks in {chunk:?}"
        );
    }
}

#[test]
fn split_giant_fence_closed_and_reopened() {
    // A fence bigger than one chunk cannot travel whole; it must be
    // closed at the chunk edge and re-opened at the top of the next.
    let mut text = String::from("```text\n");
    for i in 0..60 {
        text.push_str(&format!("row-{i:0>3}          padding padding padding\n"));
    }
    text.push_str("```\n");
    let chunks = split_message(&text, 500);
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        assert_eq!(chunk.matches('`').count() % 2, 0, "unbalanced: {chunk:?}");
    }
    assert!(
        chunks[1].starts_with("```"),
        "reopen fence missing at chunk 2 start: {:?}",
        &chunks[1][..chunks[1].len().min(40)]
    );
}

#[test]
fn split_respects_hard_limit_even_with_reopen() {
    let mut text = String::from("```text\n");
    for i in 0..80 {
        text.push_str(&format!("row-{i:0>3}          x\n"));
    }
    text.push_str("```\n");
    for chunk in split_message(&text, 300) {
        assert!(
            chunk.len() <= 300,
            "chunk exceeds Discord limit: {} bytes",
            chunk.len()
        );
    }
}
