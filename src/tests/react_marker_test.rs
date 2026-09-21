//! Tests for `utils::image::extract_react_marker` — the `<<react:emoji>>`
//! reaction directive extractor.
//!
//! The extractor is strict on purpose: only payloads that look like a real
//! emoji are treated as directives, and occurrences inside backtick code
//! spans are never extracted. A word payload (`<<react:emoji>>` written in
//! prose while discussing the feature) once fired a bogus REACTION_INVALID
//! Telegram call and mutated the final text, breaking exact-match dedup
//! against the already-sent intermediate — both copies landed in the chat.

use crate::utils::extract_react_marker;

// ── valid directives ─────────────────────────────────────────────────────

#[test]
fn react_bare_directive() {
    let (text, emoji) = extract_react_marker("<<react:👍>>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_directive_with_text() {
    let (text, emoji) = extract_react_marker("Sure thing! <<react:✅>>");
    assert_eq!(text, "Sure thing!");
    assert_eq!(emoji.as_deref(), Some("✅"));
}

#[test]
fn react_multiple_directives_uses_first() {
    let (text, emoji) = extract_react_marker("<<react:👍>> and <<react:❤️>>");
    assert_eq!(text, "and");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_whitespace_trimmed() {
    let (text, emoji) = extract_react_marker("  <<react:🔥>>  ");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("🔥"));
}

#[test]
fn react_with_surrounding_newlines() {
    let (text, emoji) = extract_react_marker("\n\n<<react:🔥>>\n\n");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("🔥"));
}

#[test]
fn react_embedded_in_middle() {
    let (text, emoji) = extract_react_marker("Hello <<react:👋>> world");
    assert_eq!(text, "Hello  world");
    assert_eq!(emoji.as_deref(), Some("👋"));
}

#[test]
fn react_only_react_with_no_extra_text() {
    // The common reaction-only case: LLM outputs just the directive
    let (text, emoji) = extract_react_marker("<<react:✅>>");
    assert!(text.trim().is_empty());
    assert_eq!(emoji.as_deref(), Some("✅"));
}

