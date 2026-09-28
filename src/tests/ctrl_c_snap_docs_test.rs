//! Structural pins for the Ctrl+C documentation drift (#1770, #1771).
//!
//! The real behavior lives in `src/tui/app/state.rs`: when the transcript is
//! scrolled up (`!auto_scroll`), the first Ctrl+C press snaps back to bottom
//! and clears the input — no quit, no quit hint. Both documented surfaces
//! (README shortcut table, `/help` dialog) described only clear-input/quit,
//! lying by omission. Source scans keep the docs welded to the behavior.

const README: &str = include_str!("../../README.md");

#[test]
fn readme_ctrl_c_row_documents_snap_to_bottom() {
    let row = README
        .lines()
        .find(|l| l.trim_start().starts_with("| `Ctrl+C` |"))
        .expect("README must keep a `Ctrl+C` row in the Keyboard Shortcuts table");
    assert!(
        row.contains("bottom") && row.contains("scrolled"),
        "#1770: the Ctrl+C row must document that a scrolled-up transcript \
         snaps to bottom on the first press, not only clear-input/quit. \
         Offending row: {row}"
    );
    assert!(
        row.contains("quit"),
        "#1770: the Ctrl+C row must keep the double-press quit behavior too. \
         Offending row: {row}"
    );
}

const HELP_SRC: &str = include_str!("../tui/render/help.rs");

#[test]
fn help_dialog_ctrl_c_row_documents_snap_and_fits_the_column() {
    let line = HELP_SRC
        .lines()
        .find(|l| l.contains("kv(\"Ctrl+C\""))
        .expect("help.rs must keep a Ctrl+C kv row in the GLOBAL section");
    assert!(
        line.contains("Snap bottom"),
        "#1771: the /help dialog must document that the first Ctrl+C press \
         snaps the scrolled-up transcript to bottom. Offending line: {line}"
    );
    // The dialog renders two 50% columns with no wrap, so a long description
    // silently clips at the column edge (#1771's proposed 50-char string
    // would have). The description lives in the second string literal.
    let literals: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    let desc = literals.get(1).copied().unwrap_or_default();
    assert!(
        !desc.is_empty() && desc.len() <= 34,
        "#1771: Ctrl+C help description is {} chars; keep it within the \
         no-wrap column budget (<= 34). Offending line: {line}",
        desc.len()
    );
}
