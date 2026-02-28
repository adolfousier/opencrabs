//! Plan Checklist Widget
//!
//! Renders a live-updating checklist of plan tasks above the input box.

use super::super::app::App;
use crate::tui::plan::TaskStatus;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Maximum number of task rows displayed (excludes header and footer).
const MAX_VISIBLE_TASKS: usize = 6;

/// Render the plan checklist panel.
pub(super) fn render_plan_checklist(f: &mut Frame, app: &App, area: Rect) {
    let plan = match app.plan_document.as_ref() {
        Some(p) => p,
        None => return,
    };

    if area.height == 0 {
        return;
    }

    let total = plan.tasks.len();
    let completed = plan
        .tasks
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Completed | TaskStatus::Skipped))
        .count();

    let percent = (completed * 100).checked_div(total).unwrap_or(0);

    // Progress bar: 10 chars wide
    let filled = (completed * 10).checked_div(total).unwrap_or(0);
    let bar: String = "█".repeat(filled) + &"░".repeat(10 - filled);

    // Truncate title to fit header
    let max_title_len = area.width.saturating_sub(40) as usize;
    let title = if plan.title.len() > max_title_len && max_title_len > 3 {
        format!("{}…", &plan.title[..max_title_len - 1])
    } else {
        plan.title.clone()
    };

    let header = Line::from(vec![
        Span::styled(
            format!(" Plan: {}  ·  {}/{}  ", title, completed, total),
            Style::default()
                .fg(Color::Rgb(160, 160, 160))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(bar, Style::default().fg(Color::Rgb(80, 175, 175))),
        Span::styled(
            format!("  {}%", percent),
            Style::default().fg(Color::Rgb(160, 160, 160)),
        ),
    ]);

    let visible: Vec<&crate::tui::plan::PlanTask> =
        plan.tasks.iter().take(MAX_VISIBLE_TASKS).collect();
    let overflow = total.saturating_sub(MAX_VISIBLE_TASKS);

    let mut lines: Vec<Line> = vec![header];

    for task in &visible {
        let (icon, color) = match &task.status {
            TaskStatus::Completed => ("✓", Color::Rgb(60, 165, 165)),
            TaskStatus::Skipped => ("✓", Color::Rgb(60, 165, 165)),
            TaskStatus::InProgress => ("▶", Color::Rgb(215, 100, 20)),
            TaskStatus::Failed => ("✗", Color::Red),
            TaskStatus::Blocked(_) => ("·", Color::DarkGray),
            TaskStatus::Pending => ("·", Color::DarkGray),
        };

        // Truncate task title to 60 chars
        let task_title = if task.title.len() > 60 {
            format!("{}…", &task.title[..59])
        } else {
            task.title.clone()
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}  #{:<2}  ", icon, task.order),
                Style::default().fg(color),
            ),
            Span::styled(task_title, Style::default().fg(color)),
        ]));
    }

    if overflow > 0 {
        lines.push(Line::from(Span::styled(
            format!("  ... ({} more)", overflow),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let border_style = Style::default().fg(Color::Rgb(50, 50, 50));
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(border_style);

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}
