//! Help, plan mode, and settings rendering
//!
//! Help screen, plan mode view, plan mode help bar, and settings screen.

use super::super::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Render the help screen
pub(super) fn render_help(f: &mut Frame, app: &App, area: Rect) {
    // Helper to build a "key → description" line
    fn kv<'a>(key: &'a str, desc: &'a str, key_color: Color) -> Line<'a> {
        Line::from(vec![
            Span::styled(
                format!(" {:<14}", key),
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default().fg(Color::DarkGray)),
            Span::styled(desc, Style::default().fg(Color::White)),
        ])
    }

    fn section_header(title: &str) -> Line<'_> {
        Line::from(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(Color::Rgb(90, 110, 150))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ))
    }

    // Split into two columns
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // ── LEFT COLUMN ──
    let gold = Color::Rgb(215, 100, 20);
    let blue = Color::Rgb(90, 110, 150);

    let mut left = vec![
        Line::from(""),
        section_header("GLOBAL"),
        kv("Ctrl+C", "Clear input / quit (2x)", gold),
        kv("Ctrl+N", "New session", gold),
        kv("Ctrl+L", "List sessions", gold),
        kv("Ctrl+K", "Clear session", gold),
        Line::from(""),
        section_header("CHAT"),
        kv("Enter", "Send message", blue),
        kv("Alt+Enter", "New line", blue),
        kv("Escape (x2)", "Clear input / abort", blue),
        kv("Page Up/Down", "Scroll history", blue),
        kv("@", "File picker", blue),
        Line::from(""),
        section_header("SLASH COMMANDS"),
        kv("/help", "Show this screen", blue),
        kv("/models", "Switch model", blue),
        kv("/usage", "Token & cost stats", blue),
        kv("/onboard", "Setup wizard", blue),
        kv("/sessions", "Session manager", blue),
        kv("/approve", "Tool approval policy", blue),
        kv("/compact", "Compact context now", blue),
        kv("/rebuild", "Build & restart from source", blue),
        kv("/cd", "Change working directory", blue),
        kv("/whisper", "Speak anywhere, paste to clipboard", blue),
        Line::from(""),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [↑↓ PgUp/Dn]",
                Style::default()
                    .fg(Color::Rgb(90, 110, 150))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Scroll  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Esc]",
                Style::default().fg(gold).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Back", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ];

    // ── RIGHT COLUMN ──
    let mag = Color::Magenta;

    let right = vec![
        Line::from(""),
        section_header("SESSIONS"),
        kv("↑ / ↓", "Navigate", mag),
        kv("Enter", "Load session", mag),
        kv("N", "New session", mag),
        kv("R", "Rename", mag),
        kv("D", "Delete", mag),
        kv("Esc", "Back to chat", mag),
        Line::from(""),
        section_header("TOOL APPROVAL"),
        kv("↑ / ↓", "Navigate options", blue),
        kv("Enter", "Confirm selection", blue),
        kv("D / Esc", "Deny", Color::Red),
        kv("V", "Toggle details", blue),
        Line::from(""),
        section_header("FEATURES"),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Rgb(90, 110, 150))),
            Span::styled(
                "Markdown & Syntax Highlighting",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Rgb(90, 110, 150))),
            Span::styled(
                "Multi-line Input & Streaming",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Rgb(90, 110, 150))),
            Span::styled(
                "Session Management & History",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Rgb(90, 110, 150))),
            Span::styled("Token & Cost Tracking", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Rgb(90, 110, 150))),
            Span::styled(
                "Inline Tool Approval (3 policies)",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
    ];

    // Pad left column to match right column length for even rendering
    while left.len() < right.len() {
        left.push(Line::from(""));
    }

    let left_para = Paragraph::new(left)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " 📚 Help & Commands ",
                    Style::default()
                        .fg(Color::Rgb(90, 110, 150))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(Color::Rgb(120, 120, 120))),
        )
        .scroll((app.help_scroll_offset as u16, 0));

    let right_para = Paragraph::new(right)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(120, 120, 120))),
        )
        .scroll((app.help_scroll_offset as u16, 0));

    f.render_widget(left_para, columns[0]);
    f.render_widget(right_para, columns[1]);
}

/// Render the settings screen
pub(super) fn render_settings(f: &mut Frame, app: &App, area: Rect) {
    fn section(title: &str) -> Line<'_> {
        Line::from(Span::styled(
            format!("  {} ", title),
            Style::default()
                .fg(Color::Rgb(90, 110, 150))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ))
    }

    fn kv<'a>(key: &'a str, val: &'a str) -> Line<'a> {
        Line::from(vec![
            Span::styled(
                format!("   {:<20}", key),
                Style::default().fg(Color::Rgb(215, 100, 20)),
            ),
            Span::styled(val, Style::default().fg(Color::White)),
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
                Style::default().fg(Color::Rgb(215, 100, 20)),
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

    let brain_display = app.brain_path.display().to_string();
    let wd_display = app.working_directory.display().to_string();

    let provider_name = app.provider_name();
    let mut lines = vec![
        Line::from(""),
        section("PROVIDER"),
        kv("Provider", &provider_name),
        kv("Model", &app.default_model_name),
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
                    .fg(Color::Rgb(90, 110, 150))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Scroll  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(Color::Rgb(215, 100, 20))
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

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " Settings ",
                    Style::default()
                        .fg(Color::Rgb(90, 110, 150))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(Color::Rgb(120, 120, 120))),
        )
        .scroll((app.help_scroll_offset as u16, 0));

    f.render_widget(para, area);
}
