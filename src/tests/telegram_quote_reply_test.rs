//! Tests for `format_reply_context` — the helper that produces the
//! "[Replying to ...]" line the agent sees when a user replies to a
//! previous Telegram message, optionally highlighting a specific
//! excerpt via Telegram's quote-reply feature (issue #131).
//!
//! Before the fix the agent only saw the full text of the
//! replied-to message; users highlighting a single sentence inside
//! a long forwarded thread got a generic answer because the model
//! had no idea which sentence was being pointed at.

use crate::channels::telegram::handler::{format_reply_context, format_reply_sender};

#[test]
fn no_quote_falls_back_to_full_text() {
    let result = format_reply_context("Alice", "Hello, world!", "");
    assert_eq!(
        result,
        Some(r#"[Replying to Alice: "Hello, world!"]"#.into())
    );
}

#[test]
fn quote_alone_surfaces_only_quote() {
    // No full text (rare; reply target was a photo with no caption
    // but quote was somehow extracted from a related entity). The
    // formatter shouldn't synthesize an empty Full message: line.
    let result = format_reply_context("Bob", "", "selected text");
    assert_eq!(result, Some(r#"[Replying to Bob: "selected text"]"#.into()));
}

#[test]
fn quote_differs_from_full_text_shows_both() {
    let result = format_reply_context(
        "Carol",
        "The roof needs urgent repair. The walls are stable.",
        "The roof needs urgent repair.",
    );
    assert_eq!(
        result,
        Some(
            r#"[Replying to Carol, user highlighted: "The roof needs urgent repair."
Full message: "The roof needs urgent repair. The walls are stable."]"#
                .into()
        )
    );
}

#[test]
fn quote_equals_full_text_shows_once() {
    // When the user highlighted the entire message Telegram still
    // sends a quote; we shouldn't duplicate it in the prompt.
    let result = format_reply_context("Dave", "hi there", "hi there");
    assert_eq!(result, Some(r#"[Replying to Dave: "hi there"]"#.into()));
}

#[test]
fn whitespace_around_quote_is_trimmed() {
    // The Telegram client sometimes includes leading/trailing
    // whitespace in quote excerpts depending on how the user
    // highlighted (drag past a word boundary, double-tap on a
    // sentence end, etc.). The prompt should be clean.
    let result = format_reply_context("Eve", "full body of the message", "   highlighted   ");
    assert_eq!(
        result,
        Some(
            r#"[Replying to Eve, user highlighted: "highlighted"
Full message: "full body of the message"]"#
                .into()
        )
    );
}

#[test]
fn whitespace_around_full_text_is_trimmed() {
    let result = format_reply_context("Frank", "  spaced text  ", "");
    assert_eq!(result, Some(r#"[Replying to Frank: "spaced text"]"#.into()));
}

#[test]
fn empty_both_returns_none() {
    assert_eq!(format_reply_context("Grace", "", ""), None);
    assert_eq!(format_reply_context("Grace", "   ", "   "), None);
}

#[test]
fn quote_matches_full_text_after_trim() {
    // Edge case: the quote field had trailing whitespace but
    // logically matches the full text. Should fold to single
    // surface, not the dual "user highlighted: / Full message:"
    // format.
    let result = format_reply_context("Henry", "exact text", "exact text  ");
    assert_eq!(result, Some(r#"[Replying to Henry: "exact text"]"#.into()));
}

#[test]
fn assistant_sender_format_is_preserved() {
    // The caller maps bot replies to the literal "assistant" so
    // the model understands it's seeing its own prior turn.
    let result = format_reply_context("assistant", "I think we should refactor", "");
    assert_eq!(
        result,
        Some(r#"[Replying to assistant: "I think we should refactor"]"#.into())
    );
}

#[test]
fn multiline_full_message_with_short_quote() {
    let full = "Here is paragraph one.\nHere is paragraph two.\nHere is paragraph three.";
    let quote = "paragraph two";
    let result = format_reply_context("Iris", full, quote);
    let expected =
        format!("[Replying to Iris, user highlighted: \"{quote}\"\nFull message: \"{full}\"]");
    assert_eq!(result, Some(expected));
}

#[test]
fn unicode_quote_is_preserved() {
    // Common case in the user's groups — French/Spanish text.
    let result = format_reply_context(
        "Jules",
        "Le toit nécessite des réparations urgentes.",
        "réparations urgentes",
    );
    assert_eq!(
        result,
        Some(
            r#"[Replying to Jules, user highlighted: "réparations urgentes"
Full message: "Le toit nécessite des réparations urgentes."]"#
                .into()
        )
    );
}

// ---------------------------------------------------------------------------
// format_reply_sender — identify WHO is being replied to (username + user id)
// ---------------------------------------------------------------------------

#[test]
fn reply_sender_includes_username_and_id() {
    let label = format_reply_sender(false, "Alice", None, Some("alice_w"), 12345);
    assert_eq!(label, "Alice (@alice_w), ID 12345");
}

#[test]
fn reply_sender_includes_last_name() {
    let label = format_reply_sender(false, "Alice", Some("Wong"), Some("aw"), 42);
    assert_eq!(label, "Alice Wong (@aw), ID 42");
}

#[test]
fn reply_sender_without_username_still_has_id() {
    // No @handle, but the numeric ID must still let the agent disambiguate.
    let label = format_reply_sender(false, "Bob", None, None, 999);
    assert_eq!(label, "Bob, ID 999");
}

#[test]
fn reply_sender_for_bot_is_assistant() {
    // The bot collapses to a stable label, never its own username/id.
    let label = format_reply_sender(true, "OpenCrabs", None, Some("opencrabs_bot"), 7);
    assert_eq!(label, "assistant");
}

#[test]
fn reply_context_carries_full_sender_identity() {
    // End to end: the identity flows into the [Replying to ...] line.
    let sender = format_reply_sender(false, "Carol", None, Some("carol"), 555);
    let ctx = format_reply_context(&sender, "the original message", "");
    assert_eq!(
        ctx,
        Some(r#"[Replying to Carol (@carol), ID 555: "the original message"]"#.into())
    );
}

// ---------------------------------------------------------------------------
// resolve_reply_context — never fabricate when the replied-to content is gone
// ---------------------------------------------------------------------------

use crate::channels::telegram::handler::resolve_reply_context;

#[test]
fn unrecoverable_bot_reply_emits_explicit_marker_not_none() {
    // Rich/cron bot message we could not retrieve: the agent must be told the
    // content is unavailable so it says so instead of inventing a reply target.
    let ctx = resolve_reply_context("assistant", "", "", true);
    let text = ctx.expect("must emit a marker, not None");
    assert!(text.contains("could not be retrieved"), "got: {text}");
    assert!(
        text.contains("Do NOT guess"),
        "must forbid fabrication: {text}"
    );
}

#[test]
fn recoverable_content_ignores_the_unavailable_flag() {
    // When we DID retrieve content, the marker must never appear even if the
    // flag is set — real content always wins.
    let ctx = resolve_reply_context("assistant", "the real message", "", true);
    assert_eq!(
        ctx,
        Some(r#"[Replying to assistant: "the real message"]"#.into())
    );
}

#[test]
fn empty_non_bot_reply_stays_none() {
    // A user message with genuinely empty text (e.g. a sticker) and no bot
    // flag must stay None — no marker, no fabrication prompt.
    assert_eq!(resolve_reply_context("Alice", "", "", false), None);
}

#[test]
fn unrecoverable_human_reply_emits_marker_not_none() {
    // #548: the marker was bot-only, so a user-to-user reply to media (photo,
    // sticker, voice) produced NOTHING — the model could not even tell a reply
    // had happened. The flag is now sender-agnostic: the caller decides by
    // whether the target is a real message that we failed to read.
    let ctx = resolve_reply_context("Carol (@carol), ID 555", "", "", true);
    let text = ctx.expect("a human's unreadable reply target must emit a marker");
    assert!(text.contains("could not be retrieved"), "got: {text}");
    assert!(text.contains("Carol"), "must name who was replied to: {text}");
    assert!(text.contains("Do NOT guess"), "must forbid fabrication: {text}");
}

#[test]
fn marker_wording_is_not_bot_specific() {
    // The old text said "rich and cron bot messages" — wrong now that the
    // marker also covers a human's media message.
    let text = resolve_reply_context("Carol", "", "", true).expect("marker");
    assert!(
        !text.contains("bot messages"),
        "marker must not claim the class is bot-only: {text}"
    );
}
