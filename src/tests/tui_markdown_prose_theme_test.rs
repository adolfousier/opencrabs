//! #1634 phase 2: body prose must carry an explicit foreground.
//!
//! The defect these pin is not a wrong colour, it is the absence of one.
//! `folded_style` folded the emphasis stack over a bare `Style::default()`, so
//! every unemphasised span left the parser with `fg: None`. Ratatui passes that
//! through untouched and the terminal resolves it against its own default
//! foreground, which no theme can move. Body text therefore rendered byte for
//! byte identically under crab-dark, dracula, alucard and every imported pack
//! theme, which is most of what "the theme barely changes anything" meant.
//!
//! These assertions deliberately check `fg.is_some()` and relative difference rather than
//! a literal colour: `theme::role` reads process-global state that the one
//! sanctioned mutator test in `tui_theme_presets_test` moves, so pinning an
//! absolute value here would race it. The theme-follows-palette half lives in
//! that mutator test instead.

use ratatui::text::Span;

use crate::tui::markdown::parse_markdown;

const WIDTH: usize = 80;

/// Every span on every rendered line, flattened.
fn spans(markdown: &str) -> Vec<Span<'static>> {
    parse_markdown(markdown, WIDTH)
        .into_iter()
        .flat_map(|l| l.spans)
        .collect()
}

/// The span carrying `needle`, or a panic naming what was actually produced.
fn span_with(markdown: &str, needle: &str) -> Span<'static> {
    let all = spans(markdown);
    all.iter()
        .find(|s| s.content.contains(needle))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "no span containing {needle:?}; got {:?}",
                all.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>()
            )
        })
}

/// The regression itself: plain paragraph text is themeable.
#[test]
fn plain_prose_carries_an_explicit_foreground() {
    let s = span_with("just some ordinary prose", "ordinary");
    assert!(
        s.style.fg.is_some(),
        "plain prose shipped fg: None, so no theme can colour it: {:?}",
        s.style
    );
}

/// Whitespace-only spans aside, nothing in a normal answer may leak through
/// with a default foreground.
#[test]
fn no_visible_prose_span_defaults_its_foreground() {
    let doc = "A paragraph with **bold**, *italic*, ~~struck~~ and a trailing word.\n\n\
               - a bullet item\n- another one\n\n\
               > a quoted line\n";
    for s in spans(doc) {
        if s.content.trim().is_empty() {
            continue;
        }
        assert!(
            s.style.fg.is_some(),
            "span {:?} has no foreground, so it renders in the terminal default \
             under every theme",
            s.content
        );
    }
}

/// Emphasis that only sets a modifier must inherit the prose colour rather
/// than fall back to the terminal's: `patch` leaves `fg: None` alone, which is
/// exactly why the seed had to move rather than each tag gain a colour.
#[test]
fn modifier_only_emphasis_inherits_the_prose_colour() {
    let plain = span_with("plain words here", "plain words here");
    let bold = span_with("a **loudword** here", "loudword");
    assert!(bold.style.fg.is_some());
    assert_eq!(
        bold.style.fg, plain.style.fg,
        "bold sets a modifier, not a colour, so it must read as body text"
    );
    assert!(
        bold.style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "bold lost its weight: {:?}",
        bold.style
    );
}

/// Tags that DO set a colour still win over the seed. If the fold had been
/// applied the other way round, inline code would have been flattened into
/// body text and the answer would lose its only in-sentence accent.
#[test]
fn inline_code_still_overrides_the_prose_colour() {
    let plain = span_with("plain words here", "plain words here");
    let code = span_with("call `norm_key` now", "norm_key");
    assert!(code.style.fg.is_some());
    assert_ne!(
        code.style.fg, plain.style.fg,
        "inline code collapsed into body text"
    );
}

/// Same for links, which carry both a colour and an underline.
#[test]
fn link_text_still_overrides_the_prose_colour() {
    let plain = span_with("plain words here", "plain words here");
    let link = span_with("see [the docs](https://example.com) now", "the docs");
    assert!(link.style.fg.is_some());
    assert_ne!(
        link.style.fg, plain.style.fg,
        "link text collapsed into body text"
    );
}

/// Inline HTML is rendered as prose rather than swallowed, so it takes the
/// prose colour too. It was the one other `Style::default()` text site.
#[test]
fn inline_html_text_carries_the_prose_colour() {
    let plain = span_with("plain words here", "plain words here");
    let html = span_with("a <tool_use> tag in prose", "tool_use");
    assert_eq!(
        html.style.fg, plain.style.fg,
        "inline HTML fell back to the terminal default"
    );
}
