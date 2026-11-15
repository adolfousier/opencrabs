//! The chat gives each voice its own side, and the pane chrome clears its ring.
//!
//! The window has no headless harness, so the shape is read from source, the
//! way the other gui tests do it. Three defects are guarded here:
//!
//! - the reader's message sat on the same left margin as the answer, because
//!   the body that wraps the bubble was a block box and handed it the whole
//!   column;
//! - the split-pane title carried no weight and both chrome rows sat inside
//!   the 24 px corner arc;
//! - the chat surface had no corner, so inside a pane its square fill painted
//!   over the ring.
//!
//! The bubble stays narrower than the cards around it: `MESSAGE_RADIUS` is
//! asserted against `CARD_RADIUS` in `transcript.rs`, and nothing here widens
//! it.

const TRANSCRIPT: &str = include_str!("../gui/transcript.rs");
const PANE_VIEW: &str = include_str!("../gui/pane_view.rs");
const PANE_CHROME: &str = include_str!("../gui/pane_chrome.rs");

/// The body of one top-level function, from its anchor to the next top-level
/// item. The cut has to include the `pub(crate)` and `pub` forms: the file's
/// own test module is indented, so a bare `\nfn ` would run past the end of the
/// function and read assertions out of unrelated tests.
fn body_after<'a>(source: &'a str, anchor: &str) -> &'a str {
    let rest = source
        .split_once(anchor)
        .unwrap_or_else(|| panic!("{anchor} is gone from the source"))
        .1;
    let mut end = rest.len();
    for stop in ["\nfn ", "\npub fn ", "\npub(crate) fn ", "\n}"] {
        if let Some(at) = rest.find(stop) {
            end = end.min(at);
        }
    }
    // Every candidate is a byte offset returned by `find`, or `rest.len()`, so
    // `end` always lands on a char boundary. `split_at` says that once instead
    // of an index expression the byte-slice ratchet has to re-litigate.
    rest.split_at(end).0
}

/// The reader's stored row hugs the right edge, and only his row does. The
/// body is a row of one pushed to its end; a block body would hand the bubble
/// the whole column, which is exactly what put both roles on the left.
#[test]
fn the_readers_stored_row_is_pushed_to_the_right() {
    let builder = body_after(TRANSCRIPT, "fn message_row(\n    id: uuid::Uuid,");
    assert!(
        builder.contains("mine: bool"),
        "the row is told whose message it draws"
    );
    assert!(
        builder.contains(".flex().flex_row().justify_end()"),
        "the reader's body is a row pushed to its end"
    );
    assert!(
        builder.contains("if mine"),
        "the push is conditional, so an answer keeps the left margin"
    );
}

/// A live turn is drawn by a second builder. If only the stored row moved, the
/// bubble would jump to the left the moment the turn persisted.
#[test]
fn the_live_turn_sits_where_the_stored_row_will() {
    let builder = body_after(TRANSCRIPT, "fn live_user_row(\n    slot: LiveSlot,");
    assert_eq!(
        builder.matches(".justify_end()").count(),
        1,
        "the live body is pushed right, once"
    );
}

/// A flex item that refuses to shrink sizes to max-content, so the bubble has
/// to be capped or a long message paints one line past the pane.
#[test]
fn the_bubble_is_capped_to_the_column() {
    let column = body_after(TRANSCRIPT, "fn bubble(\n    role: &str,")
        .split("if bubble_has_surface")
        .next()
        .unwrap_or_default();
    assert!(
        column.contains(".flex_shrink_0()"),
        "the column still refuses to be squeezed below its reserved height"
    );
    assert!(
        column.contains(".max_w_full()"),
        "the bubble wraps at the column width instead of running past the pane"
    );
}

/// The chat surface carries the card radius. It must not clip: the element is
/// already the scroller, and gpui's style setters are last-write-wins, so a
/// later clip would replace the scroll and freeze the transcript.
#[test]
fn the_chat_surface_is_rounded_and_still_scrolls() {
    let column = body_after(TRANSCRIPT, ".id(\"transcript\")")
        .split("if let Some(wheel)")
        .next()
        .unwrap_or_default();
    assert!(
        column.contains(".rounded(px(CARD_RADIUS))"),
        "the chat surface carries the same corner as the cards"
    );
    assert!(
        column.contains(".overflow_y_scroll()"),
        "the transcript still scrolls"
    );
    assert!(
        !column.contains(".overflow_hidden()"),
        "a clip here would replace the scroll, because the setters are last-write-wins"
    );
}

