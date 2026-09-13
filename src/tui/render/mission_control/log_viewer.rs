//! Full-screen log viewer (#1528).
//!
//! Renders the entries `logging::reader` parsed out of the current log file's
//! tail. Two rendering decisions matter more than they look:
//!
//! * A continuation line is drawn under its own entry, unstyled and indented.
//!   Colouring it by the parent's level would make a 40-line JSON payload a
//!   wall of red; leaving it unindented would make it indistinguishable from
//!   the next entry.
//! * Level colour comes from `theme::Role`, not raw ANSI, so the viewer
//!   follows `/theme set` like every other screen.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::theme;
use crate::logging::reader::{LogEntry, LogLevel};
use crate::tui::app::App;
use crate::tui::render::theme::{self as rtheme, Role};

/// Colour for a level badge. ERROR and WARN carry the palette's own error and
/// warning roles; the rest stay quiet, because a log where every row shouts is
/// a log nobody reads.
fn level_color(level: LogLevel) -> Color {
    match level {
        LogLevel::Error => rtheme::role(Role::Error),
        LogLevel::Warn => rtheme::role(Role::Warning),
        LogLevel::Info => rtheme::role(Role::AccentTeal),
        LogLevel::Debug => rtheme::role(Role::TextDim),
        LogLevel::Trace => rtheme::role(Role::GrayDim),
    }
}

/// Shorten a module path to its last two segments.
///
/// `opencrabs::channels::whatsapp::handler` becomes `whatsapp::handler`. The
/// leading segments are the same on almost every line, so spending 30 columns
/// on them costs the message its width for no information.
fn short_target(target: &str) -> String {
    let parts: Vec<&str> = target.split("::").collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_string(),
        n => format!("{}::{}", parts[n - 2], parts[n - 1]),
    }
}

/// Time of day from an RFC3339 stamp. The date is already in the file name,
/// so repeating it on every row would be 11 wasted columns.
fn clock(timestamp: &str) -> &str {
    timestamp
        .split_once('T')
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.get(..12))
        .unwrap_or(timestamp)
}

/// Lines for one entry: the header row, then any continuation lines.
fn entry_lines(entry: &LogEntry) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} ", clock(&entry.timestamp)),
            Style::default().fg(theme::text_dim()),
        ),
        Span::styled(
            format!("{:<5} ", entry.level.label()),
            Style::default()
                .fg(level_color(entry.level))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ", short_target(&entry.target)),
            Style::default().fg(theme::teal()),
        ),
        Span::styled(
            entry.message.clone(),
            Style::default().fg(theme::text_primary()),
        ),
    ])];
    // Indented and dim: body text belongs to the entry above it, and must not
    // read as a new one.
    for cont in &entry.continuation {
        lines.push(Line::from(Span::styled(
            format!("    {cont}"),
            Style::default().fg(theme::text_dim()),
        )));
    }
    lines
}

/// Draw the viewer over the whole Mission Control area.
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(area);
    let (body_area, bar_area) = (rows[0], rows[1]);

    // Record the height before borrowing the viewer immutably, so the key
    // handler can clamp scrolling to what actually fits (#1527 did the same
    // for the help screen).
    let viewport = body_area.height.saturating_sub(2) as usize;
    if let Some(viewer) = app.mc.log_viewer.as_mut() {
        viewer.viewport_rows = viewport;
    }
    let Some(viewer) = app.mc.log_viewer.as_ref() else {
        return;
    };

    frame.render_widget(Clear, area);

    let visible = viewer.visible();
    let mut lines: Vec<Line> = Vec::new();
    // A failed read says why. Rendering an empty pane instead would be
    // indistinguishable from a log with nothing in it.
    if let Some(err) = &viewer.error {
        lines.push(Line::from(Span::styled(
            format!("  {err}"),
            Style::default().fg(rtheme::role(Role::Error)),
        )));
    } else if visible.is_empty() {
        lines.push(Line::from(Span::styled(
            if viewer.entries.is_empty() {
                "  This log file is empty.".to_string()
            } else {
                format!(
                    "  No entry matches the current filters ({} hidden).",
                    viewer.entries.len()
                )
            },
            Style::default().fg(theme::text_dim()),
        )));
    } else {
        for entry in &visible {
            lines.extend(entry_lines(entry));
        }
    }

    let title = format!(
        " 📜 {} — {} shown of {} ",
        viewer.file_name(),
        visible.len(),
        viewer.entries.len()
    );
    let body = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(theme::orange())
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(theme::border_activity_focus())),
        )
        .scroll((viewer.scroll.min(viewer.max_scroll()) as u16, 0));
    frame.render_widget(body, body_area);

    draw_bar(frame, viewer, bar_area);
}

/// Two-row status bar: active filters on top, keys below.
fn draw_bar(
    frame: &mut Frame,
    viewer: &crate::tui::app::mission_control::log_viewer::LogViewerState,
    area: Rect,
) {
    if area.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let mut status = vec![
        Span::styled(" level ≥ ", theme::dim()),
        Span::styled(
            viewer.min_level.label(),
            Style::default()
                .fg(level_color(viewer.min_level))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    // The search box shows a caret while it has focus, so it is obvious where
    // the next keystroke lands.
    if viewer.search_active || !viewer.search.is_empty() {
        status.push(Span::styled("   filter ", theme::dim()));
        status.push(Span::styled(
            if viewer.search_active {
                format!("{}▏", viewer.search)
            } else {
                viewer.search.clone()
            },
            Style::default()
                .fg(theme::orange())
                .add_modifier(Modifier::BOLD),
        ));
    }
    if viewer.files.len() > 1 {
        status.push(Span::styled(
            format!("   file {}/{}", viewer.file_index + 1, viewer.files.len()),
            theme::dim(),
        ));
    }

    let bold =
        |s: &'static str| Span::styled(s, theme::help_bar_style().add_modifier(Modifier::BOLD));
    let keys = Line::from(vec![
        bold(" ↑↓/jk"),
        Span::styled(": scroll  ", theme::dim()),
        bold("g/G"),
        Span::styled(": top/end  ", theme::dim()),
        bold("e/w/i/d"),
        Span::styled(": level  ", theme::dim()),
        bold("/"),
        Span::styled(": search  ", theme::dim()),
        bold("[ ]"),
        Span::styled(": prev/next day  ", theme::dim()),
        bold("Esc"),
        Span::styled(": back", theme::dim()),
    ]);

    frame.render_widget(Paragraph::new(Line::from(status)), rows[0]);
    if rows.len() > 1 {
        frame.render_widget(Paragraph::new(keys), rows[1]);
    }
}
