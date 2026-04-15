//! Usage dashboard card renderers
//!
//! Each card is a self-contained render function that takes data + area and draws into a Frame.

use super::data::{
    ActivityStats, DailyStats, DashboardData, ModelStats, ProjectStats, ToolStats, fmt_cost,
    fmt_tokens,
};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

const LABEL: Style = Style::new().fg(Color::DarkGray);
const BOLD: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const DIM: Style = Style::new().fg(Color::DarkGray);
const ACCENT: Style = Style::new().fg(Color::Rgb(215, 100, 20));

fn card_block(title: &str, focused: bool) -> Block<'_> {
    let border_color = if focused {
        Color::Rgb(215, 100, 20)
    } else {
        Color::DarkGray
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" {} ", title),
            if focused { ACCENT } else { LABEL },
        ))
}

// ── Summary Bar ──────────────────────────────────────────────────────────────

pub fn render_summary(f: &mut Frame, data: &DashboardData, area: Rect, period_label: &str) {
    let s = &data.summary;
    let version = crate::VERSION;
    let line = Line::from(vec![
        Span::styled(format!("v{version}  "), ACCENT),
        Span::styled("Tokens: ", LABEL),
        Span::styled(fmt_tokens(s.total_tokens), BOLD),
        Span::styled("  Cost: ", LABEL),
        Span::styled(fmt_cost(s.total_cost), BOLD),
        Span::styled("  Sessions: ", LABEL),
        Span::styled(format!("{}", s.session_count), BOLD),
        Span::styled("  Calls: ", LABEL),
        Span::styled(format!("{}", s.call_count), BOLD),
        Span::styled(format!("  [{}]", period_label), ACCENT),
    ]);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    let paragraph = Paragraph::new(vec![line])
        .block(block)
        .alignment(Alignment::Center);
    f.render_widget(paragraph, area);
}

// ── Daily Activity ───────────────────────────────────────────────────────────

pub fn render_daily(f: &mut Frame, daily: &[DailyStats], area: Rect, focused: bool) {
    let block = card_block("Daily Activity", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if daily.is_empty() {
        let p = Paragraph::new(" No data").style(DIM);
        f.render_widget(p, inner);
        return;
    }

    let max_tokens = daily.iter().map(|d| d.tokens).max().unwrap_or(1);
    let bar_width = inner.width.saturating_sub(14) as usize; // date(10) + space + bar + cost

    let mut lines: Vec<Line> = Vec::new();
    // Show most recent days first (reversed), N that fit
    let visible = (inner.height as usize).min(daily.len());
    let start = daily.len().saturating_sub(visible);

    for day in daily[start..].iter().rev() {
        let bar_len = if max_tokens > 0 {
            ((day.tokens as f64 / max_tokens as f64) * bar_width as f64).ceil() as usize
        } else {
            0
        };
        let bar_len = bar_len.max(1).min(bar_width);
        let bar: String = "\u{2584}".repeat(bar_len); // ▄ lower-half block for visual separation
        let pad: String = " ".repeat(bar_width.saturating_sub(bar_len));
        // Show short date (MM-DD)
        let short_date = if day.date.len() >= 10 {
            &day.date[5..10]
        } else {
            &day.date
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:>5} ", short_date), DIM),
            Span::styled(bar, ACCENT),
            Span::raw(pad),
            Span::styled(format!(" {}", fmt_tokens(day.tokens)), LABEL),
        ]));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

// ── By Project ───────────────────────────────────────────────────────────────

pub fn render_projects(f: &mut Frame, projects: &[ProjectStats], area: Rect, focused: bool) {
    let block = card_block("By Project", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if projects.is_empty() {
        let p = Paragraph::new(" No data").style(DIM);
        f.render_widget(p, inner);
        return;
    }

    // Dynamic name width: reserve space for data columns (~8 chars) + spacing
    let name_width = (inner.width as usize).saturating_sub(8).max(8);

    let mut lines: Vec<Line> = Vec::new();
    let visible = (inner.height as usize).min(projects.len());
    for proj in projects.iter().take(visible) {
        let name = if proj.project.len() > name_width {
            format!(
                "{}...",
                proj.project
                    .chars()
                    .take(name_width.saturating_sub(3))
                    .collect::<String>()
            )
        } else {
            proj.project.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<width$}", name, width = name_width), BOLD),
            Span::styled(format!(" {:>8}", fmt_cost(proj.cost)), LABEL),
            Span::styled(format!(" {:>6}", fmt_tokens(proj.tokens)), DIM),
            Span::styled(format!(" {}s", proj.sessions), DIM),
        ]));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

// ── By Model ─────────────────────────────────────────────────────────────────