/// The pane paints its own fill and clips its children, so no child can paint
/// a square corner over the ring's arc.
#[test]
fn the_pane_clips_its_children_to_the_ring() {
    assert_eq!(
        PANE_VIEW.matches(".rounded(px(CARD_RADIUS))").count(),
        1,
        "one pane, one ring"
    );
    let ring = PANE_VIEW
        .find(".rounded(px(CARD_RADIUS))")
        .expect("the pane keeps its ring");
    let fill = PANE_VIEW
        .find(".bg(rgb(palette::current().bg))")
        .expect("the pane paints its own fill");
    let clip = PANE_VIEW
        .find(".overflow_hidden()")
        .expect("the pane clips its children");
    assert!(fill < ring, "the fill sits on the rounded box");
    assert!(ring < clip, "the clip comes after the ring in the chain");
}

/// The pane title is the pane's name, so it carries a heading's weight instead
/// of reading as one more line of body text.
#[test]
fn the_pane_title_carries_weight() {
    let header = body_after(PANE_CHROME, "pub(crate) fn pane_header(");
    assert!(
        header.contains(".font_weight(gpui::FontWeight::BOLD)"),
        "the title is bold"
    );
}

/// Both chrome rows step in from the corner: at 8 px of padding the 24 px arc
/// reaches the first character. The close glyph keeps its own padding, so the
/// button does not grow with the row.
#[test]
fn both_chrome_rows_clear_the_corner() {
    assert_eq!(
        PANE_CHROME.matches(".px_3()").count(),
        2,
        "the title row and the footer row both step in"
    );
    assert_eq!(
        PANE_CHROME.matches(".py_2()").count(),
        2,
        "and both step down"
    );
    assert_eq!(
        PANE_CHROME.matches(".px_2()").count(),
        1,
        "the close glyph keeps its own padding"
    );
}

/// A percentage cap is ignored while the row's width is indefinite, which is
/// exactly the narrow split pane: the bubble then sized to max-content and
/// `justify_end` pushed its beginning out of the left edge. The pixel cap comes
/// from the width the row was planned against, so no pass can ignore it
/// (#2937).
#[test]
fn the_bubble_is_capped_in_pixels() {
    let bubble = body_after(TRANSCRIPT, "fn bubble(");
    assert!(
        bubble.contains(".when(content_width > 0.0, |d| d.max_w(px(content_width)))"),
        "the bubble is capped in pixels, from the planned row width"
    );
    let cap = bubble
        .find("d.max_w(px(content_width))")
        .expect("the pixel cap is present");
    let percent = bubble
        .find(".max_w_full()")
        .expect("the percentage cap stays as a second belt");
    assert!(cap < percent, "the pixel cap is what holds in a tight pane");
}

/// The planned width is the transcript's outer measure, but its root spends
/// half a rem on each horizontal side. The bubble must inherit the remaining
/// inner width or right alignment pushes that rem of excess off the left
/// (#2947).
#[test]
fn the_pixel_cap_excludes_the_transcript_padding() {
    let view = body_after(TRANSCRIPT, "pub fn view(");
    assert!(
        view.contains("let content_width = transcript_content_width(content_width, rem_size);"),
        "the rows use the inner width, not the transcript's outer measure"
    );
    assert!(
        view.contains(".p_2()"),
        "this subtraction belongs to the root's actual horizontal padding"
    );
    let helper = body_after(TRANSCRIPT, "fn transcript_content_width(");
    assert!(
        helper.contains("(outer_width - rem_size).max(0.0)"),
        "two p_2 sides consume one rem total, clamped at zero"
    );
}

/// gpui clips children to a rectangle: `ContentMask` carries bounds and no
/// corner radii, so a clip on the pane can never round a child's own fill. The
/// header paints the pane's top edge, so it carries the radius itself, and the
/// radius is set before the fill it shapes (#2936).
#[test]
fn the_header_fill_follows_the_ring() {
    let header = body_after(PANE_CHROME, "pub(crate) fn pane_header(");
    let round = header
        .find(".rounded_t(px(CARD_RADIUS))")
        .expect("the header rounds its own top corners");
    let fill = header
        .find(".bg(rgb(header_bg))")
        .expect("the header paints its own fill");
    assert!(round < fill, "the radius is set before the fill it shapes");
}
