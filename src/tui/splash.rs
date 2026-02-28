//! Splash Screen
//!
//! Startup welcome screen with logo and project information.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Render the splash screen
pub fn render_splash(f: &mut Frame, area: Rect, provider_name: &str, model_name: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(20),
            Constraint::Min(0),
        ])
        .split(area);

    // Center horizontally
    let center_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(80),
            Constraint::Min(0),
        ])
        .split(chunks[1]);

    render_splash_content(f, center_chunks[1], provider_name, model_name);
}

fn render_splash_content(f: &mut Frame, area: Rect, provider_name: &str, model_name: &str) {
    let version = env!("CARGO_PKG_VERSION");

    let logo_style = Style::default()
        .fg(Color::Rgb(215, 100, 20))
        .add_modifier(Modifier::BOLD);

    // Pad all ASCII art lines to the same width so centering keeps them aligned
    let logo_lines: Vec<String> = vec![
        "   ___                    ___           _".to_string(),
        "  / _ \\ _ __  ___ _ _    / __|_ _ __ _| |__  ___".to_string(),
        " | (_) | '_ \\/ -_) ' \\  | (__| '_/ _` | '_ \\(_-<".to_string(),
        r"  \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/".to_string(),
        "       |_|".to_string(),
    ];
    let max_len = logo_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    let mut splash_text = vec![Line::from(""), Line::from("")];

    // Add padded logo lines
    for line in &logo_lines {
        splash_text.push(Line::from(Span::styled(
            format!("{:<width$}", line, width = max_len),
            logo_style,
        )));
    }

    splash_text.extend(vec![
        Line::from(""),
        Line::from(Span::styled(
            "🦀 Shell Yeah! AI Orchestration at Rust Speed.",
            Style::default()
                .fg(Color::Rgb(215, 100, 20))
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        )),
        Line::from(""),
        // Project name and version
        Line::from(vec![
            Span::styled("╭─── ", Style::default().fg(Color::Rgb(90, 110, 150))),
            Span::styled(
                "🦀 OpenCrabs",
                Style::default()
                    .fg(Color::Rgb(215, 100, 20))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" v{} ", version),
                Style::default()
                    .fg(Color::Rgb(215, 100, 20))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("───╮", Style::default().fg(Color::Rgb(90, 110, 150))),
        ]),
        Line::from(""),
        // Model and details
        Line::from(vec![
            Span::styled("Model: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                model_name,
                Style::default()
                    .fg(Color::Rgb(90, 110, 150))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
            Span::styled(provider_name, Style::default().fg(Color::Rgb(90, 110, 150))),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "AI Orchestration Agent for Software Development",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to continue...",
            Style::default()
                .fg(Color::Rgb(215, 100, 20))
                .add_modifier(Modifier::BOLD),
        )),
    ]);

    let splash = Paragraph::new(splash_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(120, 120, 120))),
        )
        .alignment(Alignment::Center);

    f.render_widget(splash, area);
}