pub fn render_models(f: &mut Frame, models: &[ModelStats], area: Rect, focused: bool) {
    let block = card_block("By Model", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if models.is_empty() {
        let p = Paragraph::new(" No data").style(DIM);
        f.render_widget(p, inner);
        return;
    }

    // Dynamic name width: reserve space for data columns (~18 chars) + spacing
    let name_width = (inner.width as usize).saturating_sub(18).max(10);

    let mut lines: Vec<Line> = Vec::new();
    let visible = (inner.height as usize).min(models.len());
    for m in models.iter().take(visible) {
        let display = crate::tui::provider_selector::model_display_label(&m.model).to_string();
        let name = if display.len() > name_width {
            format!(
                "{}...",
                display
                    .chars()
                    .take(name_width.saturating_sub(3))
                    .collect::<String>()
            )
        } else {
            display
        };
        let cost_style = if m.estimated { ACCENT } else { LABEL };
        let cost_str = if m.estimated {
            format!("~{}", fmt_cost(m.cost))
        } else {
            fmt_cost(m.cost)
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<width$}", name, width = name_width), BOLD),
            Span::styled(format!(" {:>9}", cost_str), cost_style),
            Span::styled(format!(" {}", fmt_tokens(m.tokens)), DIM),
        ]));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

// ── Core Tools ───────────────────────────────────────────────────────────────

pub fn render_tools(f: &mut Frame, tools: &[ToolStats], area: Rect, focused: bool) {
    let block = card_block("Core Tools", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if tools.is_empty() {
        let p = Paragraph::new(" No data").style(DIM);
        f.render_widget(p, inner);
        return;
    }

    let max_count = tools.first().map(|t| t.call_count).unwrap_or(1);
    let bar_width = inner.width.saturating_sub(22) as usize;
    let visible = (inner.height as usize).min(tools.len());

    // Dynamic name width: reserve space for count column (~7 chars) and bar padding
    let name_width = (bar_width as u16).saturating_sub(2).max(8) as usize;

    let mut lines: Vec<Line> = Vec::new();
    for tool in tools.iter().take(visible) {
        let bar_len = if max_count > 0 {
            ((tool.call_count as f64 / max_count as f64) * bar_width as f64).ceil() as usize
        } else {
            0
        };
        let bar_len = bar_len.max(1).min(bar_width);
        let bar: String = "\u{2584}".repeat(bar_len);
        let pad: String = " ".repeat(bar_width.saturating_sub(bar_len));
        let name = if tool.tool_name.len() > name_width {
            format!(
                "{}...",
                tool.tool_name
                    .chars()
                    .take(name_width.saturating_sub(3))
                    .collect::<String>()
            )
        } else {
            tool.tool_name.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<width$}", name, width = name_width), BOLD),
            Span::styled(bar, ACCENT),
            Span::raw(pad),
            Span::styled(format!(" {:>5}", tool.call_count), DIM),
        ]));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

// ── By Activity ──────────────────────────────────────────────────────────────

pub fn render_activities(f: &mut Frame, activities: &[ActivityStats], area: Rect, focused: bool) {
    let block = card_block("By Activity", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if activities.is_empty() {
        let p = Paragraph::new(" No data").style(DIM);
        f.render_widget(p, inner);
        return;
    }

    // Header
    let cat_width = inner.width.saturating_sub(46).max(10) as usize;
    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled(format!(" {:<width$}", "Category", width = cat_width), LABEL),
        Span::styled(format!("{:>10}", "Cost"), LABEL),
        Span::styled(format!("{:>8}", "Turns"), LABEL),
        Span::styled(format!("{:>8}", "1-shot%"), LABEL),
    ])];

    let max_cost = activities.iter().map(|a| a.cost).fold(0.0_f64, f64::max);
    let bar_width = inner.width.saturating_sub(cat_width as u16 + 32) as usize;
    let visible = (inner.height.saturating_sub(1) as usize).min(activities.len());

    for act in activities.iter().take(visible) {
        let bar_len = if max_cost > 0.0 && bar_width > 0 {
            ((act.cost / max_cost) * bar_width as f64).ceil() as usize
        } else {
            0
        };
        let bar_len = bar_len
            .max(if act.cost > 0.0 { 1 } else { 0 })
            .min(bar_width);
        let bar: String = "\u{2584}".repeat(bar_len);
        let pad: String = " ".repeat(bar_width.saturating_sub(bar_len));
        let category = if act.category.len() > cat_width {
            format!(
                "{}...",
                act.category
                    .chars()
                    .take(cat_width.saturating_sub(3))
                    .collect::<String>()
            )
        } else {
            act.category.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<width$}", category, width = cat_width), BOLD),
            Span::styled(bar, ACCENT),
            Span::raw(pad),
            Span::styled(format!(" {:>9}", fmt_cost(act.cost)), LABEL),
            Span::styled(format!("{:>8}", act.turns), DIM),
            Span::styled(format!("{:>7}%", act.one_shot_pct as u32), DIM),
        ]));
    }

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

// ── Footer ───────────────────────────────────────────────────────────────────

pub fn render_footer(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled("Tab", ACCENT),
        Span::styled(" navigate  ", DIM),
        Span::styled("Enter", ACCENT),
        Span::styled(" details  ", DIM),
        Span::styled("T", ACCENT),
        Span::styled(" today  ", DIM),
        Span::styled("W", ACCENT),
        Span::styled(" week  ", DIM),
        Span::styled("M", ACCENT),
        Span::styled(" month  ", DIM),
        Span::styled("A", ACCENT),
        Span::styled(" all  ", DIM),
        Span::styled("Esc", ACCENT),
        Span::styled(" close", DIM),
    ]);
    let p = Paragraph::new(vec![line])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}
