//! The one modal chrome recipe (#1775).
//!
//! `mission_control/detail_popup.rs` already rendered the look the ticket
//! wants (Clear, centered rect, rounded `border_set`, accent title). This
//! module generalizes it: every dialog routes through [`modal_block`] and
//! [`centered`], so the modal edge is the dedicated [`Role::BorderModal`]
//! color everywhere — distinct from regular panel borders so the dialog
//! pops — and no surface can forget the Clear or square its corners by
//! accident.

use super::theme::{self, Role};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders};

/// A centered rect of the given size, clamped to `area` (and to `area`'s
/// own dimensions when the request does not fit).
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    )
}

/// The canonical modal block: rounded corners, border in the dedicated
/// modal role, title in the caller's accent.
///
/// The caller still renders `Clear` on the popup rect first; the chrome
/// test pins both halves of the recipe.
pub fn modal_block(title: &str, title_accent: Color) -> Block<'static> {
    Block::default()
        .title(Span::styled(
            title.to_string(),
            Style::default()
                .fg(title_accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme::role(Role::BorderModal)))
}

/// The row just inside a bordered popup's bottom edge — where the shared
/// footer lives ("bottom, inside the border", locked decision).
pub fn footer_row(popup: Rect) -> Rect {
    Rect {
        x: popup.x + 1,
        y: popup.y + popup.height.saturating_sub(2),
        width: popup.width.saturating_sub(2),
        height: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// centered() centers and clamps.
    #[test]
    fn centered_rects_center_and_clamp() {
        let area = Rect::new(0, 0, 100, 40);
        let r = centered(area, 60, 20);
        assert_eq!((r.x, r.y, r.width, r.height), (20, 10, 60, 20));
        let clamped = centered(area, 200, 200);
        assert_eq!(
            (clamped.x, clamped.y, clamped.width, clamped.height),
            (0, 0, 100, 40)
        );
    }

    /// footer_row returns the inner bottom row.
    #[test]
    fn footer_row_is_inner_bottom() {
        let popup = Rect::new(10, 5, 40, 12);
        let r = footer_row(popup);
        assert_eq!((r.x, r.y, r.width, r.height), (11, 15, 38, 1));
    }

    /// The chrome recipe: rounded corners, the modal border role, an
    /// accent bold title. Asserted via buffer inspection because the
    /// border style only materializes when rendered.
    #[test]
    fn modal_block_is_rounded_and_uses_modal_role() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::widgets::Paragraph;

        let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();
        let mut expected_border = None;
        terminal
            .draw(|f| {
                let area = f.area();
                let popup = centered(area, 28, 6);
                f.render_widget(ratatui::widgets::Clear, popup);
                let block = modal_block(" Test ", theme::role(Role::Accent));
                // Capture the role resolution in the same draw pass that
                // renders the border, so the assertion is against the
                // theme that was actually active at render time.
                expected_border = Some(theme::role(Role::BorderModal));
                f.render_widget(Paragraph::new("").block(block), popup);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // Top-left corner of the popup (rounded set uses ╭).
        let corner = buffer[(1, 1)].symbol().to_string();
        assert_eq!(corner, "╭", "modal corners must be rounded, got {corner}");
        // Border cell carries the BorderModal role color, and it differs
        // from the regular border gray so dialogs pop (#1775). Probed at
        // x=20, clear of the left-aligned " Test " title.
        let border_cell = &buffer[(20, 1)];
        assert_eq!(
            border_cell.fg,
            expected_border.expect("role captured during draw"),
            "modal border must use Role::BorderModal"
        );
        assert_ne!(
            border_cell.fg,
            theme::role(Role::GrayDim),
            "modal border must be distinct from dim panel borders"
        );
    }
}
