//! Tool call rendering
//!
//! Tool group display, inline approval dialogs, and approval policy menu.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

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

    // Flash the dot while active (slow pulse: ~8 ticks on, ~8 ticks off = ~1.6s cycle)
    let dot = if is_active && (animation_frame / 8).is_multiple_of(2) {
        "○"
    } else {
        "●"
    };

    let mut header_spans = vec![Span::styled(
        format!("  {} {}", dot, header),
        Style::default()
            .fg(Color::Cyan)
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
        // Show all calls with tree lines + full input + details
        let is_last_call = |i: usize| i == group.calls.len() - 1;
        for (i, call) in group.calls.iter().enumerate() {
            let connector = if is_last_call(i) { "└─" } else { "├─" };
            let continuation = if is_last_call(i) { "   " } else { "│  " };
            let in_flight = call.details.is_none();

            let header_style = if call.success || in_flight {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)
            } else {
                Style::default().fg(Color::Red).add_modifier(Modifier::ITALIC)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("    {} ", connector),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(call.description.clone(), header_style),
            ]));

            // Show full tool input parameters (untruncated) below the header
            if !call.tool_input.is_null() {
                if let Some(obj) = call.tool_input.as_object() {
                    for (key, value) in obj.iter() {
                        // Key label
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("    {}  ", continuation),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled(
                                format!("{}:", key),
                                Style::default()
                                    .fg(Color::Rgb(100, 100, 100))
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        // Value — expand strings line by line, cap at 80 lines
                        let value_lines: Vec<String> = match value {
                            serde_json::Value::String(s) => {
                                s.lines().map(|l| l.to_string()).collect()
                            }
                            _ => vec![value.to_string()],
                        };
                        let total = value_lines.len();
                        for vline in value_lines.iter().take(80) {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("    {}    ", continuation),
                                    Style::default().fg(Color::DarkGray),
                                ),
                                Span::styled(
                                    vline.clone(),
                                    Style::default().fg(Color::Rgb(170, 170, 170)),
                                ),
                            ]));
                        }
                        if total > 80 {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("    {}    ", continuation),
                                    Style::default().fg(Color::DarkGray),
                                ),
                                Span::styled(
                                    format!("... ({} more lines)", total - 80),
                                    Style::default()
                                        .fg(Color::Rgb(120, 120, 120))
                                        .add_modifier(Modifier::ITALIC),
                                ),
                            ]));
                        }
                    }
                }
            }

            // If the call is still in-flight, show a running indicator
            if in_flight {
                let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                let frame = spinner_frames[animation_frame % spinner_frames.len()];
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {}  {} ", continuation, frame),
                        Style::default().fg(Color::Rgb(120, 120, 120)),
                    ),
                    Span::styled(
                        "running...",
                        Style::default().fg(Color::Rgb(215, 100, 20)),
                    ),
                ]));
            } else {
                // Show tool output details
                if let Some(ref details) = call.details {
                    let default_detail_style = Style::default().fg(Color::Rgb(90, 90, 90));
                    for detail_line in details.lines().take(30) {
                        let line_style = if detail_line.starts_with("+ ") {
                            Style::default().fg(Color::Rgb(60, 185, 185))
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
            // Header: brief description of what's being requested
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

            // Always show hint so users know V expands full details
            lines.push(Line::from(vec![Span::styled(
                if approval.show_details {
                    "  [V] collapse  [←→] navigate  [Enter] confirm"
                } else {
                    "  [V] expand full details  [←→] navigate  [Enter] confirm"
                },
                Style::default().fg(Color::Rgb(80, 80, 80)),
            )]));

            // Expanded details: show all params fully, no truncation
            if approval.show_details {
                if let Some(obj) = approval.tool_input.as_object() {
                    for (key, value) in obj.iter() {
                        lines.push(Line::from(vec![Span::styled(
                            format!("    {}:", key),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::BOLD),
                        )]));
                        // Build owned lines so there are no borrow conflicts
                        let value_lines: Vec<String> = match value {
                            serde_json::Value::String(s) => {
                                s.lines().map(|l| l.to_string()).collect()
                            }
                            _ => vec![value.to_string()],
                        };
                        let total = value_lines.len();
                        for vline in value_lines.iter().take(60) {
                            lines.push(Line::from(vec![
                                Span::styled("      ", Style::default()),
                                Span::styled(
                                    vline.clone(),
                                    Style::default().fg(Color::Rgb(200, 200, 200)),
                                ),
                            ]));
                        }
                        if total > 60 {
                            lines.push(Line::from(vec![
                                Span::styled("      ", Style::default()),
                                Span::styled(
                                    format!("... ({} more lines)", total - 60),
                                    Style::default()
                                        .fg(Color::Rgb(120, 120, 120))
                                        .add_modifier(Modifier::ITALIC),
                                ),
                            ]));
                        }
                    }
                }
                // Show capabilities if the tool declares any
                if !approval.capabilities.is_empty() {
                    lines.push(Line::from(vec![Span::styled(
                        "    capabilities:",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    )]));
                    lines.push(Line::from(vec![
                        Span::styled("      ", Style::default()),
                        Span::styled(
                            approval.capabilities.join(", "),
                            Style::default().fg(Color::Rgb(215, 100, 20)),
                        ),
                    ]));
                }
                lines.push(Line::from(""));
            }

            // "Do you approve?" + vertical option list with ❯ selector
            // Order: Yes(0), Always(1), No(2)
            lines.push(Line::from(vec![Span::styled(
                "  Do you approve?",
                Style::default().fg(Color::DarkGray),
            )]));
            let options = [
                ("Yes", Color::Cyan),
                ("Always", Color::Rgb(215, 100, 20)),
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

/// Render the /approve policy selector menu
pub(super) fn render_approve_menu<'a>(
    lines: &mut Vec<Line<'a>>,
    menu: &super::super::app::ApproveMenu,
    _content_width: usize,
) {
    use super::super::app::ApproveMenuState;

    match &menu.state {
        ApproveMenuState::Pending => {
            let gold = Color::Rgb(215, 100, 20);

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
                0 => ("Approve-only", Color::Cyan),
                1 => ("Allow all (session)", Color::Rgb(215, 100, 20)),
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
