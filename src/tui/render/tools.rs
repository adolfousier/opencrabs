//! Tool call rendering
//!
//! Tool group display, inline approval dialogs, and approval policy menu.

use super::utils::char_boundary_at_width;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

/// Render a grouped tool call display (● bullet with tree lines)
pub(super) fn render_tool_group<'a>(
    lines: &mut Vec<Line<'a>>,
    group: &super::super::app::ToolCallGroup,
    is_active: bool,
    animation_frame: usize,
) {
    // Header line: ● Processing: <tool> or ● N tool calls
    let header = if is_active {
        if let Some(last) = group.calls.last() {
            format!("Processing: {}", last.description)
        } else {
            "Processing".to_string()
        }
    } else {
        let count = group.calls.len();
        format!("{} tool call{}", count, if count == 1 { "" } else { "s" })
    };

    // Flash the dot while active (slow pulse: ~3 ticks on, ~3 ticks off)
    let dot = if is_active && (animation_frame / 3).is_multiple_of(2) {
        "○"
    } else {
        "●"
    };

    let mut header_spans = vec![Span::styled(
        format!("  {} {}", dot, header),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )];
    header_spans.push(Span::styled(
        if group.expanded {
            " (ctrl+o to collapse)"
        } else {
            " (ctrl+o to expand)"
        },
        Style::default().fg(Color::Rgb(100, 100, 100)),
    ));
    lines.push(Line::from(header_spans));

    if group.expanded {
        // Show all calls with tree lines + details
        let is_last_call = |i: usize| i == group.calls.len() - 1;
        for (i, call) in group.calls.iter().enumerate() {
            let connector = if is_last_call(i) { "└─" } else { "├─" };
            let style = if call.success {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC)
            } else {
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::ITALIC)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("    {} ", connector),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(call.description.clone(), style),
            ]));

            // Show tool output details below the description
            if let Some(ref details) = call.details {
                let continuation = if is_last_call(i) { "   " } else { "│  " };
                let default_detail_style = Style::default().fg(Color::Rgb(90, 90, 90));
                for detail_line in details.lines().take(30) {
                    // Diff-aware coloring: red for deletions, green for additions
                    let line_style = if detail_line.starts_with("+ ") {
                        Style::default().fg(Color::Rgb(80, 200, 80))
                    } else if detail_line.starts_with("- ") {
                        Style::default().fg(Color::Rgb(220, 80, 80))
                    } else if detail_line.starts_with("@@ ") {
                        Style::default().fg(Color::Cyan)
                    } else {
                        default_detail_style
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("    {}  ", continuation),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(detail_line.to_string(), line_style),
                    ]));
                }
                // Indicate truncation if output is long
                let line_count = details.lines().count();
                if line_count > 30 {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("    {}  ", continuation),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!("... ({} more lines)", line_count - 30),
                            Style::default()
                                .fg(Color::Rgb(120, 120, 120))
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
            }
        }
    } else {
        // Collapsed: show only the last call (rolling wheel effect)
        if let Some(last) = group.calls.last() {
            let style = if last.success {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC)
            } else {
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::ITALIC)
            };
            lines.push(Line::from(vec![
                Span::styled("    └─ ".to_string(), Style::default().fg(Color::DarkGray)),
                Span::styled(last.description.clone(), style),
            ]));
        }
    }
}

/// Render an inline approval request or resolved approval
pub(super) fn render_inline_approval<'a>(
    lines: &mut Vec<Line<'a>>,
    approval: &super::super::app::ApprovalData,
    _content_width: usize,
) {
    use super::super::app::ApprovalState;

    match &approval.state {
        ApprovalState::Pending => {
            // Line 1: tool description
            let desc = super::super::app::App::format_tool_description(
                &approval.tool_name,
                &approval.tool_input,
            );
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    desc,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            // Show params if expanded (V toggle)
            if approval.show_details
                && let Some(obj) = approval.tool_input.as_object()
            {
                for (key, value) in obj.iter().take(5) {
                    let val_str = match value {
                        serde_json::Value::String(s) => {
                            if s.width() > 60 {
                                let end = char_boundary_at_width(s, 57);
                                format!("\"{}...\"", &s[..end])
                            } else {
                                format!("\"{}\"", s)
                            }
                        }
                        _ => {
                            let s = value.to_string();
                            if s.width() > 60 {
                                let end = char_boundary_at_width(&s, 57);
                                format!("{}...", &s[..end])
                            } else {
                                s
                            }
                        }
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("    {}: ", key),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(val_str, Style::default().fg(Color::Rgb(120, 120, 120))),
                    ]));
                }
            }

            // "Do you approve?" + vertical option list with ❯ selector
            // Order: Yes(0), Always(1), No(2)
            lines.push(Line::from(vec![Span::styled(
                "  Do you approve?",
                Style::default().fg(Color::DarkGray),
            )]));
            let options = [
                ("Yes", Color::Green),
                ("Always", Color::Yellow),
                ("No", Color::Red),
            ];
            for (i, (label, color)) in options.iter().enumerate() {
                if i == approval.selected_option {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {} ", "\u{276F}"),
                            Style::default().fg(*color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            label.to_string(),
                            Style::default().fg(*color).add_modifier(Modifier::BOLD),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(label.to_string(), Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }
        }
        ApprovalState::Approved(_option) => {
            // Silently skip — tool execution is already shown in the tool group
        }
        ApprovalState::Denied(reason) => {
            let desc = super::super::app::App::format_tool_description(
                &approval.tool_name,
                &approval.tool_input,
            );
            let suffix = if reason.is_empty() {
                String::new()
            } else {
                format!(": {}", reason)
            };
            lines.push(Line::from(vec![Span::styled(
                format!("  {} -- denied{}", desc, suffix),
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::ITALIC),
            )]));
        }
    }
}

