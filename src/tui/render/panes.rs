//! Split pane rendering — draws pane borders, labels, and delegates chat rendering.

use crate::tui::app::App;
use crate::tui::pane::PaneId;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
};

/// Render a single inactive (non-focused) pane.
/// Shows the session name and a hint to switch focus.
pub(super) fn render_inactive_pane(f: &mut Frame, app: &App, pane_id: PaneId, area: Rect) {
    let pane = match app.pane_manager.get(pane_id) {
        Some(p) => p,
        None => return,
    };

    let session_label = pane
        .session_id
        .and_then(|sid| {
            app.sessions
                .iter()
                .find(|s| s.id == sid)
                .map(|s| {
                    s.title
                        .clone()
                        .unwrap_or_else(|| format!("Session {}", &s.id.to_string()[..8]))
                })
        })
        .unwrap_or_else(|| "No session".to_string());

    let is_processing = pane
        .session_id
        .map(|sid| app.processing_sessions.contains(&sid))
        .unwrap_or(false);

    let status = if is_processing {
        " [processing...]"
    } else {
        ""
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            format!(" {} {} ", session_label, status),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height > 0 && inner.width > 0 {
        let hint = Paragraph::new(Line::from(Span::styled(
            "Ctrl+Tab to switch focus",
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(hint, inner);
    }
}

/// Render the focused pane's border decoration.
/// Returns the inner area (content area inside the border) for the caller to render chat into.
pub(super) fn focused_pane_border(f: &mut Frame, app: &App, area: Rect) -> Rect {
    let pane = match app.pane_manager.focused_pane() {
        Some(p) => p,
        None => return area,
    };

    let session_label = pane
        .session_id
        .and_then(|sid| {
            app.sessions
                .iter()
                .find(|s| s.id == sid)
                .map(|s| {
                    s.title
                        .clone()
                        .unwrap_or_else(|| format!("Session {}", &s.id.to_string()[..8]))
                })
        })
        .unwrap_or_else(|| "No session".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 200, 120)))
        .title(Span::styled(
            format!(" {} ", session_label),
            Style::default()
                .fg(Color::Rgb(80, 200, 120))
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(0));

    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}
