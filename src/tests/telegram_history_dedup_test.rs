//! Unit tests for Telegram group history and reply context deduplication (#133).
//!
//! Tests verify that:
//! 1. `normalize_for_dedup` collapses whitespace and normalizes case.
//! 2. `is_content_in_live_context` correctly matches exact and whitespace-varying substrings.
//! 3. `format_reply_context_pruned` prunes redundant full-message bodies when in context:
//!    - with quote: emits only `[Replying to {sender}, user highlighted: "{quote}"]`.
//!    - without quote: emits `[Replying to {sender}'s message above]`.
//! 4. When uncompacted / not in context, standard full-message behavior is preserved.

use crate::channels::telegram::handler::{
    format_reply_context_pruned, is_content_in_live_context, normalize_for_dedup,
    resolve_reply_context_pruned,
};

#[test]
fn normalize_for_dedup_collapses_whitespace_and_case() {
    let input = "  Hello \n\t  WORLD!   This is   a test. \n";
    let normalized = normalize_for_dedup(input);
    assert_eq!(normalized, "hello world! this is a test.");
}

#[test]
fn normalize_for_dedup_empty_string() {
    assert_eq!(normalize_for_dedup(""), "");
    assert_eq!(normalize_for_dedup("   \n\t "), "");
}

#[test]
fn is_content_in_live_context_matches_present_content() {
    let haystacks = vec![
        normalize_for_dedup("Here is some earlier assistant answer discussing architecture."),
        normalize_for_dedup("User asked: what about caching?"),
    ];

    assert!(is_content_in_live_context(
        "earlier assistant answer discussing architecture",
        &haystacks
    ));
    assert!(is_content_in_live_context(
        "  User  asked:   what about caching?  ",
        &haystacks
    ));
}

#[test]
fn is_content_in_live_context_returns_false_for_absent_content() {
    let haystacks = vec![
        normalize_for_dedup("Session message 1"),
        normalize_for_dedup("Session message 2"),
    ];

    assert!(!is_content_in_live_context(
        "Unseen message from 2 hours ago",
        &haystacks
    ));
    assert!(!is_content_in_live_context("", &haystacks));
}

#[test]
fn format_reply_context_pruned_when_full_in_context_with_quote() {
    let full = "This is a very long message that is already in context paragraph 1 paragraph 2";
    let quote = "paragraph 1";
    let formatted = format_reply_context_pruned("Alice", full, quote, true);
    assert_eq!(
        formatted,
        Some(r#"[Replying to Alice, user highlighted: "paragraph 1"]"#.into())
    );
}

#[test]
fn format_reply_context_pruned_when_full_in_context_without_quote() {
    let full = "This is a very long message that is already in context paragraph 1 paragraph 2";
    let formatted = format_reply_context_pruned("Alice", full, "", true);
    assert_eq!(
        formatted,
        Some(r#"[Replying to Alice's message above]"#.into())
    );
}

#[test]
fn format_reply_context_pruned_when_not_in_context_preserves_full_message() {
    let full = "Complete body of original message";
    let quote = "original message";
    let formatted = format_reply_context_pruned("Bob", full, quote, false);
    assert_eq!(
        formatted,
        Some(
            r#"[Replying to Bob, user highlighted: "original message"
Full message: "Complete body of original message"]"#
                .into()
        )
    );
}

#[test]
fn resolve_reply_context_pruned_with_unrecoverable_bot_reply() {
    let formatted = resolve_reply_context_pruned("assistant", "", "", true, false);
    let text = formatted.expect("must emit unrecoverable marker");
    assert!(text.contains("could not be retrieved"));
}
