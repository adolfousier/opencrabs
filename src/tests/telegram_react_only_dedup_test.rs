//! Telegram delivery: a dedup-suppressed final is not a react-only turn (#1623).
//!
//! Extracted from an inline `#[cfg(test)] mod tests` in `delivery.rs` per the
//! #1612 convention. The suppressed-final assertion is rewritten: the original
//! rebuilt the predicate inside the test body from literals, so it stayed green
//! with the production guard reverted. These call
//! [`is_react_only_turn`] itself, which is the expression the delivery path
//! evaluates, so dropping the guard turns them red.

use crate::channels::telegram::delivery::{
    is_react_only, is_react_only_turn, strip_echoed_plan_title,
};

#[test]
fn react_only_is_decided_by_emptiness_of_the_remaining_text() {
    assert!(is_react_only(""));
    assert!(is_react_only("   \n\t  "));
    assert!(!is_react_only("Done, fixed!"));
    assert!(!is_react_only("  some text  "));
}

#[test]
fn a_dedup_suppressed_final_is_not_react_only() {
    // Dedup already shipped the body as intermediate bubbles, so text_only is
    // empty by the time delivery looks. That is delivery, not a react-only
    // turn: treating it as one fires the early return and the #546 notice on
    // top of work the user already has.
    assert!(
        !is_react_only_turn(true, ""),
        "a suppressed final must never be classified react-only"
    );
    assert!(
        !is_react_only_turn(true, "   \n  "),
        "whitespace-only is the same case once suppressed"
    );
}

#[test]
fn a_genuine_react_only_turn_still_classifies() {
    assert!(
        is_react_only_turn(false, ""),
        "empty text that was not suppressed is the real react-only turn"
    );
    assert!(is_react_only_turn(false, "  \t "));
}

#[test]
fn text_bearing_turns_are_never_react_only_either_way() {
    assert!(!is_react_only_turn(false, "Done, fixed!"));
    assert!(!is_react_only_turn(true, "Done, fixed!"));
}

#[test]
fn echoed_plan_title_is_stripped_from_the_body() {
    assert_eq!(
        strip_echoed_plan_title("📋 Plan: \"Fix Bug\"\n\nHere is the fix", "Fix Bug"),
        "Here is the fix"
    );
    assert_eq!(
        strip_echoed_plan_title("No heading\nJust text", "Fix Bug"),
        "No heading\nJust text"
    );
}
