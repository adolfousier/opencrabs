//! Tests for Telegram rich-text inline parse (HTML tag mapping).
//!
//! Extracted from an inline `#[cfg(test)]` block in
//! `src/channels/telegram/rich/inline.rs`; project policy
//! (CONTRIBUTING.md) requires all tests under `src/tests/`.

use crate::channels::telegram::rich::ast::Inline;
use crate::channels::telegram::rich::inline::parse_inlines;

fn kinds(input: &str) -> Vec<String> {
    parse_inlines(input)
        .iter()
        .map(|i| match i {
            Inline::Text(_) => "text".into(),
            Inline::Bold(_) => "bold".into(),
            Inline::Italic(_) => "italic".into(),
            Inline::Underline(_) => "underline".into(),
            Inline::Strike(_) => "strike".into(),
            Inline::Sub(_) => "sub".into(),
            Inline::Code(_) => "code".into(),
            Inline::Math(_) => "math".into(),
            Inline::Link { .. } => "link".into(),
        })
        .collect()
}

fn text_of(input: &str) -> String {
    fn flatten(inlines: &[Inline], out: &mut String) {
        for i in inlines {
            match i {
                Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => out.push_str(t),
                Inline::Bold(c)
                | Inline::Italic(c)
                | Inline::Underline(c)
                | Inline::Strike(c)
                | Inline::Sub(c) => flatten(c, out),
                Inline::Link { content, .. } => flatten(content, out),
            }
        }
    }
    let mut out = String::new();
    flatten(&parse_inlines(input), &mut out);
    out
}

#[test]
fn html_style_tags_map_to_inline_variants() {
    assert_eq!(kinds("<b>hi</b>"), vec!["bold"]);
    assert_eq!(kinds("<strong>hi</strong>"), vec!["bold"]);
    assert_eq!(kinds("<i>hi</i>"), vec!["italic"]);
    assert_eq!(kinds("<em>hi</em>"), vec!["italic"]);
    assert_eq!(kinds("<u>hi</u>"), vec!["underline"]);
    assert_eq!(kinds("<s>hi</s>"), vec!["strike"]);
    assert_eq!(kinds("<del>hi</del>"), vec!["strike"]);
}

#[test]
fn html_tags_recurse_and_mix_with_markdown() {
    assert_eq!(kinds("<b>a *b* c</b>"), vec!["bold"]);
    assert_eq!(kinds("x<b>y</b>z"), vec!["text", "bold", "text"]);
    assert_eq!(kinds("**a** <i>b</i>"), vec!["bold", "text", "italic"]);
}

#[test]
fn unmatched_openers_stay_literal() {
    // No closer → whole thing is literal text, escaped downstream.
    assert_eq!(kinds("<b>unclosed"), vec!["text"]);
    assert_eq!(text_of("<b>unclosed"), "<b>unclosed");
    // Empty body → tag_paired returns None → whole string falls through
    // to the text accumulator as ONE literal span (same law as `**` pairs).
    assert_eq!(kinds("<b></b>"), vec!["text"]);
}

#[test]
fn summary_b_renders_as_bold_not_escape() {
    // The #106 live shape: `<b>▸ summary</b>` from a degraded Details.
    assert_eq!(kinds("<b>Summary text</b>"), vec!["bold"]);
    assert_eq!(text_of("<b>Summary text</b>"), "Summary text");
}
