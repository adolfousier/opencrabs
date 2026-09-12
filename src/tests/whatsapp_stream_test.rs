//! Edit-in-place streaming decisions (#1408).
//!
//! The network side of `stream.rs` needs a live client, but the two choices
//! that decide what the user sees are pure: whether a chunk grows the tracked
//! message or starts a new one, and how a burst is merged into one payload.

use crate::channels::whatsapp::stream::{CHUNK_LIMIT, Delivery, coalesce, plan};

#[test]
fn first_chunk_of_a_turn_is_sent_not_edited() {
    assert_eq!(
        plan(None, "starting work", CHUNK_LIMIT),
        Delivery::Send("starting work".to_string()),
        "nothing is tracked yet, so there is no message to edit"
    );
}

#[test]
fn second_chunk_edits_the_tracked_message_to_the_full_text() {
    assert_eq!(
        plan(Some("step one"), "step two", CHUNK_LIMIT),
        Delivery::Edit("step one\n\nstep two".to_string()),
        "an edit replaces the whole message, so it carries the full text"
    );
}

#[test]
fn a_streamed_turn_keeps_every_earlier_chunk() {
    // The #1408 failure mode in reverse: chunks must accumulate, never
    // replace. Three chunks must leave all three visible.
    let mut body: Option<String> = None;
    for chunk in ["1. alpha", "2. beta", "3. gamma"] {
        match plan(body.as_deref(), chunk, CHUNK_LIMIT) {
            Delivery::Edit(full) | Delivery::Send(full) => body = Some(full),
        }
    }
    let body = body.expect("chunks delivered");
    assert!(body.contains("1. alpha"), "first item survived");
    assert!(body.contains("2. beta"), "middle item survived");
    assert!(body.contains("3. gamma"), "last item survived");
}

#[test]
fn a_chunk_that_would_overflow_starts_a_new_message() {
    let nearly_full = "x".repeat(CHUNK_LIMIT - 5);
    assert_eq!(
        plan(Some(&nearly_full), "overflowing tail", CHUNK_LIMIT),
        Delivery::Send("overflowing tail".to_string()),
        "appending past the ceiling must open a new message, not truncate"
    );
}

#[test]
fn a_chunk_that_exactly_fills_the_ceiling_still_edits() {
    // body + "\n\n" + chunk == limit exactly.
    let body = "a".repeat(10);
    let chunk = "b".repeat(CHUNK_LIMIT - 12);
    match plan(Some(&body), &chunk, CHUNK_LIMIT) {
        Delivery::Edit(full) => assert_eq!(full.chars().count(), CHUNK_LIMIT),
        Delivery::Send(_) => panic!("an exact fit is still an edit"),
    }
}

#[test]
fn the_ceiling_counts_characters_not_bytes() {
    // Each of these is 1 char and 4 bytes. A byte-based check would cut a
    // message the server would have accepted.
    let body = "🦀".repeat(10);
    let chunk = "🦀".repeat(20);
    assert!(
        matches!(plan(Some(&body), &chunk, CHUNK_LIMIT), Delivery::Edit(_)),
        "32 characters is nowhere near the ceiling, whatever the byte count"
    );
}

#[test]
fn chunks_are_trimmed_before_delivery() {
    assert_eq!(
        plan(None, "  padded  \n", CHUNK_LIMIT),
        Delivery::Send("padded".to_string())
    );
}

#[test]
fn coalesce_joins_a_burst_into_one_payload() {
    let batch = vec!["one".to_string(), "two".to_string(), "three".to_string()];
    assert_eq!(coalesce(&batch), "one\n\ntwo\n\nthree");
}

#[test]
fn coalesce_drops_blank_chunks() {
    let batch = vec!["one".to_string(), "   ".to_string(), "two".to_string()];
    assert_eq!(
        coalesce(&batch),
        "one\n\ntwo",
        "a whitespace-only chunk must not open a gap in the message"
    );
}

#[test]
fn coalesce_of_nothing_is_empty() {
    assert!(coalesce(&[]).is_empty());
    assert!(coalesce(&["".to_string(), "  ".to_string()]).is_empty());
}

#[test]
fn a_single_chunk_coalesces_to_itself() {
    assert_eq!(coalesce(&["only".to_string()]), "only");
}
