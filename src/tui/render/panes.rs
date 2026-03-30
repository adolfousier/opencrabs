//! Split pane rendering — draws pane borders, labels, and delegates chat rendering.

use crate::tui::app::{App, DisplayMessage};
use crate::tui::pane::PaneId;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
};

/// Render a single inactive (non-focused) pane.
/// Shows the session's cached messages as a read-only chat view.
pub(super) fn render_inactive_pane(f: &mut Frame, app: &App, pane_id: PaneId, area: Rect) {
    let pane = match app.pane_manager.get(pane_id) {
        Some(p) => p,
        None => return,
    };

    let session_label = pane
        .session_id
        .and_then(|sid| {
            app.sessions.iter().find(|s| s.id == sid).map(|s| {
                s.title
                    .clone()
                    .unwrap_or_else(|| format!("Session {}", &s.id.to_string()[..8]))
            })
        })
        .unwrap_or_else(|| "No session".to_string());

    let is_processing = pane
        .session_id
        .map(|sid| app.processing_sessions.contains(&sid))
        .unwrap_or(false);

    let status = if is_processing {
        " [processing...]"
    } else {
        ""
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            format!(" {}{} ", session_label, status),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Render cached messages if available
    let cached = pane
        .session_id
        .and_then(|sid| app.pane_message_cache.get(&sid));

    let mut lines: Vec<Line> = Vec::new();

    if let Some(messages) = cached {
        for msg in messages {
            render_simple_message(&mut lines, msg);
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Tab to switch focus",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Show last N lines that fit — no wrapping, so 1 Line = 1 row (guaranteed).
    let visible = inner.height as usize;
    let skip = lines.len().saturating_sub(visible);
    let visible_lines: Vec<Line> = lines.into_iter().skip(skip).collect();
    let para = Paragraph::new(visible_lines);
    f.render_widget(para, inner);
}

/// Render a single message in simplified form for inactive panes.
fn render_simple_message(lines: &mut Vec<Line<'_>>, msg: &DisplayMessage) {
    // Skip system messages
    if msg.role == "system" || msg.role == "history_marker" {
        return;
    }

    // Tool groups: single collapsed line matching focused pane style
    if msg.role == "tool_group" {
        if let Some(ref group) = msg.tool_group {
            let n = group.calls.len();
            let all_done = group.calls.iter().all(|c| c.completed);
            let any_failed = group.calls.iter().any(|c| c.completed && !c.success);
            let (icon, color) = if !all_done {
                ("⚙", Color::Yellow)
            } else if any_failed {
                ("●", Color::Red)
            } else {
                ("●", Color::DarkGray)
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} {} tool call{}",
                    icon,
                    n,
                    if n == 1 { "" } else { "s" }
                ),
                Style::default().fg(color),
            )));
        }
        return;
    }

    let is_assistant = msg.role == "assistant";
    let (prefix, color) = match msg.role.as_str() {
        "user" => ("> ", Color::Cyan),
        "assistant" => ("", Color::Reset),
        _ => ("", Color::DarkGray),
    };

    // Strip reasoning/tool-marker blocks from content
    let mut content = msg.content.clone();
    // Remove <!-- reasoning -->...<!-- /reasoning --> blocks
    while let Some(start) = content.find("<!-- reasoning -->") {
        if let Some(end) = content.find("<!-- /reasoning -->") {
            content = format!(
                "{}{}",
                &content[..start],
                &content[end + "<!-- /reasoning -->".len()..]
            );
        } else {
            content = content[..start].to_string();
        }
    }
    // Remove <!-- tools-v2: ... --> markers
    while let Some(start) = content.find("<!-- tools-v2:") {
        if let Some(end) = content[start..].find("-->") {
            content = format!("{}{}", &content[..start], &content[start + end + 3..]);
        } else {
            break;
        }
    }
    let content = content.trim().to_string();

    // Show thinking indicator if assistant has reasoning details
    if is_assistant && msg.details.is_some() {
        lines.push(Line::from(Span::styled(
            "  ▸ Thinking",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Truncate long messages for preview
    let content = if content.len() > 500 {
        let end = content.floor_char_boundary(497);
        format!("{}...", &content[..end])
    } else {
        content
    };

    if content.is_empty() {
        return;
    }

    for (i, line) in content.lines().enumerate() {
        let p = if i == 0 { prefix } else { "" };
        lines.push(Line::from(Span::styled(
            format!("{}{}", p, line),
            Style::default().fg(color),
        )));
    }
    lines.push(Line::from(""));
}

/// Render the focused pane's border decoration.
/// Returns the inner area (content area inside the border) for the caller to render chat into.
pub(super) fn focused_pane_border(f: &mut Frame, app: &App, area: Rect) -> Rect {
    let pane = match app.pane_manager.focused_pane() {
        Some(p) => p,
        None => return area,
    };

    let session_label = pane
        .session_id
        .and_then(|sid| {
            app.sessions.iter().find(|s| s.id == sid).map(|s| {
                s.title
                    .clone()
                    .unwrap_or_else(|| format!("Session {}", &s.id.to_string()[..8]))
            })
        })
        .unwrap_or_else(|| "No session".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 200, 120)))
        .title(Span::styled(
            format!(" {} ", session_label),
            Style::default()
                .fg(Color::Rgb(80, 200, 120))
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(0));

    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}
