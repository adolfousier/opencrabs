//! Inbox panel — left 40%, RSI proposals as cards.
//!
//! Each item from `inbox_service::list` becomes one rounded card. The
//! selected card gets a cyan border; the rest stay dimmed. Auto-scrolls
//! to keep the selected card in view.

use super::theme;
use crate::brain::mission_control::{McInboxItem, McInboxKind, inbox_service};
use crate::tui::app::App;

use super::super::theme::{self as render_theme, Role};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Idle card border — same neutral grey as the Sessions list rows.
fn card_border_idle() -> Color {
    render_theme::role(Role::TextMuted)
}
/// Selected card border — teal, matches the inbox panel's accent.
/// fn not const: resolves through the runtime theme so `/theme set` applies.
fn card_border_selected() -> Color {
    theme::teal()
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let items = inbox_service::list();
    let panel_border_color = if focused {
        theme::border_inbox_focus()
    } else {
        theme::border_idle()
    };
    let title = format!(" Inbox ({}) ", items.len());
    let block = Block::default()
        .title(title)
        .title_style(theme::title_style(theme::border_inbox_focus()))
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .border_style(Style::default().fg(panel_border_color));

    if items.is_empty() {
        let empty = Paragraph::new(Line::from(vec![
            Span::raw("\n  "),
            Span::styled(
                "No pending proposals.",
                Style::default().fg(theme::text_dim()),
            ),
        ]))
        .block(block);
        frame.render_widget(empty, area);
        return;
    }

    // Each card occupies several rows in a single Paragraph. We
    // accumulate every card's lines into one buffer, then scroll.
    let inner_w = area.width.saturating_sub(2) as usize; // 2 borders
    let card_w = inner_w.saturating_sub(2); // " ╭...╮ " — leave 1 cell each side
    let selected = if focused {
        app.mc.selected_index.min(items.len().saturating_sub(1))
    } else {
        usize::MAX // disable highlight when panel isn't focused
    };

    let mut lines: Vec<Line> = Vec::new();
    let mut card_start_rows: Vec<usize> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        card_start_rows.push(lines.len());
        let is_sel = idx == selected;
        lines.extend(card_lines(item, card_w, is_sel));
        if idx + 1 < items.len() {
            lines.push(Line::raw(""));
        }
    }

    // Auto-scroll: keep the selected card visible.
    let visible_h = area.height.saturating_sub(2) as usize; // borders only
    let scroll = if focused && selected != usize::MAX && selected < card_start_rows.len() {
        let card_top = card_start_rows[selected];
        if card_top >= visible_h {
            (card_top.saturating_sub(visible_h / 3)) as u16
        } else {
            0
        }
    } else {
        0
    };

    let para = Paragraph::new(lines).block(block).scroll((scroll, 0));
    frame.render_widget(para, area);
}

fn card_lines(item: &McInboxItem, card_w: usize, selected: bool) -> Vec<Line<'static>> {
    let border_color = if selected {
        card_border_selected()
    } else {
        card_border_idle()
    };
    let bd = Style::default().fg(border_color);

    // Inner card width (between │ and │).
    let inner = card_w.saturating_sub(2);
    let h_fill: String = "─".repeat(inner);
    let label_pad = 1;
    // Tool kind = orange (brand colour, matches the title accent on
    // sessions.rs); Command kind = teal (matches the inbox panel
    // focus accent so commands feel native to this panel); Skill
    // kind = soft amber so it sits between tool and command without
    // colliding with either focus colour.
    let kind_color = match item.kind {
        McInboxKind::ProposedTool => theme::orange(),
        McInboxKind::ProposedCommand => theme::teal(),
        McInboxKind::ProposedSkill => render_theme::role(Role::AmberMuted),
    };

    // Header: label (bold) + kind badge
    let label_max = inner.saturating_sub(item.kind.label().len() + 4); // " [tool] "
    let label = trunc(&item.label, label_max);
    let header = Line::from(vec![
        Span::styled(" ╭", bd),
        Span::styled(h_fill.clone(), bd),
        Span::styled("╮", bd),
    ]);
    let body_label = Line::from(vec![
        Span::styled(" │", bd),
        Span::raw(" ".repeat(label_pad)),
        Span::styled(
            label,
            Style::default()
                .fg(theme::text_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(" {} ", item.kind.label()),
            Style::default()
                .fg(render_theme::role(Role::Ink))
                .bg(kind_color)
                .add_modifier(Modifier::BOLD),
        ),
        // Right-pad to keep the right border aligned.
        Span::raw(pad_right_to(inner.saturating_sub(
            label_pad + item.label.chars().count().min(label_max) + 2 + item.kind.label().len() + 2,
        ))),
        Span::styled("│", bd),
    ]);

    // Summary (truncated to one line for now — multi-line lands in C11
    // alongside the detail popup).
    let summary_max = inner.saturating_sub(2 * label_pad);
    let summary = trunc(&item.summary, summary_max);
    let summary_chars = summary.chars().count();
    let body_summary = Line::from(vec![
        Span::styled(" │", bd),
        Span::raw(" ".repeat(label_pad)),
        Span::styled(summary, Style::default().fg(theme::text_secondary())),
        Span::raw(pad_right_to(
            inner.saturating_sub(label_pad + summary_chars),
        )),
        Span::styled("│", bd),
    ]);

    // Footer: source + relative timestamp
    let ago = relative_time(item.created_at);
    let footer_text = format!("{} • {}", item.source, ago);
    let footer_max = inner.saturating_sub(2 * label_pad);
    let footer_truncated = trunc(&footer_text, footer_max);
    let footer_chars = footer_truncated.chars().count();
    let footer = Line::from(vec![
        Span::styled(" │", bd),
        Span::raw(" ".repeat(label_pad)),
        Span::styled(footer_truncated, Style::default().fg(theme::text_dim())),
        Span::raw(pad_right_to(inner.saturating_sub(label_pad + footer_chars))),
        Span::styled("│", bd),
    ]);

    let bottom = Line::from(vec![
        Span::styled(" ╰", bd),
        Span::styled(h_fill, bd),
        Span::styled("╯", bd),
    ]);

    vec![header, body_label, body_summary, footer, bottom]
}

fn trunc(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

fn pad_right_to(n: usize) -> String {
    " ".repeat(n)
}

fn relative_time(ts: chrono::DateTime<chrono::Utc>) -> String {
    let secs = (chrono::Utc::now() - ts).num_seconds();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