#[test]
fn react_compound_emoji_accepted() {
    // Skin-tone modifier (👍🏽) and VS-16 sequences (❤️) are multi-char but
    // still valid reaction payloads.
    let (text, emoji) = extract_react_marker("<<react:👍🏽>>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("👍🏽"));
}

// ── mangled prefix (models that escape the angle brackets) ───────────────

#[test]
fn react_escaped_single_backslash_prefix() {
    // Some models emit `<\react:` instead of `<<react:`, escaping the second
    // angle bracket. The reaction must still fire.
    let (text, emoji) = extract_react_marker(r"<\react:👍>>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_escaped_double_backslash_prefix() {
    let (text, emoji) = extract_react_marker(r"<\\react:👍>>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_single_angle_prefix() {
    // A single opening angle also normalizes to the same extraction.
    let (text, emoji) = extract_react_marker("<react:👍>>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_escaped_prefix_with_text() {
    let (text, emoji) = extract_react_marker(r"All set <\react:✅>>");
    assert_eq!(text, "All set");
    assert_eq!(emoji.as_deref(), Some("✅"));
}

#[test]
fn react_mangled_prefix_word_payload_stays() {
    // The emoji-validation guard still applies to a mangled prefix: a word
    // payload is not a directive and must survive intact.
    let (text, emoji) = extract_react_marker(r"<\react:emoji>>");
    assert_eq!(text, r"<\react:emoji>>");
    assert!(emoji.is_none());
}

#[test]
fn react_mangled_prefix_in_code_span_untouched() {
    // Code-span protection still applies to a mangled prefix.
    let (text, emoji) = extract_react_marker(r"use `<\react:👍>>` to react");
    assert_eq!(text, r"use `<\react:👍>>` to react");
    assert!(emoji.is_none());
}

// ── no directive ─────────────────────────────────────────────────────────

#[test]
fn react_no_directive() {
    let (text, emoji) = extract_react_marker("Just a normal message.");
    assert_eq!(text, "Just a normal message.");
    assert!(emoji.is_none());
}

#[test]
fn react_malformed_no_closing() {
    // No terminator at all (`>>`, `</react>`, or `>`) — left untouched.
    let (text, emoji) = extract_react_marker("<<react:👍");
    assert_eq!(text, "<<react:👍");
    assert!(emoji.is_none());
}

// ── tolerant terminator (models that hallucinate the close) ──────────────

#[test]
fn react_xml_close_tag_terminator() {
    // Cursor/Cline muscle memory: the model closes the directive with an
    // XML-style `</react>` instead of `>>`. It must still fire, and the
    // mangled marker must never leak into the chat as raw text.
    let (text, emoji) = extract_react_marker("<<react:👍</react>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_xml_close_tag_with_text() {
    let (text, emoji) = extract_react_marker("All set <<react:✅</react>");
    assert_eq!(text, "All set");
    assert_eq!(emoji.as_deref(), Some("✅"));
}

#[test]
fn react_bare_single_bracket_terminator() {
    // A single closing `>` (dropped the second bracket) still closes the
    // directive rather than leaking the marker.
    let (text, emoji) = extract_react_marker("<<react:🔥>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("🔥"));
}

#[test]
fn react_double_bracket_preferred_over_single() {
    // `>>` and a bare `>` both start at the same offset; the longer `>>`
    // must win so no stray bracket is left in the output.
    let (text, emoji) = extract_react_marker("<<react:👍>> done");
    assert_eq!(text, "done");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_xml_close_word_payload_stays() {
    // The emoji guard still applies to the tolerant terminator: a word
    // payload closed with `</react>` is prose, not a directive.
    let (text, emoji) = extract_react_marker("<<react:emoji</react>");
    assert_eq!(text, "<<react:emoji</react>");
    assert!(emoji.is_none());
}

#[test]
fn react_bare_terminator_in_code_span_untouched() {
    // Code-span protection still applies to the tolerant terminator.
    let (text, emoji) = extract_react_marker("use `<<react:👍>` to react");
    assert_eq!(text, "use `<<react:👍>` to react");
    assert!(emoji.is_none());
}

// ── keyword-less opener (models that drop the `react:` tag) ──────────────

#[test]
fn react_keywordless_double_bracket_fires() {
    // Some models drop the `react:` tag and just double-bracket the emoji.
    let (text, emoji) = extract_react_marker("<<✅>>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("✅"));
}

#[test]
fn react_keywordless_with_text() {
    let (text, emoji) = extract_react_marker("Done <<🔥>>");
    assert_eq!(text, "Done");
    assert_eq!(emoji.as_deref(), Some("🔥"));
}

#[test]
fn react_keywordless_bare_terminator_fires() {
    // Keyword-less opener + a single dropped bracket on the close.
    let (text, emoji) = extract_react_marker("<<👍>");
    assert_eq!(text, "");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_keywordless_single_bracket_stays_text() {
    // A single-bracket `<✅>` is one char from HTML/emoticon noise — it must
    // NOT be treated as a directive, only the double-bracket form is.
    let (text, emoji) = extract_react_marker("<✅>");
    assert_eq!(text, "<✅>");
    assert!(emoji.is_none());
}

#[test]
fn react_keywordless_word_payload_stays_text() {
    // The emoji guard still applies without the `react:` tag: an ASCII word
    // between double brackets is prose, not a reaction.
    let (text, emoji) = extract_react_marker("<<word>>");
    assert_eq!(text, "<<word>>");
    assert!(emoji.is_none());
}

#[test]
fn react_keywordless_in_code_span_untouched() {
    let (text, emoji) = extract_react_marker("use `<<✅>>` to react");
    assert_eq!(text, "use `<<✅>>` to react");
    assert!(emoji.is_none());
}

#[test]
fn react_keywordless_empty_payload_stays_text() {
    let (text, emoji) = extract_react_marker("<<>>");
    assert_eq!(text, "<<>>");
    assert!(emoji.is_none());
}

// ── prose mentions must NOT extract ──────────────────────────────────────

#[test]
fn react_word_payload_stays_in_prose() {
    // "emoji" is a placeholder word, not an emoji — the marker is prose
    // (e.g. the agent discussing this very feature) and must survive intact
    // so the text matches the already-sent intermediate for dedup.
    let (text, emoji) = extract_react_marker("the leak where <<react:emoji>> showed as raw text");
    assert_eq!(text, "the leak where <<react:emoji>> showed as raw text");
    assert!(emoji.is_none());
}

#[test]
fn react_hello_payload_not_extracted() {
    let (text, emoji) = extract_react_marker("<<react:hello>>");
    assert_eq!(text, "<<react:hello>>");
    assert!(emoji.is_none());
}

#[test]
fn react_empty_payload_not_extracted() {
    let (text, emoji) = extract_react_marker("<<react:>>");
    assert_eq!(text, "<<react:>>");
    assert!(emoji.is_none());
}

#[test]
fn react_marker_in_code_span_untouched() {
    // Even a REAL emoji payload is not a directive inside backticks — it's
    // quoted documentation.
    let (text, emoji) = extract_react_marker("use `<<react:👍>>` to react");
    assert_eq!(text, "use `<<react:👍>>` to react");
    assert!(emoji.is_none());
}

#[test]
fn react_directive_after_closed_code_span_still_extracts() {
    let (text, emoji) = extract_react_marker("see `code` here <<react:👍>>");
    assert_eq!(text, "see `code` here");
    assert_eq!(emoji.as_deref(), Some("👍"));
}

#[test]
fn react_long_payload_rejected() {
    // Over the 8-char cap — not a plausible single reaction emoji.
    let (text, emoji) = extract_react_marker("<<react:🔥🔥🔥🔥🔥🔥🔥🔥🔥>>");
    assert_eq!(text, "<<react:🔥🔥🔥🔥🔥🔥🔥🔥🔥>>");
    assert!(emoji.is_none());
}

#[test]
fn lenient_extractor_fires_a_code_span_marker() {
    // On a reaction turn the expected output IS a bare marker; a small model
    // wraps it in a code span and narrates its reasoning (#583). The strict
    // extractor leaves it as prose; the lenient one fires and strips it.
    use crate::utils::{extract_react_marker, extract_react_marker_lenient};

    let leaky = "So I should reply with only `<<react:🙏>>`.";
    let (strict_text, strict_emoji) = extract_react_marker(leaky);
    assert_eq!(
        strict_emoji, None,
        "strict leaves a code-span marker as prose"
    );
    assert!(strict_text.contains("<<react:🙏>>"));

    let (lenient_text, lenient_emoji) = extract_react_marker_lenient(leaky);
    assert_eq!(lenient_emoji.as_deref(), Some("🙏"));
    assert!(
        !lenient_text.contains("<<react:"),
        "lenient strips the marker: {lenient_text:?}"
    );

    // Lenient still validates the payload: a word payload never fires.
    let (_t, none) = extract_react_marker_lenient("see `<<react:emoji>>` in the docs");
    assert_eq!(none, None);
}

// ── orphan-fence recovery (#1182, observed live 2026-08-24) ──────────────
//
// The model wrapped its reaction turn in an orphan code fence with a stray
// leading angle bracket. The triple backtick flips `in_code` an odd number of
// times, so the strict extractor used to skip the directive and the raw text
// `<```html <react:✅>> ...` leaked into the Telegram group verbatim.

#[test]
fn react_recovered_from_orphan_fence_single_bracket() {
    // EXACT shape posted to the group at ~18:00 local on 2026-08-24.
    let raw = "<```html\n<react:✅>> Done — all receipts verified in the last run:\n\nbody stays put\n```>";
    let (text, emoji) = extract_react_marker(raw);
    assert_eq!(emoji.as_deref(), Some("✅"));
    assert!(!text.contains("```"), "fence lines must go: {text:?}");
    assert!(!text.contains("<react:"), "marker must go: {text:?}");
    assert!(text.contains("body stays put"));
}

#[test]
fn react_recovered_from_orphan_fence_double_bracket() {
    // Same incident, earlier turn (~17:53 local): canonical <<react:>> inside
    // the same orphan fence.
    let raw = "<```html\n<<react:💯>>\n\nChecked before arguing, and empirically right.\n```>";
    let (text, emoji) = extract_react_marker(raw);
    assert_eq!(emoji.as_deref(), Some("💯"));
    assert!(!text.contains("```"));
    assert!(text.contains("empirically right"));
}

#[test]
fn react_recovered_from_bare_orphan_fence() {
    let raw = "<```\n<<react:🔥>>\nplain body";
    let (text, emoji) = extract_react_marker(raw);
    assert_eq!(emoji.as_deref(), Some("🔥"));
    assert!(!text.contains("```"));
    assert!(text.contains("plain body"));
}

#[test]
fn react_recovered_when_marker_shares_the_fence_line() {
    let raw = "<```html <<react:👀>> visible text";
    let (text, emoji) = extract_react_marker(raw);
    assert_eq!(emoji.as_deref(), Some("👀"));
    assert!(!text.contains("```html"));
    assert!(text.contains("visible text"));
}

#[test]
fn react_recovery_strips_paired_trailing_fence_only() {
    // A trailing fence line is dropped WITH recovery; a message that never had
    // a leading orphan fence keeps its content untouched.
    let clean = "<<react:👍>> normal turn\n```";
    let (text, emoji) = extract_react_marker(clean);
    assert_eq!(emoji.as_deref(), Some("👍"));
    assert!(text.contains("normal turn"), "{text:?}");
}

#[test]
fn react_no_recovery_for_prose_then_fenced_example() {
    // Docs discussing the feature: prose first, THEN a fenced example. The
    // strict guard must keep holding there — no bogus reaction fires.
    let docs = "Use fences like this:\n```html\n<<react:✅>>\n```\nto react.";
    let (text, emoji) = extract_react_marker(docs);
    assert_eq!(emoji, None);
    assert!(
        text.contains("<<react:✅>>"),
        "example stays as text: {text:?}"
    );
}

#[test]
fn react_no_recovery_for_word_payload_in_orphan_fence() {
    // Fence-shaped junk but a WORD payload: still prose, nothing fires,
    // original text comes back untouched.
    let raw = "<```html\n<react:hello>> not an emoji";
    let (text, emoji) = extract_react_marker(raw);
    assert_eq!(emoji, None);
    assert_eq!(text, raw);
}

// ── strip_invalid_react_markers (#1670) ──────────────────────────────────
// The reaction path delivers what the extractor leaves behind. A malformed
// marker (empty payload, word payload) survives extraction as visible text
// and shipped as a bubble into the group's General topic. These tests pin
// the reaction-path-only strip that closes the leak.

use crate::utils::strip_invalid_react_markers as strip;

#[test]
fn strip_incident_repro_empty_payload_goes_to_silence() {
    // The exact #1670 shape: the model emitted a bare empty-payload marker.
    assert_eq!(strip("<<react:>>"), "");
}

#[test]
fn strip_empty_payload_with_narration_keeps_the_narration() {
    // Debris is removed, real narration survives (stop-signal turns reply).
    assert_eq!(
        strip("I will not react to that. <<react:>>"),
        "I will not react to that."
    );
}

#[test]
fn strip_word_payload_removed() {
    assert_eq!(strip("<<react:emoji>>"), "");
    assert_eq!(strip("Sure <<react:hello>> done"), "Sure done");
}

#[test]
fn strip_keyword_less_noise_removed() {
    assert_eq!(strip("<<hello>>"), "");
}

#[test]
fn strip_backtick_wrapped_debris_leaves_no_stray_span() {
    // A stranded "``" pair is still a visible bubble; the span goes with it.
    assert_eq!(strip("`<<react:>>`"), "");
    assert_eq!(strip("narration `<<react:>>` tail"), "narration tail");
}

#[test]
fn strip_valid_markers_are_left_for_the_extractor() {
    // Defensive: the extractor consumes valid markers BEFORE the strip runs.
    // If ever called first, a valid marker must survive untouched.
    assert_eq!(strip("<<react:👍>>"), "<<react:👍>>");
    assert_eq!(strip("Sure <<react:✅>>"), "Sure <<react:✅>>");
}

#[test]
fn strip_unterminated_opening_is_left_as_text() {
    // No terminator to anchor a strip to; eating the rest would be a worse
    // leak. Same shape the extractor already leaves (react_malformed_no_closing).
    assert_eq!(strip("<<react:"), "<<react:");
    assert_eq!(strip("<<react:👍"), "<<react:👍");
}

#[test]
fn strip_plain_prose_untouched() {
    assert_eq!(strip("Just a normal message."), "Just a normal message.");
    assert_eq!(strip("a < b and c > d"), "a < b and c > d");
}

#[test]
fn strip_escaped_prefix_forms_removed_too() {
    // The tolerant openers (single bracket, backslash escapes) are debris
    // shapes the same scanner matches.
    assert_eq!(strip("<react:>>"), "");
    assert_eq!(strip("<\\react:>>"), "");
}

#[test]
fn strip_multiple_debris_all_gone() {
    assert_eq!(strip("<<react:>><<react:>>"), "");
    assert_eq!(strip("<<react:>>x<<react:emoji>>"), "x");
}

#[test]
fn reaction_path_wires_strip_after_extractor_and_threads_the_reply() {
    // Source sentinel: the handler must (a) call the strip AFTER the lenient
    // extractor, and (b) deliver the reaction text reply with the resolved
    // topic instead of None (#1670 defect B — every reply landed in General).
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/channels/telegram/handler.rs"),
    )
    .expect("handler source");
    let extract = src
        .find("extract_react_marker_lenient(&text_only)")
        .expect("lenient extractor call");
    let strip_at = src
        .find("strip_invalid_react_markers(&text_only)")
        .expect("debris strip call");
    assert!(
        extract < strip_at,
        "the strip must run on the extractor's output, not before it"
    );
    let deliver = src
        .find("topic_id.map(|t| ThreadId(MessageId(t))),")
        .expect("reaction text reply must carry the resolved topic (#1670)");
    assert!(
        strip_at < deliver,
        "sentinel anchors to the reaction handler, not some other path"
    );
}
