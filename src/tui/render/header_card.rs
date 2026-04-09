//! Header card overlay
//!
//! Replaces the old blocking splash screen. Rendered as a centered
//! bordered block on top of the chat area on launch, showing the
//! OpenCrabs logo, version, provider/model, available tools, quick
//! commands, and tips. Vanishes after 500ms, on Enter, or on scroll
//! — whichever comes first. Does not block input: the user can type
//! (and submit) while it's visible.

use super::super::app::App;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// Render the header card centered within the given area (the chat region).
pub(super) fn render_header_card(f: &mut Frame, app: &App, area: Rect) {
    // Logo is 5 rows, plus ~15 rows of content/tips = ~22 rows including borders.
    // Cap by actual area height so narrow terminals still render something.
    let desired_height: u16 = 22;
    let card_height = desired_height.min(area.height.saturating_sub(2).max(5));
    let desired_width: u16 = 76;
    let card_width = desired_width.min(area.width.saturating_sub(2).max(20));

    // Center within area
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(card_height),
            Constraint::Min(0),
        ])
        .split(area);
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(card_width),
            Constraint::Min(0),
        ])
        .split(v_chunks[1]);
    let card_area = h_chunks[1];

    // Wipe whatever chat was rendered underneath so the card reads cleanly.
    f.render_widget(Clear, card_area);

    render_card_content(f, card_area, app);
}

fn render_card_content(f: &mut Frame, area: Rect, app: &App) {
    let version = env!("CARGO_PKG_VERSION");
    let provider = app.agent_service.provider_name();
    let model = app.default_model_name.as_str();

    let accent = Style::default()
        .fg(Color::Rgb(215, 100, 20))
        .add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(Color::Rgb(120, 120, 120));
    let dim = Style::default().fg(Color::DarkGray);
    let header = Style::default()
        .fg(Color::Rgb(90, 110, 150))
        .add_modifier(Modifier::BOLD);

    // 5-line ASCII logo, padded to equal width so Alignment::Center doesn't
    // fragment the letters across rows.
    let logo_lines: Vec<&str> = vec![
        "   ___                    ___           _",
        "  / _ \\ _ __  ___ _ _    / __|_ _ __ _| |__  ___",
        " | (_) | '_ \\/ -_) ' \\  | (__| '_/ _` | '_ \\(_-<",
        r"  \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/",
        "       |_|",
    ];
    let max_logo = logo_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    let mut text: Vec<Line> = Vec::with_capacity(22);
    for line in &logo_lines {
        text.push(Line::from(Span::styled(
            format!("{:<width$}", line, width = max_logo),
            accent,
        )));
    }

    text.push(Line::from(""));
    text.push(Line::from(Span::styled(
        "The autonomous AI agent. Self-improving. Every channel.",
        Style::default()
            .fg(Color::Rgb(215, 100, 20))
            .add_modifier(Modifier::ITALIC),
    )));
    text.push(Line::from(""));

    // Version + provider/model on a single line
    text.push(Line::from(vec![
        Span::styled("🦀 OpenCrabs ", accent),
        Span::styled(format!("v{version}"), accent),
        Span::styled("  ·  ", muted),
        Span::styled(provider, header),
        Span::styled("  ·  ", muted),
        Span::styled(model.to_string(), header),
    ]));
    text.push(Line::from(""));

    // Available Tools — first few tool names from the registry
    let tool_count = app.agent_service.tool_registry().count();
    let mut tool_names: Vec<String> = app
        .agent_service
        .tool_registry()
        .list_tools()
        .into_iter()
        .collect();
    tool_names.sort();
    let preview = tool_names
        .iter()
        .take(10)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let tools_line = if tool_count > 10 {
        format!("{preview}  (and {} more)", tool_count - 10)
    } else {
        preview
    };
    text.push(Line::from(vec![Span::styled("Available Tools", header)]));
    text.push(Line::from(Span::styled(tools_line, dim)));
    text.push(Line::from(""));

    // Quick Commands — built-in slash commands that matter most + user commands
    let builtin = "/help  /sessions  /model  /settings  /usage  /approve  /rebuild";
    text.push(Line::from(vec![Span::styled("Quick Commands", header)]));
    text.push(Line::from(Span::styled(builtin, dim)));
    if !app.user_commands.is_empty() {
        let user_cmds = app
            .user_commands
            .iter()
            .take(8)
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
            .join("  ");
        text.push(Line::from(Span::styled(user_cmds, dim)));
    }
    text.push(Line::from(""));

    // Tips
    text.push(Line::from(vec![Span::styled("Tips", header)]));
    text.push(Line::from(Span::styled(
        "@ for files  ·  ! for shell  ·  Shift+Enter for newline  ·  Ctrl+O older messages",
        dim,
    )));

    let widget = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(120, 120, 120))),
        )
        .alignment(Alignment::Center);

    f.render_widget(widget, area);
}
