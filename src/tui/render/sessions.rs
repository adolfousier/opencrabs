//! Session list rendering
//!
//! Session manager view with navigation, renaming, and status indicators.

use super::super::app::App;
use super::utils::{format_token_count_raw, format_token_count_with_label};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Render the sessions list
pub(super) fn render_sessions(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            "  [↑↓] ",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Navigate  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Enter] ",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Select  ", Style::default().fg(Color::White)),
        Span::styled(
            "[N] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("New  ", Style::default().fg(Color::White)),
        Span::styled(
            "[R] ",
            Style::default()
                .fg(Color::Rgb(184, 134, 11))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Rename  ", Style::default().fg(Color::White)),
        Span::styled(
            "[D] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("Delete  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Esc] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("Back", Style::default().fg(Color::White)),
    ]));
    lines.push(Line::from(""));

    for (idx, session) in app.sessions.iter().enumerate() {
        let is_selected = idx == app.selected_session_index;
        let is_current = app
            .current_session
            .as_ref()
            .map(|s| s.id == session.id)
            .unwrap_or(false);

        let is_renaming = is_selected && app.session_renaming;

        let prefix = if is_selected { "  > " } else { "    " };

        let name = session.title.as_deref().unwrap_or("Untitled");
        let created = session.created_at.format("%Y-%m-%d %H:%M");

        // Format session total usage (cumulative billing tokens)
        let history_label = format_token_count_with_label(session.token_count, "total");

        // For current session, show live context window usage with actual token counts
        let context_info = if is_current {
            if let Some(input_tok) = app.last_input_tokens {
                let pct = app.context_usage_percent();
                let ctx_label = format_token_count_raw(input_tok as i32);
                let max_label = format_token_count_raw(app.context_max_tokens as i32);
                format!(" [ctx: {}/{} {:.0}%]", ctx_label, max_label, pct)
            } else {
                " [ctx: –]".to_string()
            }
        } else {
            String::new()
        };

        let current_suffix = if is_current { " *" } else { "" };

        if is_renaming {
            // Show rename input
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(Color::Rgb(184, 134, 11))),
                Span::styled(
                    format!("{}█", app.session_rename_buffer),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" - {}", created),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        } else {
            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Rgb(184, 134, 11))
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(Color::Blue)
            } else {
                Style::default().fg(Color::White)
            };

            let mut spans = vec![
                Span::styled(format!("{}{}", prefix, name), name_style),
                Span::styled(
                    format!(" - {} ", created),
                    Style::default().fg(Color::DarkGray),
                ),
            ];

            // Provider badge
            if let Some(ref prov) = session.provider_name {
                let model_label = session.model.as_deref().unwrap_or("default");
                spans.push(Span::styled(
                    format!(" [{}/{}]", prov, model_label),
                    Style::default().fg(Color::Rgb(120, 120, 120)),
                ));
            }

            // History size badge
            if session.token_count > 0 {
                spans.push(Span::styled(
                    format!(" {}", history_label),
                    Style::default().fg(Color::Rgb(100, 100, 100)),
                ));
            }

            // Status indicators for background sessions
            if app.processing_sessions.contains(&session.id) {
                let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                let frame = app.animation_frame % spinner_chars.len();
                spans.push(Span::styled(
                    format!(" {}", spinner_chars[frame]),
                    Style::default().fg(Color::Rgb(215, 100, 20)),
                ));
            } else if app.sessions_with_pending_approval.contains(&session.id) {
                spans.push(Span::styled(
                    " !",
                    Style::default()
                        .fg(Color::Rgb(215, 100, 20))
                        .add_modifier(Modifier::BOLD),
                ));
            } else if app.sessions_with_unread.contains(&session.id) {
                spans.push(Span::styled(" ●", Style::default().fg(Color::Green)));
            }

            // Context usage for current session
            if !context_info.is_empty() {
                let ctx_color = if app.last_input_tokens.is_some() {
                    let ctx_pct = app.context_usage_percent();
                    if ctx_pct > 80.0 {
                        Color::Red
                    } else if ctx_pct > 50.0 {
                        Color::Rgb(215, 100, 20)
                    } else {
                        Color::Green
                    }
                } else {
                    Color::DarkGray
                };
                spans.push(Span::styled(context_info, Style::default().fg(ctx_color)));
            }

            // Current marker
            if !current_suffix.is_empty() {
                spans.push(Span::styled(
                    current_suffix,
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ));
            }

            lines.push(Line::from(spans));
        }
    }

    let sessions = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Sessions "))
        .wrap(Wrap { trim: false });

    f.render_widget(sessions, area);
}
