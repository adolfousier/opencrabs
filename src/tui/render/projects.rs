//! Projects list rendering
//!
//! Displays projects with sessions, create/delete/assign actions.

use super::super::app::App;
use super::theme::{self, Role};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Render the projects list or detail view
pub(super) fn render_projects(f: &mut Frame, app: &App, area: Rect) {
    if app.project_detail_view {
        render_project_detail(f, app, area);
    } else {
        render_project_list(f, app, area);
    }
}

/// Render the project list view
fn render_project_list(f: &mut Frame, app: &App, area: Rect) {
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
            "View  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[N] ",
            Style::default()
                .fg(theme::role(Role::Success))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("New  ", Style::default().fg(theme::role(Role::TextPrimary))),
        Span::styled(
            "[A] ",
            Style::default()
                .fg(theme::role(Role::BlueSky))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Assign  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[D] ",
            Style::default()
                .fg(theme::role(Role::Error))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Delete  ",
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

    // Inline name input
    if app.project_name_input_active {
        lines.push(Line::from(Span::styled(
            format!("  New project name: {}▌", app.project_name_input),
            Style::default()
                .fg(theme::role(Role::Success))
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  [Enter] Confirm  [Esc] Cancel",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
        lines.push(Line::from(""));
    }

    if app.projects.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No projects yet.",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press [N] to create a project and organize your sessions.",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Projects group related sessions and their tracked files.",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "  Default directory: {}",
                crate::services::ProjectService::projects_dir().display()
            ),
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
    } else {
        let count = app.projects.len();
        lines.push(Line::from(Span::styled(
            format!("  {} project{}", count, if count == 1 { "" } else { "s" }),
            Style::default().fg(theme::role(Role::BlueSteel)),
        )));
        lines.push(Line::from(""));

        for (idx, project) in app.projects.iter().enumerate() {
            let is_selected = idx == app.selected_project_index;

            let prefix = if is_selected { "  > " } else { "    " };

            let name_style = if is_selected {
                Style::default()
                    .fg(theme::role(Role::Success))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::role(Role::TextPrimary))
            };

            let mut spans = vec![Span::styled(
                format!("{}{}", prefix, project.name),
                name_style,
            )];

            if let Some(ref desc) = project.description {
                spans.push(Span::styled(
                    format!(" — {}", desc),
                    Style::default().fg(theme::role(Role::GrayDim)),
                ));
            }

            // Show creation date
            let created = project.created_at.format("%Y-%m-%d");
            spans.push(Span::styled(
                format!("  {}", created),
                Style::default().fg(theme::role(Role::GrayDim)),
            ));

            lines.push(Line::from(spans));
        }
    }

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " Projects ",
                    Style::default()
                        .fg(theme::role(Role::Success))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(theme::role(Role::Gray))),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(para, area);
}

/// Render project detail view (sessions within a project)
fn render_project_detail(f: &mut Frame, app: &App, area: Rect) {
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
            "[U] ",
            Style::default()
                .fg(theme::role(Role::Accent))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Unassign  ",
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

    // Project name header
    if let Some(project) = app.projects.get(app.selected_project_index) {
        lines.push(Line::from(Span::styled(
            format!("  📁 {}", project.name),
            Style::default()
                .fg(theme::role(Role::Success))
                .add_modifier(Modifier::BOLD),
        )));
        if let Some(ref desc) = project.description {
            lines.push(Line::from(Span::styled(
                format!("  {}", desc),
                Style::default().fg(theme::role(Role::GrayDim)),
            )));
        }
        lines.push(Line::from(""));
    }

    if app.project_sessions.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No sessions in this project.",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Go back to /sessions, select a session, then press [A] to assign it.",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
    } else {
        let count = app.project_sessions.len();
        lines.push(Line::from(Span::styled(
            format!("  {} session{}", count, if count == 1 { "" } else { "s" }),
            Style::default().fg(theme::role(Role::BlueSteel)),
        )));
        lines.push(Line::from(""));

        for (idx, session) in app.project_sessions.iter().enumerate() {
            let is_selected = idx == app.selected_project_session_index;

            let prefix = if is_selected { "  > " } else { "    " };

            let title_style = if is_selected {
                Style::default()
                    .fg(theme::role(Role::Success))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::role(Role::TextPrimary))
            };

            let title = session.title.as_deref().unwrap_or("Untitled Session");

            let mut spans = vec![Span::styled(format!("{}{}", prefix, title), title_style)];

            // Show model
            if let Some(ref model) = session.model {
                spans.push(Span::styled(
                    format!("  [{}]", model),
                    Style::default().fg(theme::role(Role::GrayDim)),
                ));
            }

            // Show updated date
            let updated = session.updated_at.format("%Y-%m-%d %H:%M");
            spans.push(Span::styled(
                format!("  {}", updated),
                Style::default().fg(theme::role(Role::GrayDim)),
            ));

            // Token count
            if session.token_count > 0 {
                spans.push(Span::styled(
                    format!("  {} tok", format_token_count(session.token_count)),
                    Style::default().fg(theme::role(Role::GrayMid)),
                ));
            }

            lines.push(Line::from(spans));
        }
    }

    let title = if let Some(project) = app.projects.get(app.selected_project_index) {
        format!(" {} ", project.name)
    } else {
        " Project ".to_string()
    };

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(theme::role(Role::Success))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(theme::role(Role::Gray))),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(para, area);
}

/// Format token count with K/M suffix
fn format_token_count(count: i64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}
