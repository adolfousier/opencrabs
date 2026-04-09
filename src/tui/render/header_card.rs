//! Header card overlay
//!
//! Replaces the old blocking splash screen. Rendered as a centered
//! bordered block on top of the chat area on launch, showing the
//! OpenCrabs logo, version, provider/model, available tools, quick
//! commands, and tips. Vanishes after the timeout (see state.rs), on
//! Enter, or on scroll — whichever comes first. Does not block input:
//! the user can type (and submit) while it's visible.
//!
//! The card is fully responsive to the chat area it's handed. Resize
//! events (terminal resize, pane split, etc.) flow through the normal
//! render loop and the card recomputes its geometry on the next frame.
//! Long lines (tool list, tips) wrap onto additional rows.

use super::super::app::App;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

/// Render the header card centered within the given area (the chat region).
pub(super) fn render_header_card(f: &mut Frame, app: &App, area: Rect) {
    // Outer margin: the card floats inside the chat area with a small
    // visible gap on each side so the surrounding chat still peeks
    // through. Margins shrink on tiny terminals so we always render
    // something rather than collapsing to nothing.
    let h_margin: u16 = if area.width >= 60 {
        4
    } else if area.width >= 30 {
        2
    } else {
        1
    };
    let v_margin: u16 = if area.height >= 24 {
        2
    } else if area.height >= 12 {
        1
    } else {
        0
    };

    let card_area = area.inner(Margin {
        horizontal: h_margin,
        vertical: v_margin,
    });

    if card_area.width < 20 || card_area.height < 8 {
        return; // too small to render meaningfully
    }

    // Wipe whatever chat was rendered underneath so the card reads cleanly.
    f.render_widget(Clear, card_area);

    render_card_content(f, card_area, app);
}

fn render_card_content(f: &mut Frame, area: Rect, app: &App) {
    let accent = Style::default()
        .fg(Color::Rgb(215, 100, 20))
        .add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(Color::Rgb(120, 120, 120));
    let dim = Style::default().fg(Color::DarkGray);
    let header = Style::default()
        .fg(Color::Rgb(90, 110, 150))
        .add_modifier(Modifier::BOLD);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(120, 120, 120)))
        // Inner padding: 2 columns horizontal, 1 row top/bottom
        .padding(Padding::new(2, 2, 1, 1));

    // Inner content area (inside borders + padding)
    let inner = block.inner(area);
    f.render_widget(block, area);

    // ── Logo ─────────────────────────────────────────────────────────────
    // The ASCII art is ~48 columns wide; on very narrow cards we skip it.
    let logo_lines: &[&str] = &[
        "   ___                    ___           _",
        "  / _ \\ _ __  ___ _ _    / __|_ _ __ _| |__  ___",
        " | (_) | '_ \\/ -_) ' \\  | (__| '_/ _` | '_ \\(_-<",
        r"  \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/",
        "       |_|",
    ];
    let logo_width: u16 = logo_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let show_logo = inner.width >= logo_width;

    // ── Build the content lines ─────────────────────────────────────────
    let version = env!("CARGO_PKG_VERSION");
    let provider = app.agent_service.provider_name();
    let model = app.default_model_name.as_str();

    let mut text: Vec<Line> = Vec::new();

    if show_logo {
        for line in logo_lines {
            text.push(Line::from(Span::styled((*line).to_string(), accent)));
        }
        text.push(Line::from(""));
    }

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

    // ── Available Tools ─────────────────────────────────────────────────
    let tool_count = app.agent_service.tool_registry().count();
    if tool_count > 0 {
        let mut tool_names: Vec<String> = app.agent_service.tool_registry().list_tools();
        tool_names.sort();
        // Show all tool names — Paragraph::Wrap will break them onto as
        // many rows as needed so nothing gets truncated at the border.
        let tools_line = tool_names.join(", ");
        text.push(Line::from(Span::styled("Available Tools", header)));
        text.push(Line::from(Span::styled(tools_line, dim)));
        text.push(Line::from(""));
    }

    // ── Quick Commands ──────────────────────────────────────────────────
    // Built-in slash commands that matter most. User-defined commands
    // are intentionally omitted here because they often duplicate the
    // builtins and are already discoverable via `/help`.
    let builtins = "/help  /sessions  /model  /settings  /usage  /approve  /rebuild  /doctor";
    text.push(Line::from(Span::styled("Quick Commands", header)));
    text.push(Line::from(Span::styled(builtins, dim)));
    text.push(Line::from(""));

    // ── Tips ─────────────────────────────────────────────────────────────
    text.push(Line::from(Span::styled("Tips", header)));
    text.push(Line::from(Span::styled(
        "@ for files  ·  ! for shell  ·  Shift+Enter for newline  ·  Ctrl+O for older messages",
        dim,
    )));

    // The logo block is centered, but wrapping text (tools/commands/tips)
    // reads much better left-aligned. Split the render into two passes:
    // top half (logo + tagline + version) centered, bottom half (tools,
    // commands, tips) left-aligned with wrap enabled.
    let (centered_text, wrapped_text) = split_centered_and_wrapped(text, show_logo);

    let centered_rows = centered_text.len() as u16;
    let wrap_rows = inner.height.saturating_sub(centered_rows);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(centered_rows.min(inner.height)),
            Constraint::Length(wrap_rows),
        ])
        .split(inner);

    let centered = Paragraph::new(centered_text).alignment(Alignment::Center);
    f.render_widget(centered, chunks[0]);

    if wrap_rows > 0 {
        let wrapped = Paragraph::new(wrapped_text)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false });
        f.render_widget(wrapped, chunks[1]);
    }
}

/// Split the card's content into the top (centered) block and the bottom
/// (left-aligned, wrapping) block. The split point is right before the
/// "Available Tools" header line.
fn split_centered_and_wrapped(
    all: Vec<Line<'static>>,
    _show_logo: bool,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let mut centered: Vec<Line<'static>> = Vec::new();
    let mut wrapped: Vec<Line<'static>> = Vec::new();
    let mut crossed = false;
    for line in all {
        if !crossed {
            // The "Available Tools" header starts the wrapping section.
            let is_tools_header = line
                .spans
                .first()
                .map(|s| s.content == "Available Tools")
                .unwrap_or(false);
            if is_tools_header {
                crossed = true;
                wrapped.push(line);
                continue;
            }
            centered.push(line);
        } else {
            wrapped.push(line);
        }
    }
    (centered, wrapped)
}