/// Render an inline plan approval selector (Approve / Reject / Request Changes / View Plan)
pub(super) fn render_inline_plan_approval<'a>(
    lines: &mut Vec<Line<'a>>,
    plan: &super::super::app::PlanApprovalData,
    _content_width: usize,
) {
    use super::super::app::PlanApprovalState;

    match &plan.state {
        PlanApprovalState::Pending => {
            // Plan title line
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    "\u{1F4CB} ", // 📋
                    Style::default(),
                ),
                Span::styled(
                    format!("Plan: {}", plan.plan_title),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            // Task count
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} tasks", plan.task_count),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    " (V to show tasks)",
                    Style::default().fg(Color::Rgb(80, 80, 80)),
                ),
            ]));

            // Show task list if expanded
            if plan.show_details {
                for (i, summary) in plan.task_summaries.iter().enumerate() {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("    {}. ", i + 1),
                            Style::default().fg(Color::Rgb(100, 100, 100)),
                        ),
                        Span::styled(
                            summary.clone(),
                            Style::default().fg(Color::Rgb(140, 140, 140)),
                        ),
                    ]));
                }
            }

            // Blank line before options
            lines.push(Line::from(""));

            // Options: Approve(0), Reject(1), Request Changes(2), View Plan(3)
            let options = [
                ("Approve & Execute", Color::Green),
                ("Reject", Color::Red),
                ("Request Changes", Color::Yellow),
                ("View Full Plan", Color::Rgb(70, 130, 180)),
            ];
            for (i, (label, color)) in options.iter().enumerate() {
                if i == plan.selected_option {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {} ", "\u{276F}"), // ❯
                            Style::default().fg(*color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            label.to_string(),
                            Style::default().fg(*color).add_modifier(Modifier::BOLD),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(label.to_string(), Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }
        }
        PlanApprovalState::Approved => {
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  \u{2705} Plan '{}' approved — executing...",
                    plan.plan_title
                ),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::ITALIC),
            )]));
        }
        PlanApprovalState::Rejected => {
            lines.push(Line::from(vec![Span::styled(
                format!("  \u{274C} Plan '{}' rejected", plan.plan_title),
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::ITALIC),
            )]));
        }
        PlanApprovalState::RevisionRequested => {
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  \u{1F504} Plan '{}' — revision requested",
                    plan.plan_title
                ),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::ITALIC),
            )]));
        }
    }
}

/// Render the /approve policy selector menu
pub(super) fn render_approve_menu<'a>(
    lines: &mut Vec<Line<'a>>,
    menu: &super::super::app::ApproveMenu,
    _content_width: usize,
) {
    use super::super::app::ApproveMenuState;

    match &menu.state {
        ApproveMenuState::Pending => {
            let gold = Color::Rgb(255, 200, 50);

            lines.push(Line::from(vec![Span::styled(
                "  TOOL APPROVAL POLICY",
                Style::default().fg(gold).add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(""));

            let options = [
                ("Approve-only", "Always ask before executing tools"),
                (
                    "Allow all (session)",
                    "Auto-approve all tools for this session",
                ),
                (
                    "Yolo mode",
                    "Execute everything without approval until reset",
                ),
            ];

            lines.push(Line::from(Span::styled(
                "  Select a policy:",
                Style::default().fg(Color::Gray),
            )));
            lines.push(Line::from(""));

            for (i, (label, desc)) in options.iter().enumerate() {
                let is_selected = i == menu.selected_option;
                let prefix = if is_selected { "\u{25b6} " } else { "  " };

                let style = if is_selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{}{}", prefix, label), style),
                ]));

                if is_selected {
                    lines.push(Line::from(vec![
                        Span::raw("      "),
                        Span::styled(
                            *desc,
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  [\u{2191}\u{2193}] Navigate  [Enter] Confirm  [Esc] Cancel",
                Style::default().fg(Color::DarkGray),
            )));
        }
        ApproveMenuState::Selected(choice) => {
            let (label, color) = match choice {
                0 => ("Approve-only", Color::Green),
                1 => ("Allow all (session)", Color::Yellow),
                2 => ("Yolo mode", Color::Red),
                _ => ("Cancelled", Color::DarkGray),
            };
            lines.push(Line::from(vec![Span::styled(
                format!("  Policy set: {}", label),
                Style::default().fg(color).add_modifier(Modifier::ITALIC),
            )]));
        }
    }
}
