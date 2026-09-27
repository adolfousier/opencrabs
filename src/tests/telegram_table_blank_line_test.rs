//! Regression for #95: a GFM table that directly abuts a text line renders as
//! raw pipes in Telegram's rich-markdown dialect (probe matrix A/B/C/D — blank
//! line before the table = rendered, abutting = raw).
//! `ensure_blank_line_before_tables` inserts the missing blank line before
//! exactly the blocks `rich/table::try_parse` accepts, never mutates code
//! fences, and is idempotent.
//!
//! Also covers its sibling pass #552: a blockquote run not terminated by a blank
//! line swallows a following block-level HTML opener (`<details>`) as a CommonMark
//! lazy continuation, so the opener lands inside the quote, its closers go
//! unmatched, and Telegram rejects the WHOLE message with
//! `RICH_MESSAGE_CONTENT_REQUIRED` — the card silently falls back to HTML.
//! `ensure_blank_line_before_block_html` terminates the run at exactly that
//! boundary, is fence-safe, and is idempotent.

use crate::channels::telegram::rich::table::{
    ensure_blank_line_before_block_html, ensure_blank_line_before_tables,
};

#[test]
fn abutting_table_gets_blank_line_inserted() {
    // Probe B → probe C: the failing GREEN-report shape (bold line glued to
    // the table, no blank line between).
    let input = "**Tests** (host-diag exits, disk, docker):\n| Check | Result |\n|---|---|\n| uptime | ok |";
    let expected = "**Tests** (host-diag exits, disk, docker):\n\n| Check | Result |\n|---|---|\n| uptime | ok |";
    assert_eq!(ensure_blank_line_before_tables(input), expected);
}

#[test]
fn blank_line_before_table_is_untouched() {
    // Probe A/C shape: blank line already present — byte-identical no-op.
    let input = "Text line:\n\n| Check | Result |\n|---|---|\n| uptime | ok |";
    assert_eq!(ensure_blank_line_before_tables(input), input);
}

#[test]
fn table_inside_code_fence_is_untouched() {
    // A pipe-table look-alike inside a fence is code content, never mutated.
    let input = "```text\nheader above\n| A | B |\n|---|---|\n```\n";
    assert_eq!(ensure_blank_line_before_tables(input), input);
}

#[test]
fn pass_is_idempotent() {
    let input = "**Tests**:\n| A | B |\n|---|---|\n| 1 | 2 |";
    let once = ensure_blank_line_before_tables(input);
    assert_eq!(ensure_blank_line_before_tables(&once), once);
}

#[test]
fn pipe_free_input_is_untouched() {
    let input = "# Report\n\nAll clear, no pipes in sight.\n";
    assert_eq!(ensure_blank_line_before_tables(input), input);
}

#[test]
fn stray_pipe_prose_is_untouched() {
    // A pipe without a separator row is not a table (the try_parse gate) —
    // prose is never mutated.
    let input = "cost | value\nno separator here\n| also not a table";
    assert_eq!(ensure_blank_line_before_tables(input), input);
}

// ---- #552: blockquote run swallowed a following HTML block ----
//
// A blockquote run that is not terminated by a blank line swallows a following
// block-level HTML opener as a CommonMark lazy continuation: the opener lands
// inside the quote, its closers go unmatched, and Telegram rejects the whole
// message with `RICH_MESSAGE_CONTENT_REQUIRED` — the card silently falls back to
// HTML. `ensure_blank_line_before_block_html` terminates the run at exactly that
// boundary. Every case below asserts against the real pass, so each one fails if
// the pass is removed or its opener set is broken.

#[test]
fn blockquote_then_details_gets_blank_line_inserted() {
    // The issue's minimal repro, verbatim: a blockquote run with no blank line
    // after it, directly followed by a <details> block. This exact shape made
    // Telegram reject the whole card (#552, measured 409 rejections on 09-24).
    let input = "> quoted line here\n<details><summary><b>Context</b></summary>\n\nbody text\n\n</details>";
    let expected = "> quoted line here\n\n<details><summary><b>Context</b></summary>\n\nbody text\n\n</details>";
    assert_eq!(ensure_blank_line_before_block_html(input), expected);
}

