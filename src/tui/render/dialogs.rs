//! Dialog rendering
//!
//! File picker, directory picker, model selector, usage dialog, restart dialog, and update prompt.

use super::super::app::App;
use super::theme::{self, Role};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// Render the file picker
pub(super) fn render_file_picker(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Header: directory path
    lines.push(Line::from(vec![
        Span::styled(
            "📁 File Picker",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(theme::role(Role::GrayDim))),
        Span::styled(
            app.file_picker_current_dir.to_string_lossy().to_string(),
            Style::default().fg(theme::role(Role::Accent)),
        ),
    ]));

    // Search bar
    if app.file_picker_search.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  Type to filter...",
            Style::default().fg(theme::role(Role::GrayDim)),
        )]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  🔍 ", Style::default().fg(theme::role(Role::GrayDim))),
            Span::styled(
                &app.file_picker_search,
                Style::default().fg(theme::role(Role::Accent)),
            ),
        ]));
    }
    lines.push(Line::from(""));

    let filtered = &app.file_picker_filtered;

    // Calculate visible range
    let visible_items = (area.height as usize).saturating_sub(7);
    let start = app.file_picker_scroll_offset;
    let end = (start + visible_items).min(filtered.len());

    // Render filtered file list
    for (display_idx, &file_idx) in filtered.iter().enumerate().skip(start).take(end - start) {
        let path = &app.file_picker_files[file_idx];
        let is_selected = display_idx == app.file_picker_selected;
        let is_dir = path.is_dir();

        let icon = if path.ends_with("..") {
            "📂 .."
        } else if is_dir {
            "📂"
        } else {
            "📄"
        };

        // In recursive mode, show the path relative to the working dir so
        // matches deep in the tree are disambiguated (e.g. `src/tui/render/dialogs.rs`
        // vs just `dialogs.rs`).
        let label = if app.file_picker_recursive {
            path.strip_prefix(&app.working_directory)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string()
                })
        } else {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string()
        };

        let style = if is_selected {
            Style::default()
                .fg(theme::role(Role::Ink))
                .bg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD)
        } else if is_dir {
            Style::default().fg(theme::role(Role::Gray))
        } else {
            Style::default().fg(theme::role(Role::TextPrimary))
        };

        let prefix = if is_selected { "▶ " } else { "  " };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(format!("{} {}", icon, label), style),
        ]));
    }

    // Scroll indicator
    if filtered.len() > visible_items {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("Showing {}-{} of {} files", start + 1, end, filtered.len()),
            Style::default().fg(theme::role(Role::GrayDim)),
        )]));
    }

    // Help text
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "[↑↓]",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Navigate  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[Enter]",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Select  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[Esc]",
            Style::default()
                .fg(theme::role(Role::Error))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Cancel",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
    ]));

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::role(Role::Gray)))
                .title(Span::styled(
                    " Select a file ",
                    Style::default()
                        .fg(theme::role(Role::Gray))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(widget, area);
}

/// Render directory picker (reuses file picker state, dirs only)
pub(super) fn render_directory_picker(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            "📂 Directory Picker",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(theme::role(Role::GrayDim))),
        Span::styled(
            app.file_picker_current_dir.to_string_lossy().to_string(),
            Style::default().fg(theme::role(Role::Accent)),
        ),
    ]));
    lines.push(Line::from(""));

    let visible_items = (area.height as usize).saturating_sub(6);
    let start = app.file_picker_scroll_offset;
    let end = (start + visible_items).min(app.file_picker_files.len());

    for (idx, path) in app
        .file_picker_files
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let is_selected = idx == app.file_picker_selected;

        let icon = if path.ends_with("..") {
            "📂 .."
        } else {
            "📂"
        };

        let filename = if path.ends_with("..") {
            ".."
        } else {
            path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
        };

        let style = if is_selected {
            Style::default()
                .fg(theme::role(Role::Ink))
                .bg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::role(Role::Gray))
        };

        let prefix = if is_selected { "▶ " } else { "  " };

        let display = if path.ends_with("..") {
            icon.to_string()
        } else {
            format!("{} {}", icon, filename)
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(display, style),
        ]));
    }

    if app.file_picker_files.len() > visible_items {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "Showing {}-{} of {}",
                start + 1,
                end,
                app.file_picker_files.len()
            ),
            Style::default().fg(theme::role(Role::GrayDim)),
        )]));
    }

    // Help text
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "[↑↓]",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Navigate  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[Enter]",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Open  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[Space/Tab]",
            Style::default()
                .fg(theme::role(Role::TealBright))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Select here  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[.]",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " {} hidden  ",
                if app.file_picker_show_hidden {
                    "Hide"
                } else {
                    "Show"
                }
            ),
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[Esc]",
            Style::default()
                .fg(theme::role(Role::Error))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Cancel",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
    ]));

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::role(Role::Gray)))
                .title(Span::styled(
                    " Change working directory ",
                    Style::default()
                        .fg(theme::role(Role::Gray))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(widget, area);
}

/// Render restart confirmation dialog
pub(super) fn render_restart_dialog(f: &mut Frame, app: &App, area: Rect) {
    let status = app.rebuild_status.as_deref().unwrap_or("Build successful");

    let dialog_height = 8u16;
    let dialog_width = 50u16.min(area.width.saturating_sub(4));

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(dialog_height),
            Constraint::Percentage(40),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min((area.width.saturating_sub(dialog_width)) / 2),
            Constraint::Length(dialog_width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    let dialog_area = horizontal[1];
    f.render_widget(Clear, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", status),
            Style::default()
                .fg(theme::role(Role::AccentTeal))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Restart with new binary?"),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [Enter] ",
                Style::default()
                    .fg(theme::role(Role::AccentTeal))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Restart  "),
            Span::styled(
                "[Esc] ",
                Style::default()
                    .fg(theme::role(Role::Error))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]),
    ];

    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(" Rebuild Complete ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::role(Role::AccentTeal))),
    );
    f.render_widget(dialog, dialog_area);
}

/// Render update prompt dialog
pub(super) fn render_update_dialog(f: &mut Frame, app: &App, area: Rect) {
    let version = app.update_available_version.as_deref().unwrap_or("unknown");

    let dialog_height = 8u16;
    let dialog_width = 55u16.min(area.width.saturating_sub(4));

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(dialog_height),
            Constraint::Percentage(40),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min((area.width.saturating_sub(dialog_width)) / 2),
            Constraint::Length(dialog_width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    let dialog_area = horizontal[1];
    f.render_widget(Clear, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  v{} -> v{}", crate::VERSION, version),
            Style::default()
                .fg(theme::role(Role::AccentTeal))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Update now?"),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [Enter] ",
                Style::default()
                    .fg(theme::role(Role::AccentTeal))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Update  "),
            Span::styled(
                "[Esc] ",
                Style::default()
                    .fg(theme::role(Role::Error))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Skip"),
        ]),
    ];

    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(" Update Available ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::role(Role::AccentTeal))),
    );
    f.render_widget(dialog, dialog_area);
}
