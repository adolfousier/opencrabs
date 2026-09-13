//! Help, plan mode, and settings rendering
//!
//! Help screen, plan mode view, plan mode help bar, and settings screen.

use super::super::app::App;
use super::super::app::help_catalog;
use super::theme::{self, Role};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Render the help screen
pub(super) fn render_help(f: &mut Frame, app: &mut App, area: Rect) {
    // Helper to build a "key → description" line
    fn kv<'a>(key: &'a str, desc: &'a str, key_color: Color) -> Line<'a> {
        Line::from(vec![
            Span::styled(
                format!(" {:<14}", key),
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default().fg(Color::DarkGray)),
            Span::styled(desc, Style::default().fg(Color::Reset)),
        ])
    }

    fn section_header(title: &str) -> Line<'_> {
        Line::from(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(theme::role(Role::Accent))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ))
    }

    // Split into two columns
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // ── LEFT COLUMN ──
    let cyan = Color::Cyan;

    let version_line = format!("v{}", env!("CARGO_PKG_VERSION"));
    // Session-aware: the global name lies when the loaded session runs a
    // per-session provider swap or a sticky fallback (#1464).
    let provider_name = app.provider_name_for_current_session();
    let model_name = app.provider_model_for_current_session();

    let mut left = vec![
        Line::from(""),
        section_header("ABOUT"),
        kv("Version", &version_line, cyan),
        kv("Provider", &provider_name, cyan),
        kv("Model", &model_name, cyan),
        Line::from(""),
        section_header("GLOBAL"),
        kv("Ctrl+C", "Clear input / quit (2x)", cyan),
        kv("Ctrl+N", "New session", cyan),
        kv("Ctrl+L", "List sessions", cyan),
        kv("Ctrl+K", "Clear session", cyan),
        Line::from(""),
        section_header("CHAT"),
        kv("Enter", "Send message", cyan),
        kv("Shift+Enter", "New line", cyan),
        kv("Ctrl+J", "New line (alt)", cyan),
        kv("Escape (x2)", "Cancel / abort immediately", cyan),
        kv("Page Up/Down", "Scroll history", cyan),
        kv("@", "File picker", cyan),
        Line::from(""),
        section_header("INPUT EDITING"),
        kv("↑ / ↓", "Line nav / start-end / history", cyan),
        kv("← / →", "Move cursor", cyan),
        kv("Ctrl/Alt+←→", "Jump word", cyan),
        kv("Home / End", "Start / end of line", cyan),
        kv("Ctrl+W", "Delete word (vim)", cyan),
        kv("Ctrl+U", "Delete to line start (vim)", cyan),
        Line::from(""),
        section_header("SLASH COMMANDS"),
    ];

    // Commands come from the catalogue collected on entry (#1530) rather than
    // from a directory walk and a TOML parse per frame. The SLASH COMMANDS
    // header is already on `left` above; every other section prints its own.
    // Hoisted: `kv` borrows its arguments into the returned `Line`, so a
    // summary built inside the loop would be dropped before the frame draws.
    let skills_summary = format!(
        "Browse {} skills",
        help_catalog::section_rows(&app.help_catalog, help_catalog::HelpSection::Skill).len()
    );
    for section in help_catalog::HelpSection::all() {
        let rows = help_catalog::section_rows(&app.help_catalog, section);
        if rows.is_empty() {
            continue;
        }
        if section != help_catalog::HelpSection::BuiltIn {
            left.push(Line::from(""));
            left.push(section_header(section.title()));
        }
        if section == help_catalog::HelpSection::Skill {
            left.push(kv("/skills", &skills_summary, cyan));
        }
        for row in rows {
            left.push(kv(&row.name, &row.description, cyan));
        }
    }

    left.extend([
        Line::from(""),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [↑↓ PgUp/Dn]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Scroll  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Back", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" 📖 ", Style::default().fg(theme::role(Role::Accent))),
            Span::styled(
                "docs.opencrabs.com",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled(
                "  Official documentation",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
    ]);

    // ── RIGHT COLUMN ──
    let right = vec![
        Line::from(""),
        section_header("SESSIONS"),
        kv("↑ / ↓", "Navigate", cyan),
        kv("Enter", "Load session", cyan),
        kv("N", "New session", cyan),
        kv("R", "Rename", cyan),
        kv("D", "Delete", cyan),
        kv("/", "Search sessions", cyan),
        kv("F", "Session files", cyan),
        kv("P", "Projects", cyan),
        kv("Esc", "Back to chat", cyan),
        Line::from(""),
        section_header("FILE ARTIFACTS"),
        kv("↑ / ↓", "Navigate files", cyan),
        kv("Enter", "Open file", cyan),
        kv("D", "Remove tracking", cyan),
        kv("O", "Open containing folder", cyan),
        kv("Esc", "Back to sessions", cyan),
        Line::from(""),
        section_header("PROJECTS"),
        kv("↑ / ↓", "Navigate projects", cyan),
        kv("Enter", "View sessions", cyan),
        kv("N", "New project", cyan),
        kv("A", "Assign current session", cyan),
        kv("D", "Delete project", cyan),
        kv("U", "Unassign session (detail)", cyan),
        kv("Esc", "Back / exit", cyan),
        Line::from(""),
        section_header("TOOL APPROVAL"),
        kv("↑ / ↓", "Navigate options", cyan),
        kv("Enter", "Confirm selection", cyan),
        kv("D / Esc", "Deny", cyan),
        kv("V", "Toggle details", cyan),
        Line::from(""),
        section_header("SPLIT PANES (from Sessions)"),
        kv("| (in sessions)", "Split horizontal (L|R)", cyan),
        kv("_ (in sessions)", "Split vertical (T/B)", cyan),
        kv("Tab", "Cycle pane focus", cyan),
        kv("Ctrl+X", "Close pane", cyan),
        Line::from(""),
        section_header("FEATURES"),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Markdown & Syntax Highlighting",
                Style::default().fg(Color::Reset),
            ),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Multi-line Input & Streaming",
                Style::default().fg(Color::Reset),
            ),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Session Management & History",
                Style::default().fg(Color::Reset),
            ),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Cyan)),
            Span::styled("Token & Cost Tracking", Style::default().fg(Color::Reset)),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Inline Tool Approval (3 policies)",
                Style::default().fg(Color::Reset),
            ),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Cyan)),
            Span::styled("Session File Tracking", Style::default().fg(Color::Reset)),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Cyan)),
            Span::styled("Project Organization", Style::default().fg(Color::Reset)),
        ]),
        Line::from(""),
    ];

    // Pad left column to match right column length for even rendering
    while left.len() < right.len() {
        left.push(Line::from(""));
    }

    // Record what was rendered so the key handler can clamp scroll to it.
    // `render_chat` does the same with `chat_area_height`; without it the
    // offset has no ceiling and winds off the end of the content (#1527).
    app.help_content_rows = left.len();
    app.help_viewport_rows = area.height.saturating_sub(2) as usize; // minus borders
    let scroll = app.help_scroll_offset.min(help_catalog::max_scroll(
        app.help_content_rows,
        app.help_viewport_rows,
    )) as u16;

    let left_para = Paragraph::new(left)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " 📚 Help & Commands ",
                    Style::default()
                        .fg(theme::role(Role::Accent))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(theme::role(Role::Gray))),
        )
        .scroll((scroll, 0));

    let right_para = Paragraph::new(right)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::role(Role::Gray))),
        )
        .scroll((scroll, 0));

    f.render_widget(left_para, columns[0]);
    f.render_widget(right_para, columns[1]);
}

