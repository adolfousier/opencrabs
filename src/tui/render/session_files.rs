//! Session files list rendering
//!
//! Displays files tracked for a session with navigation and actions.

use super::super::app::App;
use super::theme::{self, Role};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Render the session files list
pub(super) fn render_session_files(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Key hints bar
    lines.push(Line::from(vec![
        Span::styled(
            "  [↑↓] ",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Navigate  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[Enter] ",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Open  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[D] ",
            Style::default()
                .fg(theme::role(Role::Error))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Remove  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[O] ",
            Style::default()
                .fg(theme::role(Role::Accent))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Folder  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[Esc] ",
            Style::default()
                .fg(theme::role(Role::Error))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Back", Style::default().fg(theme::role(Role::TextPrimary))),
    ]));

    lines.push(Line::from(""));

    if app.session_files.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No files tracked for this session.",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Files are auto-tracked when the agent writes or edits them,",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
        lines.push(Line::from(Span::styled(
            "  or when you paste images from the clipboard.",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
    } else {
        let count = app.session_files.len();
        lines.push(Line::from(Span::styled(
            format!(
                "  {} file{} tracked",
                count,
                if count == 1 { "" } else { "s" }
            ),
            Style::default().fg(theme::role(Role::BlueSteel)),
        )));
        lines.push(Line::from(""));

        for (idx, file) in app.session_files.iter().enumerate() {
            let is_selected = idx == app.selected_file_index;

            let prefix = if is_selected { "  > " } else { "    " };

            let path_display = file.path.display().to_string();
            // Collapse home directory
            let home_dir = dirs::home_dir()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_default();
            let short_path = if !home_dir.is_empty() && path_display.starts_with(&home_dir) {
                format!("~{}", &path_display[home_dir.len()..])
            } else {
                path_display
            };

            let name_style = if is_selected {
                Style::default()
                    .fg(theme::role(Role::Accent))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::role(Role::TextPrimary))
            };

            let mut spans = vec![Span::styled(
                format!("{}{}", prefix, short_path),
                name_style,
            )];

            // Show size if available
            if let Some(size) = file.size {
                let size_str = format_size(size);
                spans.push(Span::styled(
                    format!(" ({})", size_str),
                    Style::default().fg(theme::role(Role::GrayMid)),
                ));
            }

            // Show created date
            let created = file.created_at.format("%Y-%m-%d %H:%M");
            spans.push(Span::styled(
                format!(" {}", created),
                Style::default().fg(theme::role(Role::GrayDim)),
            ));

            // Content indicator
            if file.content.is_some() {
                spans.push(Span::styled(
                    " [stored]",
                    Style::default().fg(theme::role(Role::AccentTeal)),
                ));
            }

            lines.push(Line::from(spans));
        }
    }

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " Session Files ",
                    Style::default()
                        .fg(theme::role(Role::Accent))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(theme::role(Role::Gray))),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(para, area);
}

/// Format file size in human-readable form
fn format_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
