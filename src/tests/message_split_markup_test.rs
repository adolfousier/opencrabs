//! Chunk boundaries avoid splitting markup (#876).
//!
//! Chunks are sent as separate messages and Telegram parses each one
//! independently, so a boundary inside `<code>…</code>` or between a pair of
//! backticks leaves that chunk invalid on its own: an unclosed start tag is a
//! 400 and the chunk is lost, which reads as the reply simply stopping.
//!
//! `split_message` previously took the last newline near the limit with no
//! knowledge of markup. It now prefers a newline where nothing is open, and
//! falls back to the old choice when no such point exists in the window — so
//! the only case that moves is the one that was already broken.
//!
//! Fixtures are synthetic and carry no user identifiers.

use crate::channels::discord::handler::split_message;

/// Every chunk must stand alone: balanced backticks and balanced tags.
fn each_chunk_is_self_contained(chunks: &[String]) -> bool {
    chunks.iter().all(|c| {
        let ticks_balanced = c.matches('`').count().is_multiple_of(2);
        let opens = c.matches("<code>").count() + c.matches("<b>").count();
        let closes = c.matches("</code>").count() + c.matches("</b>").count();
        ticks_balanced && opens == closes
    })
}

#[test]
fn nothing_is_lost_when_splitting() {
    // The property that matters most: chunking must never drop characters.
    let text = (0..400)
        .map(|i| format!("line {i} with `code span {i}` in it"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = split_message(&text, 1000);
    assert_eq!(chunks.concat(), text, "chunking lost or altered characters");
}

#[test]
fn a_boundary_does_not_land_inside_a_code_span() {
    // Each line carries a backtick pair, so a naive break can land between
    // them. Every chunk must still be independently parseable.
    let text = (0..400)
        .map(|i| format!("line {i} with `code span {i}` in it"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = split_message(&text, 1000);
    assert!(chunks.len() > 1, "fixture must actually split");
    assert!(
        each_chunk_is_self_contained(&chunks),
        "a chunk ended with markup left open"
    );
}

#[test]
fn a_boundary_does_not_land_inside_an_html_tag_pair() {
    let text = (0..400)
        .map(|i| format!("row {i} <code>value {i}</code> trailing"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = split_message(&text, 1000);
    assert!(chunks.len() > 1, "fixture must actually split");
    assert!(
        each_chunk_is_self_contained(&chunks),
        "a chunk ended with an unclosed tag"
    );
    assert_eq!(chunks.concat(), text);
}

#[test]
fn plain_text_splitting_is_unchanged() {
    // No markup anywhere, so the new preference must not alter the result.
    let text = (0..400)
        .map(|i| format!("plain line number {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = split_message(&text, 1000);
    assert_eq!(chunks.concat(), text);
    for c in chunks.iter().take(chunks.len() - 1) {
        assert!(c.len() <= 1000, "a chunk exceeded the limit");
    }
}

#[test]
fn a_short_message_is_never_split() {
    let text = "one `span` and a <b>tag</b>";
    assert_eq!(split_message(text, 1000), vec![text]);
}

#[test]
fn no_chunk_exceeds_the_limit() {
    // The safety search may only move a break EARLIER, never past the cap.
    let text = (0..500)
        .map(|i| format!("line {i} `x` <b>y</b>"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = split_message(&text, 800);
    for c in &chunks {
        assert!(c.len() <= 800, "chunk of {} exceeded 800", c.len());
    }
}

#[test]
fn text_with_no_newlines_still_splits() {
    // Degenerate input: no newline to prefer, so it falls back to the hard
    // boundary rather than looping or returning nothing.
    let text = "x".repeat(2500);
    let chunks = split_message(&text, 1000);
    assert!(chunks.len() >= 3);
    assert_eq!(chunks.concat(), text);
}
