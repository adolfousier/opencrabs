//! Tests for the native Discord table renderer (`tables_to_discord`).
//!
//! Extracted from an inline `#[cfg(test)]` block that landed with PR #1608;
//! project policy (CONTRIBUTING.md) requires all tests under `src/tests/`.

use crate::channels::discord::table_convert::tables_to_discord;

const NARROW: &str = "| Item | Value |\n|---|---|\n| Disk | 29% |\n| RAM | 9.9G |";

#[test]
fn narrow_table_becomes_grid() {
    let out = tables_to_discord(NARROW);
    assert!(out.starts_with("```text"));
    assert!(out.contains("─"));
    assert!(out.contains("Disk"));
    assert!(!out.contains('|'));
}

#[test]
fn wide_table_becomes_key_value() {
    let wide = format!("| A | {b} |\n|---|---|\n| k | {b} |", b = "x".repeat(120));
    let out = tables_to_discord(&wide);
    assert!(out.starts_with("**k**"));
    assert!(out.contains("└"));
}

#[test]
fn fenced_table_untouched() {
    let src = format!("```\n{NARROW}\n```");
    assert_eq!(tables_to_discord(&src), src);
}

#[test]
fn plain_text_untouched() {
    assert_eq!(tables_to_discord("no pipes here"), "no pipes here");
}

#[test]
fn emphasis_stripped_from_cells() {
    let status = "| **Item** | **Valor** |\n|---|---|\n| **Modelo** | minimax / `MiniMax-M3` |\n| **Perfil** | fixtest |";
    let out = tables_to_discord(status);
    assert!(out.starts_with("```text"));
    assert!(
        !out.contains("**"),
        "grid must not carry bold source: {out}"
    );
    assert!(
        !out.replace("```", "").contains('`'),
        "grid must not carry ticks outside the fence: {out}"
    );
    assert!(out.contains("minimax / MiniMax-M3"));
}
