//! Session list rendering
//!
//! Session manager view with navigation, renaming, and status indicators.

use super::super::app::App;
use super::palette;
use super::theme::{self, Role};
use super::utils::{format_token_count_raw, format_token_count_with_label};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// On-brand palette for project badges — variations of orange, cyan, white,
/// and blue. Each project is mapped to one entry deterministically so the same
/// project always shows the same colour and different projects stay
/// distinguishable in the session list.
const PROJECT_BADGE_COLORS: &[Color] = palette::PROJECT_BADGE_COLORS;

/// Pick a stable badge colour for `project_id` from [`PROJECT_BADGE_COLORS`].
fn project_badge_color(project_id: uuid::Uuid) -> Color {
    let idx = (project_id.as_u128() % PROJECT_BADGE_COLORS.len() as u128) as usize;
    PROJECT_BADGE_COLORS[idx]
}

/// Render the sessions list
pub(super) fn render_sessions(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Compute filtered session indices based on search
    let visible_indices: Vec<usize> = if app.session_search.is_empty() {
        (0..app.sessions.len()).collect()
    } else {
        let search = app.session_search.to_lowercase();
        app.sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                let name = s.title.as_deref().unwrap_or("New Chat").to_lowercase();
                name.contains(&search)
            })
            .map(|(i, _)| i)
            .collect()
    };

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
            "Select  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[N] ",
            Style::default()
                .fg(theme::role(Role::AccentTeal))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("New  ", Style::default().fg(theme::role(Role::TextPrimary))),
        Span::styled(
            "[R] ",
            Style::default()
                .fg(theme::role(Role::Accent))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Rename  ",
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
            "[F] ",
            Style::default()
                .fg(theme::role(Role::AccentTeal))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Files  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[P] ",
            Style::default()
                .fg(theme::role(Role::Gray))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Projects  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[|] ",
            Style::default()
                .fg(theme::role(Role::Success))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Split H  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[_] ",
            Style::default()
                .fg(theme::role(Role::Success))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Split V  ",
            Style::default().fg(theme::role(Role::TextPrimary)),
        ),
        Span::styled(
            "[/] ",
            Style::default()
                .fg(theme::role(Role::AccentTeal))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Search  ",
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

    // Show hint when a pane is waiting for session assignment (just split)
    let has_unassigned = app
        .pane_manager
        .focused_pane()
        .is_some_and(|p| p.session_id.is_none());
    if has_unassigned {
        lines.push(Line::from(Span::styled(
            "  Select a session for the new pane (or N for new)",
            Style::default()
                .fg(theme::role(Role::Success))
                .add_modifier(Modifier::BOLD),
        )));
    }

    // Show assign mode banner when assigning sessions to a project
    if let Some(project_id) = app.assigning_to_project {
        let project_name = app
            .project_name_cache
            .get(&project_id)
            .map(|s| s.as_str())
            .unwrap_or("project");
        lines.push(Line::from(vec![
            Span::styled(
                format!("  ASSIGNING TO: {} ", project_name),
                Style::default()
                    .fg(theme::role(Role::Warning))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "[Enter] assign  [Esc] done",
                Style::default().fg(theme::role(Role::Gray)),
            ),
        ]));
    }
    lines.push(Line::from(""));

    // Search input line
    if app.session_search_active {
        lines.push(Line::from(vec![
            Span::styled("  🔍 ", Style::default().fg(theme::role(Role::AccentTeal))),
            Span::styled(
                format!("{}█", app.session_search),
                Style::default()
                    .fg(theme::role(Role::TextPrimary))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  [type to filter]  [Enter] open  [Esc] clear",
                Style::default().fg(theme::role(Role::GrayDim)),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // Track header line count for viewport scroll calculation
    let header_line_count = lines.len();

    if visible_indices.is_empty() && !app.session_search.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No matching sessions",
            Style::default().fg(theme::role(Role::GrayDim)),
        )));
    }

    for (display_idx, &session_idx) in visible_indices.iter().enumerate() {
        let session = &app.sessions[session_idx];
        let is_selected = display_idx == app.selected_session_index;
        let is_current = app
            .current_session
            .as_ref()
            .map(|s| s.id == session.id)
            .unwrap_or(false);

        let is_renaming = is_selected && app.session_renaming;

        let prefix = if is_selected { "  > " } else { "    " };

        let name = session.title.as_deref().unwrap_or("New Chat");
        // Show LAST ACTIVITY, not creation. The list is ordered by
        // `updated_at DESC` (`list_sessions`), so printing `created_at` made
        // the rows look mis-sorted: a session last used minutes ago but opened
        // in March rendered "2026-03-26" above one created yesterday, and the
        // order looked arbitrary even though it was correct. The column now
        // shows the same field the sort uses, so position and timestamp agree.
        let created = session.updated_at.format("%Y-%m-%d %H:%M");

        // Format session total usage (cumulative billing tokens)
        let history_label = format_token_count_with_label(session.token_count, "total");

        // For current session, show live context window usage with actual token counts
        let context_info = if is_current {
            if let Some(input_tok) = app.last_input_tokens {
                let pct = app.context_usage_percent();
                let ctx_label = format_token_count_raw(input_tok as i64);
                let max_label = format_token_count_raw(app.context_max_tokens as i64);
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
                Span::styled(prefix, Style::default().fg(theme::role(Role::Accent))),
                Span::styled(
                    format!("{}█", app.session_rename_buffer),
                    Style::default()
                        .fg(theme::role(Role::TextPrimary))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" - {}", created),
                    Style::default().fg(theme::role(Role::GrayDim)),
                ),
            ]));
        } else {
            // When in assign mode, highlight sessions already assigned to
            // target project in green so the user has clear visual feedback.
            let is_assigned_to_target = app
                .assigning_to_project
                .is_some_and(|pid| session.project_id == Some(pid));

            let name_style = if is_assigned_to_target {
                Style::default()
                    .fg(theme::role(Role::Success))
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(theme::role(Role::Accent))
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(theme::role(Role::Gray))
            } else {
                Style::default().fg(theme::role(Role::TextPrimary))
            };

            let mut spans = vec![
                Span::styled(format!("{}{}", prefix, name), name_style),
                Span::styled(
                    format!(" - {} ", created),
                    Style::default().fg(theme::role(Role::GrayDim)),
                ),
            ];

            // Provider badge
            if let Some(ref prov) = session.provider_name {
                let model_label = session.model.as_deref().unwrap_or("default");
                spans.push(Span::styled(
                    format!(" [{}/{}]", prov, model_label),
                    Style::default().fg(theme::role(Role::Gray)),
                ));
            }

            // Project badge
            if let Some(pid) = session.project_id {
                let project_name = app
                    .project_name_cache
                    .get(&pid)
                    .map(|s| s.as_str())
                    .unwrap_or("?");
                spans.push(Span::styled(
                    format!(" {{.{}}}", project_name),
                    Style::default().fg(project_badge_color(pid)),
                ));
            }

            // Working directory badge
            if let Some(ref wd) = session.working_directory {
                let home_dir = dirs::home_dir()
                    .map(|h| h.to_string_lossy().to_string())
                    .unwrap_or_default();
                let short = if !home_dir.is_empty() && wd.starts_with(&home_dir) {
                    format!("~{}", &wd[home_dir.len()..])
                } else {
                    wd.clone()
                };
                spans.push(Span::styled(
                    format!(" {}", short),
                    Style::default().fg(theme::role(Role::BlueSteel)),
                ));
                // Git branch badge (cyan)
                if let Some(branch) =
                    crate::utils::git_branch::current_branch(std::path::Path::new(wd))
                {
                    spans.push(Span::styled(
                        format!(" ({branch})"),
                        Style::default().fg(theme::role(Role::AccentTeal)),
                    ));
                }
            }

            // History size badge
            if session.token_count > 0 {
                spans.push(Span::styled(
                    format!(" {}", history_label),
                    Style::default().fg(theme::role(Role::GrayMid)),
                ));
            }

            // Status indicators for background sessions
            if app.processing_sessions.contains(&session.id) {
                let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                let frame = app.animation_frame % spinner_chars.len();
                spans.push(Span::styled(
                    format!(" {}", spinner_chars[frame]),
                    Style::default().fg(theme::role(Role::Accent)),
                ));
            } else if app.sessions_with_pending_approval.contains(&session.id) {
                spans.push(Span::styled(
                    " !",
                    Style::default()
                        .fg(theme::role(Role::Accent))
                        .add_modifier(Modifier::BOLD),
                ));
            } else if app.sessions_with_unread.contains(&session.id) {
                spans.push(Span::styled(
                    " ●",
                    Style::default().fg(theme::role(Role::AccentTeal)),
                ));
            }

            // Context usage for current session
            if !context_info.is_empty() {
                let ctx_color = if app.last_input_tokens.is_some() {
                    let ctx_pct = app.context_usage_percent();
                    if ctx_pct > 80.0 {
                        theme::role(Role::Error)
                    } else if ctx_pct > 50.0 {
                        theme::role(Role::Accent)
                    } else {
                        theme::role(Role::AccentTeal)
                    }
                } else {
                    theme::role(Role::GrayDim)
                };
                spans.push(Span::styled(context_info, Style::default().fg(ctx_color)));
            }

            // Current marker
            if !current_suffix.is_empty() {
                spans.push(Span::styled(
                    current_suffix,
                    Style::default()
                        .fg(theme::role(Role::Gray))
                        .add_modifier(Modifier::BOLD),
                ));
            }

            // Pane indicator — show which pane this session is already in
            if app.pane_manager.is_split() {
                let pane_ids = app.pane_manager.pane_ids_in_order();
                if let Some(pos) = pane_ids.iter().position(|pid| {
                    app.pane_manager
                        .get(*pid)
                        .is_some_and(|p| p.session_id == Some(session.id))
                }) {
                    spans.push(Span::styled(
                        format!(" [pane {}]", pos + 1),
                        Style::default().fg(theme::role(Role::Success)),
                    ));
                }
            }

            lines.push(Line::from(spans));
        }
    }

    // Viewport scroll: keep the selected session visible
    let visible_height = area.height.saturating_sub(2) as usize; // inside borders
    let selected_line = header_line_count + app.selected_session_index;
    let scroll = if selected_line >= visible_height {
        (selected_line.saturating_sub(visible_height) + 1) as u16
    } else {
        0u16
    };

    let sessions = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Sessions "))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    f.render_widget(sessions, area);
}
