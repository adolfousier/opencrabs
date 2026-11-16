//! The conversation never collapses to nothing (#2985).
//!
//! The window has no headless harness, so these checks read source.
//!
//! The bug they pin: the wrappers around the transcript declared `min_h_0()`,
//! which throws away the floor the transcript asks for. A flex item with a zero
//! basis takes none of the shrink and all of the loss, so when the plan card,
//! the queue strip, the suggested options and a tall draft together outgrew the
//! window, the conversation was handed zero height and clipped out. The region
//! then painted as the window background: a chat that goes completely black.
//! Nothing failed and nothing logged, because a layout that legally resolves to
//! zero is not an error, which is why the report arrived with no evidence
//! beside it. Both wrappers and the witness that watches them are pinned here.

const PANE_CHROME: &str = include_str!("../gui/pane_chrome.rs");
const PANE_VIEW: &str = include_str!("../gui/pane_view.rs");

/// The body of one function, from its definition to the brace that closes it
/// at the same indent.
///
/// The scans below have to look inside a single function: `min_h_0()` is right
/// in dozens of places in this tree and wrong in exactly the two wrappers this
/// module is about, so a file-wide search would be meaningless.
fn fn_body<'a>(src: &'a str, needle: &str) -> &'a str {
    let at = src
        .find(needle)
        .unwrap_or_else(|| panic!("{needle} not found"));
    let head = src
        .get(..at)
        .expect("the needle offset lands on a char boundary");
    let indent = at - head.rfind('\n').map(|n| n + 1).unwrap_or(0);
    let close = format!("\n{}}}", " ".repeat(indent));
    let tail = src
        .get(at..)
        .expect("the needle offset lands on a char boundary");
    tail.split(&close).next().unwrap_or(tail)
}

#[test]
fn the_measure_column_carries_the_floor_at_both_levels() {
    let body = fn_body(PANE_CHROME, "pub(crate) fn chat_measure_column");
    // The outer flex item and the inner column: the floor has to survive both
    // levels, because the inner one alone is still inside a parent that may
    // have been handed zero.
    assert_eq!(
        body.matches(".min_h(px(CONVERSATION_FLOOR))").count(),
        2,
        "the outer row and the inner column both need the conversation floor"
    );
    assert!(
        !body.contains(".min_h_0()"),
        "a zero floor discards the transcript's own min height (#2985)"
    );
}

/// The source with every space and newline removed.
///
/// A builder chain is one expression however the formatter wrapped it, so the
/// chain is compared without its whitespace. Comments lose their line breaks
/// too, which is why the markers below stay short enough that no comment in
/// these two files can accidentally spell one.
fn compact(src: &str) -> String {
    src.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn the_split_pane_wrapper_carries_the_floor_too() {
    let source = compact(PANE_VIEW);
    assert!(
        source.contains(".flex_1().min_w_0().min_h(px(CONVERSATION_FLOOR))"),
        "the leaf transcript wrapper keeps the conversation floor (#2985)"
    );
    assert!(
        !source.contains(".flex_1().min_w_0().min_h_0().child(body),"),
        "the zero floor on the leaf wrapper is what lost the whole pane (#2985)"
    );
}

/// #3003: the transcript wrapper clips at the pane's edge.
///
/// It was the only box in that leaf with no clip. Its neighbours both carry
/// one, so a transcript that outgrew the wrapper painted over the pane's
/// chrome and past the leaf's edge into the neighbour, which reads as a pane's
/// text overlapping itself while the pane is unfocused.
#[test]
fn the_split_pane_wrapper_clips_its_conversation() {
    let source = compact(PANE_VIEW);
    assert!(
        source.contains(".min_h(px(CONVERSATION_FLOOR)).overflow_hidden()"),
        "the leaf transcript wrapper has to clip what it cannot hold (#3003)"
    );
}

#[test]
fn the_floor_is_the_one_number_the_transcript_asks_for() {
    // Two floors that drift apart are worse than none: the wrapper would keep
    // a height the transcript already considers collapsed.
    assert!(
        PANE_CHROME
            .contains("pub(crate) const CONVERSATION_FLOOR: f32 = transcript::TRANSCRIPT_MIN_H;"),
        "the wrapper floor is the transcript's own floor, not a second number"
    );
}

#[test]
fn the_collapse_witness_watches_both_paths() {
    assert!(
        PANE_CHROME.contains("pub(crate) fn witness_conversation_height"),
        "the witness exists, so a black chat leaves a line behind"
    );
    let body = fn_body(PANE_CHROME, "pub(crate) fn chat_measure_column");
    assert!(
        body.contains("witness_conversation_height("),
        "the single pane reports its conversation height"
    );
    assert!(
        PANE_VIEW.contains(".on_children_prepainted(move |bounds, _window, _cx| {")
            && PANE_VIEW.contains("witness_conversation_height("),
        "the split pane reports its conversation height"
    );
}
