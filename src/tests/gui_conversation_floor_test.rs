//! The conversation keeps its floor and its measure (#2985, #2981).
//!
//! The window has no headless harness, so these checks read source.
//!
//! Two bugs are pinned here, both about the wrapper that used to sit between a
//! single pane and the transcript.
//!
//! The first: that wrapper declared `min_h_0()`, which throws away the floor
//! the transcript asks for. A flex item with a zero basis takes none of the
//! shrink and all of the loss, so when the plan card, the queue strip, the
//! suggested options and a tall draft together outgrew the window, the
//! conversation was handed zero height and clipped out. The region then painted
//! as the window background: a chat that goes completely black. Nothing failed
//! and nothing logged, because a layout that legally resolves to zero is not an
//! error.
//!
//! The second: the wrapper was also the one node a single pane had and a split
//! never did, and a single pane drew a black chat through it while the split
//! panes beside it were clean (#2981). It is gone now. The cap rides on the
//! transcript itself and the column centres it, so the surface keeps the tree
//! it is drawn in and still lands on the axis the chrome under it uses.

use crate::tests::gui_dashboard_source::DASHBOARD;

const PANE_CHROME: &str = include_str!("../gui/pane_chrome.rs");
const PANE_VIEW: &str = include_str!("../gui/pane_view.rs");

/// The body of one function, from its definition to the brace that closes it
/// at the same indent.
///
/// The scans below have to look inside a single function: `min_h_0()` is right
/// in dozens of places in this tree and wrong in exactly the wrapper this
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

/// The source with every space and newline removed.
///
/// A builder chain is one expression however the formatter wrapped it, so the
/// chain is compared without its whitespace. Comments lose their line breaks
/// too, which is why the markers below stay short enough that no comment in
/// these files can accidentally spell one.
fn compact(src: &str) -> String {
    src.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn the_single_pane_caps_the_surface_and_centres_it() {
    // #2981: the cap is the transcript's own width now, and the column that
    // holds it centres it. The surface is drawn in the tree it always had, so
    // there is no extra flex level for a single pane to lose its rows in.
    let flat = compact(DASHBOARD);
    assert!(
        flat.contains(
            "letprimary=ifcentred_chat{primary.w(px(measure)).into_any_element()}else{primary.into_any_element()};"
        ),
        "the capped surface takes the measure as its own width"
    );
    assert!(
        flat.contains(".when(centred_chat,|d|d.items_center())"),
        "the column centres the capped surface, or the chat reads left of its own chrome"
    );
}

#[test]
fn the_wrapper_that_blacked_the_single_pane_is_gone() {
    // #2981: a single pane drew a black chat through this wrapper while the
    // split panes beside it were clean, and the wrapper was the only element
    // that differed between them. A wrapper that comes back without a fix for
    // that is the same black screen again.
    assert!(
        !PANE_CHROME.contains("pub(crate) fn chat_measure_column"),
        "the single pane's measure wrapper is back"
    );
    assert!(
        !PANE_CHROME.contains("fn paint_witness"),
        "the wrapper's paint witness outlived the wrapper it watched"
    );
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
fn the_collapse_witness_still_watches_the_split_pane() {
    assert!(
        PANE_CHROME.contains("pub(crate) fn witness_conversation_height"),
        "the witness exists, so a black chat leaves a line behind"
    );
    let leaf = fn_body(PANE_VIEW, "pub(crate) fn leaf_box");
    assert!(
        leaf.contains("witness_conversation_height("),
        "the split pane reports its conversation height"
    );
    // The single pane needs no second witness: the transcript reads its own
    // box and its own rows every frame it paints, and names the frame in the
    // log when they disagree (#2981).
    assert!(
        !PANE_VIEW.contains("chat_measure_column"),
        "the split pane never took the measure wrapper, and still must not"
    );
}