/// Render the settings screen
pub(super) fn render_settings(f: &mut Frame, app: &mut App, area: Rect) {
    fn section(title: &str) -> Line<'_> {
        Line::from(Span::styled(
            format!("  {} ", title),
            Style::default()
                .fg(theme::role(Role::BlueSlate))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ))
    }

    fn kv<'a>(key: &'a str, val: &'a str) -> Line<'a> {
        Line::from(vec![
            Span::styled(
                format!("   {:<20}", key),
                Style::default().fg(theme::role(Role::Accent)),
            ),
            Span::styled(val, Style::default().fg(Color::Reset)),
        ])
    }

    fn status_dot<'a>(label: &'a str, enabled: bool) -> Line<'a> {
        let (dot, color) = if enabled {
            ("●", Color::Cyan)
        } else {
            ("○", Color::DarkGray)
        };
        Line::from(vec![
            Span::styled(
                format!("   {:<20}", label),
                Style::default().fg(theme::role(Role::Accent)),
            ),
            Span::styled(dot, Style::default().fg(color)),
            Span::styled(
                if enabled { " enabled" } else { " disabled" },
                Style::default().fg(Color::DarkGray),
            ),
        ])
    }

    // Approval policy display
    let approval = if app.approval_auto_always {
        "auto-always"
    } else if app.approval_auto_session {
        "auto-session"
    } else {
        "ask"
    };

    // Memory search is always available (built-in FTS5)
    let memory_available = true;

    // User commands count
    let cmd_count = app.user_commands.len();
    let cmd_summary = if cmd_count == 0 {
        "none".to_string()
    } else {
        let names: Vec<&str> = app.user_commands.iter().map(|c| c.name.as_str()).collect();
        format!("{} ({})", cmd_count, names.join(", "))
    };

    // Config file path
    let config_path = crate::config::Config::system_config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/.opencrabs/config.toml".into());

    let home_dir = dirs::home_dir()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default();
    let collapse_home = |path: &str| -> String {
        if !home_dir.is_empty() && path.starts_with(&home_dir) {
            format!("~{}", &path[home_dir.len()..])
        } else {
            path.to_string()
        }
    };
    let brain_display = collapse_home(&app.brain_path.display().to_string());
    let wd_display = collapse_home(&app.working_directory.display().to_string());

    // Session-aware: /debug must show what actually serves this session,
    // not the global default (#1464).
    let provider_name = app.provider_name_for_current_session();
    let model_name = app.provider_model_for_current_session();
    let mut lines = vec![
        Line::from(""),
        section("PROVIDER"),
        kv("Provider", &provider_name),
        kv("Model", &model_name),
        Line::from(""),
        section("APPROVAL"),
        kv("Policy", approval),
        Line::from(""),
        section("COMMANDS"),
        kv("User commands", &cmd_summary),
        Line::from(""),
        section("MEMORY"),
        status_dot("Memory search", memory_available),
        Line::from(""),
        section("PATHS"),
        kv("Config", &config_path),
        kv("Brain", &brain_display),
        kv("Working dir", &wd_display),
        Line::from(""),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [↑↓ PgUp/Dn]",
                Style::default()
                    .fg(theme::role(Role::BlueSlate))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Scroll  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(theme::role(Role::Accent))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Back", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ];

    // Pad to fill the area
    let min_height = area.height as usize;
    while lines.len() < min_height {
        lines.push(Line::from(""));
    }

    // Settings shares `help_scroll_offset` and the key-handler arm that clamps
    // it, so it has to record its own height too. Clamping against the help
    // screen's numbers would be wrong when help was last open, and fatal when
    // it was never opened: max_scroll(0, 0) is 0, and Settings would refuse to
    // scroll at all.
    app.help_content_rows = lines.len();
    app.help_viewport_rows = area.height.saturating_sub(2) as usize; // minus borders
    let scroll = app.help_scroll_offset.min(help_catalog::max_scroll(
        app.help_content_rows,
        app.help_viewport_rows,
    )) as u16;

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " Settings ",
                    Style::default()
                        .fg(theme::role(Role::BlueSlate))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(theme::role(Role::Gray))),
        )
        .scroll((scroll, 0));

    f.render_widget(para, area);
}