#[test]
fn blockquote_then_details_with_blank_line_is_untouched() {
    // The issue's own control: the same body already terminated by a blank line.
    // Must be byte-identical — this is the over-insertion guard.
    let input = "> quoted line here\n\n<details><summary><b>Context</b></summary>\n\nbody text\n\n</details>";
    assert_eq!(ensure_blank_line_before_block_html(input), input);
}

#[test]
fn html_opener_inside_code_fence_is_untouched() {
    // A <details> look-alike inside a fence is code content, never mutated.
    // The fence must be tracked exactly as the sibling table pass tracks it.
    let input = "> quoted line here\n```text\n<details>\n```\n";
    assert_eq!(ensure_blank_line_before_block_html(input), input);
}

#[test]
fn inline_tag_after_blockquote_is_untouched() {
    // An INLINE tag after a quote stays inline content: splitting it off would
    // change rendering the message never asked for. Only BLOCK openers fire.
    let input = "> quoted line here\n<b>bold</b> inline";
    assert_eq!(ensure_blank_line_before_block_html(input), input);
}

#[test]
fn block_html_pass_is_idempotent() {
    // A second application must change nothing — otherwise every rich refresh
    // would grow the body by one blank line.
    let input = "> quoted line here\n<details><summary>s</summary>\n\nbody\n\n</details>";
    let expected = "> quoted line here\n\n<details><summary>s</summary>\n\nbody\n\n</details>";
    assert_eq!(ensure_blank_line_before_block_html(input), expected);
    let once = ensure_blank_line_before_block_html(input);
    assert_eq!(ensure_blank_line_before_block_html(&once), once);
}

#[test]
fn blockquote_then_paragraph_is_untouched() {
    // A quote run followed by ordinary prose is NOT the defect shape — only a
    // block-level HTML opener is swallowed as a lazy continuation.
    let input = "> quoted line one\n> quoted line two\nplain paragraph";
    assert_eq!(ensure_blank_line_before_block_html(input), input);
}

#[test]
fn closing_tag_and_comment_after_blockquote_are_untouched() {
    // A closing tag and an HTML comment open no element. Treating either as an
    // opener would insert a spurious blank line.
    let input = "> quoted line here\n</details>\n<!-- note -->";
    assert_eq!(ensure_blank_line_before_block_html(input), input);
}

#[test]
fn blockquote_then_multiline_html_opener_gets_one_blank_line() {
    // A quote run of several lines, then a block opener: exactly ONE blank line
    // goes in, at the boundary — not one per quote line.
    let input = "> line one\n> line two\n> line three\n<pre>code</pre>";
    let expected = "> line one\n> line two\n> line three\n\n<pre>code</pre>";
    assert_eq!(ensure_blank_line_before_block_html(input), expected);
}

#[test]
fn html_opener_without_a_preceding_quote_is_untouched() {
    // No quote run above it means no lazy continuation: the pass must not fire
    // on a body that merely contains HTML.
    let input = "plain line\n<details>\nbody\n</details>";
    assert_eq!(ensure_blank_line_before_block_html(input), input);
}

#[test]
fn html_free_input_is_untouched() {
    let input = "> quoted line here\nplain text, no tags at all";
    assert_eq!(ensure_blank_line_before_block_html(input), input);
}

#[test]
fn uppercase_and_attributed_openers_are_detected() {
    // Tag matching is case-insensitive, and an opener carrying attributes is
    // still an opener.
    let input = "> quoted line here\n<DETAILS open class=\"x\">\nbody\n</DETAILS>";
    let expected = "> quoted line here\n\n<DETAILS open class=\"x\">\nbody\n</DETAILS>";
    assert_eq!(ensure_blank_line_before_block_html(input), expected);
}

#[test]
fn pass_reaches_the_canonical_rich_entry() {
    // Wiring invariant: the fix must be reachable from the entry every rich path
    // funnels through, not only from the pass itself. A body carrying the shape
    // comes back normalized.
    let input = "> quoted line here\n<details><summary>s</summary>\n\nbody\n\n</details>";
    let normalized = crate::channels::telegram::rich::table::normalize_tables(input);
    assert!(
        normalized.contains("> quoted line here\n\n<details>"),
        "canonical entry did not terminate the quote run: {normalized:?}"
    );
}
