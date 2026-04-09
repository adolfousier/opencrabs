//! Header card overlay
//!
//! Replaces the old blocking splash screen. Rendered as a centered
//! bordered block on top of the chat area on launch, showing the
//! OpenCrabs logo, version, provider/model, available tools, quick
//! commands, and tips. Vanishes after the timeout (see state.rs), on
//! Enter, or on scroll — whichever comes first. Does not block input:
//! the user can type (and submit) while it's visible.
//!
//! The card auto-fits its content: width matches the longest unwrapped
//! line (clamped to the chat area) and height matches however many rows
//! the content needs after wrapping.

use super::super::app::App;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

// Border + padding overhead.
const H_CHROME: u16 = 2 /* borders */ + 4 /* padding (2 each side) */;
const V_CHROME: u16 = 2 /* borders */ + 2 /* padding (1 top/bottom) */;

/// Render the header card centered within the given area (the chat region).
pub(super) fn render_header_card(f: &mut Frame, app: &App, area: Rect) {
    if area.width < 20 || area.height < 8 {
        return;
    }

    let accent = Style::default()
        .fg(Color::Rgb(215, 100, 20))
        .add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(Color::Rgb(120, 120, 120));
    let dim = Style::default().fg(Color::DarkGray);
    let header = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    // Leave a 2-cell gutter on each side of the chat area.
    let max_inner_w = area.width.saturating_sub(H_CHROME + 4);
    let max_inner_h = area.height.saturating_sub(V_CHROME + 2);

    let (centered, wrapped) = build_content(app, accent, muted, dim, header);

    // Natural width is driven by the centered block (logo + tagline +
    // version line). The wrapped block (tool list, commands, tips) will
    // soft-wrap to this width — we deliberately don't let it drag the
    // card out to the widest single tool-list line, or the card ends up
    // extremely wide and short. Add a little breathing room.
    let natural_w: u16 = centered.iter().map(display_width).max().unwrap_or(40) + 4;
    let inner_w = natural_w.clamp(20, max_inner_w.max(20));

    // Height = centered rows + wrapped rows (after soft-wrap to inner_w).
    let centered_rows = centered.len() as u16;
    let wrapped_rows: u16 = wrapped.iter().map(|l| wrap_rows_for_line(l, inner_w)).sum();
    let inner_h = (centered_rows + wrapped_rows).min(max_inner_h);

    let card_w = (inner_w + H_CHROME).min(area.width);
    let card_h = (inner_h + V_CHROME).min(area.height);

    let x = area.x + area.width.saturating_sub(card_w) / 2;
    let y = area.y + area.height.saturating_sub(card_h) / 2;
    let card_area = Rect {
        x,
        y,
        width: card_w,
        height: card_h,
    };

    f.render_widget(Clear, card_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(120, 120, 120)))
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(card_area);
    f.render_widget(block, card_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(centered_rows.min(inner.height)),
            Constraint::Length(inner.height.saturating_sub(centered_rows)),
        ])
        .split(inner);

    let centered_para = Paragraph::new(centered).alignment(Alignment::Center);
    f.render_widget(centered_para, chunks[0]);

    if chunks[1].height > 0 {
        let wrapped_para = Paragraph::new(wrapped)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        f.render_widget(wrapped_para, chunks[1]);
    }
}

/// Build centered (logo/tagline/version) and wrapped (tools/commands/tips) content.
fn build_content(
    app: &App,
    accent: Style,
    muted: Style,
    dim: Style,
    header: Style,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let logo_lines: &[&str] = &[
        "   ___                    ___           _",
        "  / _ \\ _ __  ___ _ _    / __|_ _ __ _| |__  ___",
        " | (_) | '_ \\/ -_) ' \\  | (__| '_/ _` | '_ \\(_-<",
        r"  \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/",
        "       |_|",
    ];

    let version = env!("CARGO_PKG_VERSION");
    let provider = app.agent_service.provider_name().to_string();
    let model = app.default_model_name.clone();

    // Pad every logo line to the widest one so when each line is centered
    // individually, the glyph columns still line up vertically.
    let logo_max = logo_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    let mut centered: Vec<Line<'static>> = Vec::new();
    for line in logo_lines {
        centered.push(Line::from(Span::styled(
            format!("{:<width$}", line, width = logo_max),
            accent,
        )));
    }
    centered.push(Line::from(""));
    centered.push(Line::from(Span::styled(
        "🦀 The autonomous AI agent. Self-improving. Every channel.",
        Style::default()
            .fg(Color::Rgb(215, 100, 20))
            .add_modifier(Modifier::ITALIC),
    )));
    centered.push(Line::from(""));
    centered.push(Line::from(vec![
        Span::styled(format!("v{version}"), accent),
        Span::styled("  ·  ", muted),
        Span::styled(provider, header),
        Span::styled("  ·  ", muted),
        Span::styled(model, header),
    ]));
    centered.push(Line::from(""));

    let mut wrapped: Vec<Line<'static>> = Vec::new();

    let tool_count = app.agent_service.tool_registry().count();
    if tool_count > 0 {
        let mut tool_names: Vec<String> = app.agent_service.tool_registry().list_tools();
        tool_names.sort();
        wrapped.push(Line::from(Span::styled("Available Tools", header)));
        wrapped.push(Line::from(Span::styled(tool_names.join(", "), dim)));
        wrapped.push(Line::from(""));
    }

    wrapped.push(Line::from(Span::styled("Quick Commands", header)));
    wrapped.push(Line::from(Span::styled(
        "/help  /sessions  /model  /settings  /usage  /approve  /rebuild  /doctor".to_string(),
        dim,
    )));
    wrapped.push(Line::from(""));

    wrapped.push(Line::from(Span::styled("Tips", header)));
    wrapped.push(Line::from(Span::styled(
        "@ for files  ·  ! for shell  ·  Shift+Enter for newline  ·  Ctrl+O for older messages"
            .to_string(),
        dim,
    )));

    (centered, wrapped)
}

/// Visible width (in cells) of a rendered line.
fn display_width(line: &Line<'_>) -> u16 {
    line.spans
        .iter()
        .map(|s| s.content.chars().count() as u16)
        .sum()
}

/// Number of rows a line will occupy after soft-wrapping to `width`.
/// Uses whitespace-preserving wrap roughly matching ratatui's Wrap { trim: false }.
fn wrap_rows_for_line(line: &Line<'_>, width: u16) -> u16 {
    if width == 0 {
        return 1;
    }
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    if text.is_empty() {
        return 1;
    }
    let w = width as usize;
    let mut rows: u16 = 0;
    let mut col: usize = 0;
    for word in text.split_inclusive(' ') {
        let wl = word.chars().count();
        if col == 0 {
            if wl > w {
                // Word longer than a row — it will wrap across ceil(wl/w) rows.
                rows += wl.div_ceil(w) as u16;
                col = wl % w;
                if col == 0 {
                    col = w;
                }
            } else {
                rows += 1;
                col = wl;
            }
        } else if col + wl > w {
            rows += 1;
            col = wl.min(w);
        } else {
            col += wl;
        }
    }
    rows.max(1)
}
