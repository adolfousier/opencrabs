//! The one-row app title bar shown above the full-screen modes.

use super::theme::{self, Role};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Split 1 row off the top of an area for the app title bar.
pub(super) fn split_title_area(area: Rect) -> (Rect, Rect) {
    let title_height = 1u16; // title only
    let used_title = title_height.min(area.height);
    let title_area = Rect {
        height: used_title,
        ..area
    };
    // Clamp content_area.y so it never lands past the buffer when area is
    // very small (e.g. height == 0 during a resize). Without the clamp,
    // `area.y + title_height` can walk one row past the valid buffer and
    // downstream renders panic on the first cell write.
    let content_area = Rect {
        y: area.y.saturating_add(used_title),
        height: area.height.saturating_sub(used_title),
        ..area
    };
    (title_area, content_area)
}

/// Render the app name header used on Sessions, Help, and Settings screens.
/// Carries the running build version on the same line so users can see which
/// version they're on at a glance (#696).
pub(super) fn render_app_title(f: &mut Frame, area: Rect) {
    let para = Paragraph::new(vec![Line::from(vec![
        Span::styled(
            " 🦀 OpenCrabs AI Agent",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(theme::role(Role::GrayDim)),
        ),
    ])]);
    f.render_widget(para, area);
}
