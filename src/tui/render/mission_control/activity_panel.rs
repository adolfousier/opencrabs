//! Activity panel — top right (60% × 50%), RSI activity feed.
//!
//! Reads the cached `Vec<McActivity>` snapshot from `app.mc.activity`
//! (populated by `actions::refresh`). The renderer never hits the
//! filesystem itself.

use super::theme;
use crate::brain::mission_control::{McActivity, McActivityLevel};
use crate::tui::app::App;

use super::super::theme::{self as render_theme, Role};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn draw(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let entries = &app.mc.activity;
    let border_color = if focused {
        theme::border_activity_focus()
    } else {
        theme::border_idle()
    };
    let title = format!(" Activity ({}) ", entries.len());
    let block = Block::default()
        .title(title)
        .title_style(theme::title_style(theme::border_activity_focus()))
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .border_style(Style::default().fg(border_color));

    if entries.is_empty() {
        let empty = Paragraph::new(Line::from(vec![
            Span::raw("\n  "),
            Span::styled("No activity yet.", Style::default().fg(theme::text_dim())),
        ]))
        .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize; // borders only
    let visible_h = area.height.saturating_sub(2) as usize;

    let selected = if focused {
        Some(app.mc.selected_index.min(entries.len().saturating_sub(1)))
    } else {
        None
    };

    let lines: Vec<Line> = entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| build_line(entry, inner_w, selected == Some(idx)))
        .collect();

    let scroll = compute_scroll(selected, entries.len(), visible_h);
    let para = Paragraph::new(lines).block(block).scroll((scroll, 0));
    frame.render_widget(para, area);
}

fn build_line(entry: &McActivity, inner_w: usize, selected: bool) -> Line<'static> {
    let dot_color = level_color(entry.level);
    let stamp = relative_time(entry.timestamp);
    // Reserve: " ●  HH:MM:SS  " ≈ 14 cells, plus 1 trailing.
    let prefix_chars = 4 + stamp.chars().count() + 2;
    let detail_max = inner_w.saturating_sub(prefix_chars + 1);
    let detail = trunc(&entry.detail, detail_max);

    let mut spans = vec![
        Span::raw(" "),
        Span::styled("●", Style::default().fg(dot_color)),
        Span::raw("  "),
        Span::styled(stamp, Style::default().fg(theme::text_dim())),
        Span::raw("  "),
        Span::styled(detail, Style::default().fg(theme::text_primary())),
    ];

    if selected {
        // Subtle background + bold to mark the focused row.
        for span in &mut spans {
            span.style = span.style.bg(render_theme::role(Role::SurfacePanel));
        }
        spans.last_mut().unwrap().style =
            spans.last_mut().unwrap().style.add_modifier(Modifier::BOLD);
    }
    Line::from(spans)
}

fn level_color(level: McActivityLevel) -> Color {
    match level {
        McActivityLevel::Success => theme::teal(),
        McActivityLevel::Warn => theme::orange(),
        McActivityLevel::Error => render_theme::role(Role::Error),
        McActivityLevel::Info => theme::white(),
    }
}

fn compute_scroll(selected: Option<usize>, count: usize, visible_h: usize) -> u16 {
    let Some(sel) = selected else { return 0 };
    if visible_h == 0 || count == 0 {
        return 0;
    }
    let sel = sel.min(count.saturating_sub(1));
    if sel >= visible_h {
        (sel - visible_h + 1) as u16
    } else {
        0
    }
}

fn relative_time(ts: Option<DateTime<Utc>>) -> String {
    // An entry the journal wrote without a parseable date reads as undated
    // rather than borrowing the current instant (#841).
    let Some(ts) = ts else {
        return "undated".to_string();
    };
    let secs = (Utc::now() - ts).num_seconds();
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
