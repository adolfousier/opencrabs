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
            Span::styled(desc, Style::default().fg(Color::Reset)),
        ])
    }

    fn section_header(title: &str) -> Line<'_> {
        Line::from(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(Color::Rgb(215, 100, 20))
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

    let mut left = vec![
        Line::from(""),
        section_header("GLOBAL"),
        kv("Ctrl+C", "Clear input / quit (2x)", cyan),
        kv("Ctrl+N", "New session", cyan),
        kv("Ctrl+L", "List sessions", cyan),
        kv("Ctrl+K", "Clear session", cyan),
        Line::from(""),
        section_header("CHAT"),
        kv("Enter", "Send message", cyan),
        kv("Ctrl+J", "New line (vim)", cyan),
        kv("Escape (x2)", "Clear input / abort", cyan),
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
        kv("/help", "Show this screen", cyan),
        kv("/models", "Switch model", cyan),
        kv("/usage", "Token & cost stats", cyan),
        kv("/onboard", "Setup wizard (start)", cyan),
        kv("/onboard:provider", "Jump to AI provider setup", cyan),
        kv("/onboard:workspace", "Jump to workspace settings", cyan),
        kv("/onboard:channels", "Jump to channel config", cyan),
        kv("/onboard:voice", "Jump to voice STT/TTS setup", cyan),
        kv("/onboard:image", "Jump to image handling setup", cyan),
        kv("/onboard:brain", "Jump to brain/persona setup", cyan),
        kv("/doctor", "Run connection health check", cyan),
        kv("/new", "Start a new session", cyan),
        kv("/sessions", "Session manager", cyan),
        kv("/approve", "Tool approval policy", cyan),
        kv("/compact", "Compact context now", cyan),
        kv("/rebuild", "Build & restart from source", cyan),
        kv("/evolve", "Download latest release & restart", cyan),
        kv("/cd", "Change working directory", cyan),
        kv("/whisper", "Speak anywhere, paste to clipboard", cyan),
    ];

    // Append user-defined commands from commands.toml
    let brain_path = crate::brain::BrainLoader::resolve_path();
    let loader = crate::brain::CommandLoader::from_brain_path(&brain_path);
    let mut user_cmds = loader.load();
    if !user_cmds.is_empty() {
        left.push(Line::from(""));
        left.push(section_header("CUSTOM COMMANDS"));
        user_cmds.sort_by(|a, b| a.name.cmp(&b.name));
        for cmd in &user_cmds {
            left.push(kv(&cmd.name, &cmd.description, cyan));
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
        kv("Esc", "Back to chat", cyan),
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
                        .fg(Color::Rgb(215, 100, 20))
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
