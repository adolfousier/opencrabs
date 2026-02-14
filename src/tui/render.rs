//! TUI Rendering
//!
//! Main rendering logic for the terminal interface.

use super::app::App;
use super::events::AppMode;
use super::markdown::parse_markdown;
use super::onboarding_render;
use super::splash;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// Render the entire UI
pub fn render(f: &mut Frame, app: &App) {
    // Show splash screen if in splash mode
    if app.mode == AppMode::Splash {
        splash::render_splash(f, f.area(), app.provider_name(), app.provider_model());
        return;
    }

    // Show onboarding wizard if in onboarding mode
    if app.mode == AppMode::Onboarding {
        if let Some(ref wizard) = app.onboarding {
            onboarding_render::render_onboarding(f, wizard);
        }
        return;
    }

    // Dynamic input height: 3 lines base (1 content + 2 border), grows with content
    let input_line_count = if app.input_buffer.is_empty() {
        1
    } else {
        let terminal_width = f.area().width.saturating_sub(4) as usize; // borders + padding
        app.input_buffer
            .lines()
            .map(|line| {
                if line.is_empty() {
                    1
                } else {
                    // Account for "  " padding prefix
                    (line.len() + 2).div_ceil(terminal_width.max(1))
                }
            })
            .sum::<usize>()
            .max(1)
    };
    let input_height = (input_line_count as u16 + 2).min(10); // +2 for borders, cap at 10

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),            // Header (1 content line + borders)
            Constraint::Min(10),              // Main content
            Constraint::Length(input_height),  // Input (dynamic)
        ])
        .split(f.area());

    // Render components based on mode
    render_header(f, app, chunks[0]);

    // Merge main content + input area for modes that don't need the input box
    let full_content_area = Rect {
        x: chunks[1].x,
        y: chunks[1].y,
        width: chunks[1].width,
        height: chunks[1].height + chunks[2].height,
    };

    match app.mode {
        AppMode::Splash => {
            // Already handled above
        }
        AppMode::Chat => {
            render_chat(f, app, chunks[1]);
            render_input(f, app, chunks[2]);
            // Render slash autocomplete dropdown above the input area
            if app.slash_suggestions_active {
                render_slash_autocomplete(f, app, chunks[2]);
            }
        }
        AppMode::Plan => {
            render_plan(f, app, chunks[1]);
            // Clear the input area (render help text instead)
            render_plan_help(f, chunks[2]);
        }
        AppMode::Sessions => {
            render_sessions(f, app, full_content_area);
        }
        AppMode::Help => {
            render_help(f, app, full_content_area);
        }
        AppMode::Settings => {
            render_settings(f, app, full_content_area);
        }
        AppMode::FilePicker => {
            render_file_picker(f, app, full_content_area);
        }
        AppMode::ModelSelector => {
            render_chat(f, app, chunks[1]);
            render_input(f, app, chunks[2]);
            render_model_selector(f, app, f.area());
        }
        AppMode::UsageDialog => {
            render_chat(f, app, chunks[1]);
            render_input(f, app, chunks[2]);
            render_usage_dialog(f, app, f.area());
        }
        AppMode::RestartPending => {
            render_chat(f, app, chunks[1]);
            render_input(f, app, chunks[2]);
            render_restart_dialog(f, app, f.area());
        }
        AppMode::Onboarding => {
            // Handled by early return above
        }
    }
}

/// Render the header with working directory
fn render_header(f: &mut Frame, app: &App, area: Rect) {
    // Format working directory - show relative or full path
    let working_dir = app.working_directory.to_string_lossy().to_string();
    let display_dir = if working_dir.len() > 60 {
        format!("...{}", &working_dir[working_dir.len() - 57..])
    } else {
        working_dir
    };

    let header_line = Line::from(vec![
        Span::styled(" 📁 ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            display_dir,
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let header = Paragraph::new(vec![header_line]).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " 🦀 OpenCrabs AI Orchestration Agent ",
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(Color::Rgb(70, 130, 180))),
    );

    f.render_widget(header, area);
}

/// Pre-wrap a Line's text content to fit within max_width, preserving the style
/// of the first span and prepending `padding` to each continuation line.
fn wrap_line_with_padding<'a>(line: Line<'a>, max_width: usize, padding: &'a str) -> Vec<Line<'a>> {
    if max_width == 0 {
        return vec![line];
    }
    // Flatten all spans into a single styled string for wrapping
    let total_width: usize = line.spans.iter().map(|s| s.content.len()).sum();
    if total_width <= max_width {
        return vec![line];
    }

    // Collect all text and track style boundaries
    let mut segments: Vec<(String, Style)> = Vec::new();
    for span in &line.spans {
        segments.push((span.content.to_string(), span.style));
    }

    // Build wrapped lines
    let mut result: Vec<Line<'a>> = Vec::new();
    let mut current_spans: Vec<Span<'a>> = Vec::new();
    let mut current_width: usize = 0;

    for (text, style) in segments {
        let mut remaining = text.as_str();
        while !remaining.is_empty() {
            let available = max_width.saturating_sub(current_width);
            if available == 0 {
                result.push(Line::from(current_spans));
                current_spans = vec![Span::styled(padding.to_string(), Style::default())];
                current_width = padding.len();
                continue;
            }

            if remaining.len() <= available {
                current_spans.push(Span::styled(remaining.to_string(), style));
                current_width += remaining.len();
                break;
            } else {
                let break_at = remaining[..available]
                    .rfind(' ')
                    .map(|p| p + 1)
                    .unwrap_or(available);
                let (chunk, rest) = remaining.split_at(break_at);
                current_spans.push(Span::styled(chunk.to_string(), style));
                remaining = rest.trim_start();
                result.push(Line::from(current_spans));
                current_spans = vec![Span::styled(padding.to_string(), Style::default())];
                current_width = padding.len();
            }
        }
    }
    if !current_spans.is_empty() {
        result.push(Line::from(current_spans));
    }
    if result.is_empty() {
        result.push(line);
    }
    result
}

/// Render the chat messages
fn render_chat(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Show banner if there's a pending plan
    if let Some(ref plan) = app.current_plan {
        if matches!(plan.status, crate::tui::plan::PlanStatus::PendingApproval) {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  ⚠️  ", Style::default().fg(Color::Rgb(184, 134, 11))),
                Span::styled(
                    "Plan Pending Approval",
                    Style::default()
                        .fg(Color::Rgb(184, 134, 11))
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled("Press ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "Ctrl+P",
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " to review the plan, or switch to Plan Mode to approve/reject.",
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                "  ─".repeat(30),
                Style::default().fg(Color::Rgb(184, 134, 11)),
            )));
            lines.push(Line::from(""));
        }
    }

    // Get the model name from the current session
    let model_name = app
        .current_session
        .as_ref()
        .and_then(|s| s.model.as_deref())
        .unwrap_or("AI");

    let content_width = area.width.saturating_sub(2) as usize; // borders

    for msg in &app.messages {
        // Render inline approval messages
        if let Some(ref approval) = msg.approval {
            render_inline_approval(&mut lines, approval, content_width);
            lines.push(Line::from(""));
            continue;
        }

        if msg.role == "system" {
            // System messages: compact, DarkGray italic, ℹ️ prefix, no separator
            lines.push(Line::from(vec![
                Span::styled(
                    "  ℹ️  ",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    &msg.content,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        // Add timestamp and role with better formatting
        let timestamp = msg.timestamp.format("%H:%M:%S");

        // Build role text and style
        let (role_text, role_style, prefix) = if msg.role == "user" {
            (
                "You".to_string(),
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
                "  ",
            )
        } else {
            (
                model_name.to_string(),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
                "  ",
            )
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default()),
            Span::styled(role_text, role_style),
            Span::styled(
                format!(" ({})", timestamp),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Parse and render message content as markdown (with left padding)
        let content_lines = parse_markdown(&msg.content);
        for line in content_lines {
            let mut padded_spans = vec![Span::raw("  ")];
            padded_spans.extend(line.spans);
            let padded_line = Line::from(padded_spans);
            for wrapped in wrap_line_with_padding(padded_line, content_width, "  ") {
                lines.push(wrapped);
            }
        }

        // Add spacing between messages
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {}", "─".repeat(58)),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
    }

    // Add streaming response if present
    if let Some(ref response) = app.streaming_response {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", model_name),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("[streaming]", Style::default().fg(Color::DarkGray)),
        ]));

        let streaming_lines = parse_markdown(response);
        for line in streaming_lines {
            let mut padded_spans = vec![Span::raw("  ")];
            padded_spans.extend(line.spans);
            let padded_line = Line::from(padded_spans);
            for wrapped in wrap_line_with_padding(padded_line, content_width, "  ") {
                lines.push(wrapped);
            }
        }
    }

    // Show processing indicator with animated spinner
    if app.is_processing && app.streaming_response.is_none() {
        let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = spinner_frames[app.animation_frame % spinner_frames.len()];

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", frame),
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} is thinking...", model_name),
                Style::default().fg(Color::Rgb(184, 134, 11)),
            ),
        ]));
    }

    // Calculate scroll offset — lines are pre-wrapped so count is accurate
    let total_lines = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize; // Subtract borders
    let max_scroll = total_lines.saturating_sub(visible_height);
    let actual_scroll_offset = max_scroll.saturating_sub(app.scroll_offset);

    let session_name = app
        .current_session
        .as_ref()
        .and_then(|s| s.title.as_deref())
        .unwrap_or("New Session");
    let chat_title = format!(" {} ", session_name);

    let chat = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    chat_title,
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(Color::Rgb(70, 130, 180))),
        )
        .scroll((actual_scroll_offset as u16, 0));

    f.render_widget(chat, area);
}

/// Render the input box
fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let mut input_text = app.input_buffer.clone();

    // Add cursor indicator
    if !app.is_processing {
        input_text.push('█');
    }

    let input_content_width = area.width.saturating_sub(2) as usize; // borders
    let mut input_lines: Vec<Line> = Vec::new();
    for line in input_text.lines() {
        let padded = Line::from(format!("  {}", line));
        for wrapped in wrap_line_with_padding(padded, input_content_width, "  ") {
            input_lines.push(wrapped);
        }
    }
    if input_lines.is_empty() {
        input_lines.push(Line::from("  "));
    }

    let title = if app.is_processing {
        Span::styled(
            " Waiting for response...",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        Span::styled(
            " Type here (enter = send | alt + enter = newline)",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        )
    };

    let border_style = if app.is_processing {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Rgb(70, 130, 180))
    };

    let input = Paragraph::new(input_lines)
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border_style),
        );

    f.render_widget(input, area);
}

/// Render an inline approval request or resolved approval
fn render_inline_approval<'a>(
    lines: &mut Vec<Line<'a>>,
    approval: &super::app::ApprovalData,
    content_width: usize,
) {
    use super::app::{ApprovalOption, ApprovalState};

    match &approval.state {
        ApprovalState::Pending => {
            // Full interactive approval display
            lines.push(Line::from(vec![
                Span::styled(
                    "  \u{1f512} ",
                    Style::default().fg(Color::Rgb(184, 134, 11)),
                ),
                Span::styled(
                    "Tool Permission Required",
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(""));

            // Tool name
            lines.push(Line::from(vec![
                Span::styled("  Tool: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    approval.tool_name.clone(),
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            // Description
            lines.push(Line::from(vec![
                Span::styled("  Description: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    approval.tool_description.clone(),
                    Style::default().fg(Color::White),
                ),
            ]));
            lines.push(Line::from(""));

            // Capabilities
            if !approval.capabilities.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    "  Capabilities:",
                    Style::default().fg(Color::Rgb(184, 134, 11)),
                )]));
                for cap in &approval.capabilities {
                    lines.push(Line::from(vec![
                        Span::styled("     \u{2022} ", Style::default().fg(Color::DarkGray)),
                        Span::styled(cap.clone(), Style::default().fg(Color::Red)),
                    ]));
                }
                lines.push(Line::from(""));
            }

            // Parameters (show details or summary)
            if approval.show_details {
                lines.push(Line::from(vec![Span::styled(
                    "  Parameters (JSON):",
                    Style::default().fg(Color::DarkGray),
                )]));
                let json_str = serde_json::to_string_pretty(&approval.tool_input)
                    .unwrap_or_else(|_| "{}".to_string());
                for line in json_str.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("     ", Style::default()),
                        Span::styled(line.to_string(), Style::default().fg(Color::Blue)),
                    ]));
                }
                lines.push(Line::from(""));
            } else if let Some(obj) = approval.tool_input.as_object() {
                if !obj.is_empty() {
                    lines.push(Line::from(vec![Span::styled(
                        "  Parameters:",
                        Style::default().fg(Color::DarkGray),
                    )]));
                    for (key, value) in obj.iter().take(3) {
                        let value_str = match value {
                            serde_json::Value::String(s) => {
                                if s.len() > 50 {
                                    format!("\"{}...\"", &s[..47])
                                } else {
                                    format!("\"{}\"", s)
                                }
                            }
                            _ => {
                                let s = value.to_string();
                                if s.len() > 50 {
                                    format!("{}...", &s[..47])
                                } else {
                                    s
                                }
                            }
                        };
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("     {}: ", key),
                                Style::default().fg(Color::Rgb(70, 130, 180)),
                            ),
                            Span::styled(value_str, Style::default().fg(Color::White)),
                        ]));
                    }
                    if obj.len() > 3 {
                        lines.push(Line::from(vec![Span::styled(
                            format!("     ... ({} more)", obj.len() - 3),
                            Style::default().fg(Color::DarkGray),
                        )]));
                    }
                    lines.push(Line::from(""));
                }
            }

            // Option descriptions for the selected item
            let option_labels = [
                ("Allow once", "Approve this single tool call"),
                (
                    "Allow all for this task",
                    "Auto-approve all tools this session",
                ),
                (
                    "Allow all moving forward",
                    "Auto-approve all tools permanently",
                ),
            ];

            lines.push(Line::from(vec![Span::styled(
                "  Select an option:",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )]));

            for (i, (label, desc)) in option_labels.iter().enumerate() {
                let is_selected = i == approval.selected_option;
                let prefix = if is_selected { "\u{25b6} " } else { "  " };
                let style = if is_selected {
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {}", prefix), style),
                    Span::styled(label.to_string(), style),
                ]));

                if is_selected {
                    lines.push(Line::from(vec![Span::styled(
                        format!("      {}", desc),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )]));
                }
            }

            lines.push(Line::from(""));

            // Key hints
            lines.push(Line::from(vec![
                Span::styled(
                    "  [\u{2191}\u{2193}]",
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Navigate  ", Style::default().fg(Color::White)),
                Span::styled(
                    "[Enter]",
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Confirm  ", Style::default().fg(Color::White)),
                Span::styled(
                    "[D/Esc]",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Deny  ", Style::default().fg(Color::White)),
                Span::styled(
                    "[V]",
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Details", Style::default().fg(Color::White)),
            ]));

            // Timer
            let time_remaining = approval.requested_at.elapsed();
            let timeout = std::time::Duration::from_secs(300);
            let remaining = timeout.saturating_sub(time_remaining);
            let secs = remaining.as_secs();
            let time_color = if secs < 60 {
                Color::Red
            } else if secs < 180 {
                Color::Rgb(184, 134, 11)
            } else {
                Color::DarkGray
            };
            lines.push(Line::from(vec![Span::styled(
                format!("  {}m {}s remaining", secs / 60, secs % 60),
                Style::default().fg(time_color),
            )]));

            // Separator
            let sep_len = content_width.min(58);
            lines.push(Line::from(Span::styled(
                format!("  {}", "\u{2500}".repeat(sep_len)),
                Style::default().fg(Color::DarkGray),
            )));
        }
        ApprovalState::Approved(option) => {
            // Compact resolved line
            let option_text = match option {
                ApprovalOption::AllowOnce => "Allow once",
                ApprovalOption::AllowForSession => "Allow all for this task",
                ApprovalOption::AllowAlways => "Allow all moving forward",
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{1f512} {} \u{2014} ", approval.tool_name),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
                Span::styled(
                    format!("\u{2713} Approved ({})", option_text),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
        ApprovalState::Denied(reason) => {
            // Compact denied line
            let suffix = if reason.is_empty() {
                String::new()
            } else {
                format!(": {}", reason)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{1f512} {} \u{2014} ", approval.tool_name),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
                Span::styled(
                    format!("\u{2717} Denied{}", suffix),
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }
}

/// Render slash command autocomplete dropdown above the input area
fn render_slash_autocomplete(f: &mut Frame, app: &App, input_area: Rect) {
    let count = app.slash_filtered.len() as u16;
    if count == 0 {
        return;
    }

    // Position dropdown above the input box
    let height = count + 2; // +2 for borders
    let width = 40u16.min(input_area.width);
    let dropdown_area = Rect {
        x: input_area.x + 1,
        y: input_area.y.saturating_sub(height),
        width,
        height,
    };

    // Build dropdown lines (supports both built-in and user-defined commands)
    let lines: Vec<Line> = app
        .slash_filtered
        .iter()
        .enumerate()
        .map(|(i, &cmd_idx)| {
            let name = app.slash_command_name(cmd_idx).unwrap_or("???");
            let desc = app.slash_command_description(cmd_idx).unwrap_or("");
            let is_selected = i == app.slash_selected_index;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let desc_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(70, 130, 180))
            } else {
                Style::default().fg(Color::DarkGray)
            };

            Line::from(vec![
                Span::styled(format!(" {:<10}", name), style),
                Span::styled(format!(" {}", desc), desc_style),
            ])
        })
        .collect();

    // Clear the area and render the dropdown
    f.render_widget(Clear, dropdown_area);
    let dropdown = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(70, 130, 180))),
    );
    f.render_widget(dropdown, dropdown_area);
}

/// Render the sessions list
fn render_sessions(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            "  [↑↓] ",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Navigate  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Enter] ",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Select  ", Style::default().fg(Color::White)),
        Span::styled(
            "[R] ",
            Style::default()
                .fg(Color::Rgb(184, 134, 11))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Rename  ", Style::default().fg(Color::White)),
        Span::styled(
            "[D] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("Delete  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Esc] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("Back", Style::default().fg(Color::White)),
    ]));
    lines.push(Line::from(""));

    for (idx, session) in app.sessions.iter().enumerate() {
        let is_selected = idx == app.selected_session_index;
        let is_current = app
            .current_session
            .as_ref()
            .map(|s| s.id == session.id)
            .unwrap_or(false);

        let is_renaming = is_selected && app.session_renaming;

        let prefix = if is_selected { "  > " } else { "    " };
        let suffix = if is_current { " [current]" } else { "" };

        let name = session.title.as_deref().unwrap_or("Untitled");
        let created = session.created_at.format("%Y-%m-%d %H:%M");

        if is_renaming {
            // Show rename input
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(Color::Rgb(184, 134, 11))),
                Span::styled(
                    format!("{}█", app.session_rename_buffer),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" - {}{}", created, suffix),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        } else {
            let style = if is_selected {
                Style::default()
                    .fg(Color::Rgb(184, 134, 11))
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(Color::Blue)
            } else {
                Style::default().fg(Color::White)
            };

            lines.push(Line::from(Span::styled(
                format!("{}{} - {}{}", prefix, name, created, suffix),
                style,
            )));
        }
    }

    let sessions = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Sessions "))
        .wrap(Wrap { trim: false });

    f.render_widget(sessions, area);
}

/// Render the help screen
fn render_help(f: &mut Frame, app: &App, area: Rect) {
    // Helper to build a "key → description" line
    fn kv<'a>(key: &'a str, desc: &'a str, key_color: Color) -> Line<'a> {
        Line::from(vec![
            Span::styled(
                format!(" {:<14}", key),
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default().fg(Color::DarkGray)),
            Span::styled(desc, Style::default().fg(Color::White)),
        ])
    }

    fn section_header(title: &str) -> Line<'_> {
        Line::from(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ))
    }

    // Split into two columns
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // ── LEFT COLUMN ──
    let gold = Color::Rgb(184, 134, 11);
    let blue = Color::Blue;

    let mut left = vec![
        Line::from(""),
        section_header("GLOBAL"),
        kv("Ctrl+C", "Clear input / quit (2x)", gold),
        kv("Ctrl+N", "New session", gold),
        kv("Ctrl+L", "List sessions", gold),
        kv("Ctrl+K", "Clear session", gold),
        kv("Ctrl+P", "Toggle Plan Mode", gold),
        Line::from(""),
        section_header("CHAT"),
        kv("Enter", "Send message", blue),
        kv("Alt+Enter", "New line", blue),
        kv("Escape (x2)", "Clear input", blue),
        kv("Page Up/Down", "Scroll history", blue),
        kv("@", "File picker", blue),
        Line::from(""),
        section_header("SLASH COMMANDS"),
        kv("/help", "Show this screen", blue),
        kv("/model", "Current model", blue),
        kv("/models", "Switch model", blue),
        kv("/usage", "Token & cost stats", blue),
        kv("/onboard", "Setup wizard", blue),
        kv("/sessions", "Session manager", blue),
        kv("/approve", "Reset approval policy", blue),
        Line::from(""),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [↑↓ PgUp/Dn]",
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Scroll  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Esc]",
                Style::default().fg(gold).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Back", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ];

    // ── RIGHT COLUMN ──
    let mag = Color::Magenta;

    let right = vec![
        Line::from(""),
        section_header("SESSIONS"),
        kv("↑ / ↓", "Navigate", mag),
        kv("Enter", "Load session", mag),
        kv("R", "Rename", mag),
        kv("D", "Delete", mag),
        kv("Esc", "Back to chat", mag),
        Line::from(""),
        section_header("PLAN MODE"),
        kv("Ctrl+A", "Approve & execute", blue),
        kv("Ctrl+R", "Reject plan", blue),
        kv("Ctrl+I", "Request changes", blue),
        kv("↑ / ↓", "Scroll plan", blue),
        Line::from(""),
        section_header("TOOL APPROVAL"),
        kv("↑ / ↓", "Navigate options", blue),
        kv("Enter", "Confirm selection", blue),
        kv("D / Esc", "Deny", Color::Red),
        kv("V", "Toggle details", blue),
        Line::from(""),
        section_header("FEATURES"),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Blue)),
            Span::styled("Markdown & Syntax Highlighting", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Blue)),
            Span::styled("Multi-line Input & Streaming", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Blue)),
            Span::styled("Session Management & History", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Blue)),
            Span::styled("Token & Cost Tracking", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Blue)),
            Span::styled("Plan Mode & Tool Approval", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Blue)),
            Span::styled("Inline Tool Approval (3 policies)", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
    ];

    // Pad left column to match right column length for even rendering
    while left.len() < right.len() {
        left.push(Line::from(""));
    }

    let left_para = Paragraph::new(left)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " 📚 Help & Commands ",
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(Color::Rgb(70, 130, 180))),
        )
        .scroll((app.help_scroll_offset as u16, 0));

    let right_para = Paragraph::new(right)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(70, 130, 180))),
        )
        .scroll((app.help_scroll_offset as u16, 0));

    f.render_widget(left_para, columns[0]);
    f.render_widget(right_para, columns[1]);
}

/// Render help text in the input area during Plan Mode
fn render_plan_help(f: &mut Frame, area: Rect) {
    let help_text = vec![Line::from(vec![
        Span::styled(
            "[Ctrl+A] ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Approve & Execute  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Ctrl+R] ",
            Style::default()
                .fg(Color::Rgb(184, 134, 11))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Reject  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Ctrl+I] ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Request Changes  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Esc] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("Back  ", Style::default().fg(Color::White)),
        Span::styled(
            "[↑↓] ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Scroll", Style::default().fg(Color::White)),
    ])];

    let paragraph = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(70, 130, 180)))
                .title(Span::styled(
                    " Plan Mode - Review & Approve ",
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .alignment(Alignment::Center);

    f.render_widget(paragraph, area);
}

/// Render the plan mode view
#[allow(clippy::vec_init_then_push)]
fn render_plan(f: &mut Frame, app: &App, area: Rect) {
    if let Some(plan) = &app.current_plan {
        // Render the plan document
        let mut lines = vec![];

        // Plan header
        lines.push(Line::from(vec![
            Span::styled("📋 ", Style::default().fg(Color::Rgb(70, 130, 180))),
            Span::styled(
                &plan.title,
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(""));

        // Status
        lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
            Span::styled(plan.status.to_string(), Style::default().fg(Color::Rgb(184, 134, 11))),
        ]));

        lines.push(Line::from(""));

        // Description
        if !plan.description.is_empty() {
            lines.push(Line::from(Span::styled(
                "📝 Description:",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                &plan.description,
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(""));
        }

        // Technical Stack
        if !plan.technical_stack.is_empty() {
            lines.push(Line::from(Span::styled(
                "🛠️  Technical Stack:",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            )));
            for tech in &plan.technical_stack {
                lines.push(Line::from(vec![
                    Span::styled("    • ", Style::default().fg(Color::DarkGray)),
                    Span::styled(tech, Style::default().fg(Color::White)),
                ]));
            }
            lines.push(Line::from(""));
        }

        // Test Strategy
        if !plan.test_strategy.is_empty() {
            lines.push(Line::from(Span::styled(
                "🧪 Test Strategy:",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                &plan.test_strategy,
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(""));
        }

        // Tasks
        lines.push(Line::from(Span::styled(
            format!("📋 Tasks ({}):", plan.tasks.len()),
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        for (idx, task) in plan.tasks.iter().enumerate() {
            // Task line
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", task.status.icon()), Style::default()),
                Span::styled(
                    format!("{}. ", idx + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(&task.title, Style::default().fg(Color::White)),
            ]));

            // Task details (type and complexity)
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled("Type: ", Style::default().fg(Color::DarkGray)),
                Span::styled(task.task_type.to_string(), Style::default().fg(Color::Rgb(70, 130, 180))),
                Span::styled("  |  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Complexity: ", Style::default().fg(Color::DarkGray)),
                Span::styled(task.complexity_stars(), Style::default().fg(Color::Rgb(184, 134, 11))),
            ]));

            // Acceptance Criteria
            if !task.acceptance_criteria.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled("✓ Acceptance Criteria:", Style::default().fg(Color::Blue)),
                ]));
                for criterion in &task.acceptance_criteria {
                    lines.push(Line::from(vec![
                        Span::styled("      • ", Style::default().fg(Color::DarkGray)),
                        Span::styled(criterion, Style::default().fg(Color::White)),
                    ]));
                }
            }

            lines.push(Line::from(""));
        }

        // Action bar
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(vec![
            Span::styled("[Ctrl+A] ", Style::default().fg(Color::Blue)),
            Span::styled("Approve  ", Style::default().fg(Color::White)),
            Span::styled("[Ctrl+R] ", Style::default().fg(Color::Rgb(184, 134, 11))),
            Span::styled("Reject  ", Style::default().fg(Color::White)),
            Span::styled("[Esc] ", Style::default().fg(Color::Red)),
            Span::styled("Cancel", Style::default().fg(Color::White)),
        ]));

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 📋 PLAN MODE ")
                    .border_style(Style::default().fg(Color::Rgb(70, 130, 180))),
            )
            .wrap(Wrap { trim: false })
            .scroll((app.plan_scroll_offset as u16, 0));

        f.render_widget(paragraph, area);
    } else {
        // No plan available
        let text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "📋 Plan Mode",
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "No active plan. Switch to Chat mode to create a plan.",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);

        f.render_widget(paragraph, area);
    }
}

/// Render the settings screen
fn render_settings(f: &mut Frame, app: &App, area: Rect) {
    let settings_text = vec![
        Line::from(Span::styled(
            "Settings",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Coming soon..."),
        Line::from(""),
        Line::from(Span::styled(
            "Press Esc to return",
            Style::default().fg(Color::Rgb(184, 134, 11)),
        )),
    ];

    let settings = Paragraph::new(settings_text)
        .block(Block::default().borders(Borders::ALL).title(" Settings "))
        .alignment(Alignment::Left)
        .scroll((app.help_scroll_offset as u16, 0));

    f.render_widget(settings, area);
}

/// Render the file picker
fn render_file_picker(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            "📁 File Picker",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            app.file_picker_current_dir.to_string_lossy().to_string(),
            Style::default().fg(Color::Rgb(184, 134, 11)),
        ),
    ]));
    lines.push(Line::from(""));

    // Calculate visible range
    let visible_items = (area.height as usize).saturating_sub(6); // Leave space for header and help
    let start = app.file_picker_scroll_offset;
    let end = (start + visible_items).min(app.file_picker_files.len());

    // Render file list
    for (idx, path) in app
        .file_picker_files
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let is_selected = idx == app.file_picker_selected;
        let is_dir = path.is_dir();

        let icon = if path.ends_with("..") {
            "📂 .."
        } else if is_dir {
            "📂"
        } else {
            "📄"
        };

        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD)
        } else if is_dir {
            Style::default().fg(Color::Rgb(70, 130, 180))
        } else {
            Style::default().fg(Color::White)
        };

        let prefix = if is_selected { "▶ " } else { "  " };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(format!("{} {}", icon, filename), style),
        ]));
    }

    // Add scroll indicator if needed
    if app.file_picker_files.len() > visible_items {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "Showing {}-{} of {} files",
                start + 1,
                end,
                app.file_picker_files.len()
            ),
            Style::default().fg(Color::DarkGray),
        )]));
    }

    // Help text
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "[↑↓]",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Navigate  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Enter]",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Select  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Esc]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel", Style::default().fg(Color::White)),
    ]));

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(70, 130, 180)))
                .title(Span::styled(
                    " Select a file ",
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(widget, area);
}

/// Render the model selector dialog (centered overlay)
fn render_model_selector(f: &mut Frame, app: &App, area: Rect) {
    let model_count = app.model_selector_models.len() as u16;
    let dialog_height = (model_count + 6).min(area.height.saturating_sub(4)); // title + padding + footer + borders
    let dialog_width = 60u16.min(area.width.saturating_sub(4));

    // Center the dialog
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(dialog_height),
            Constraint::Min(0),
        ])
        .split(area);
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(dialog_width),
            Constraint::Min(0),
        ])
        .split(v_chunks[1]);
    let dialog_area = h_chunks[1];

    let current_model = app
        .current_session
        .as_ref()
        .and_then(|s| s.model.as_deref())
        .unwrap_or_else(|| app.provider_model());

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (idx, model) in app.model_selector_models.iter().enumerate() {
        let is_selected = idx == app.model_selector_selected;
        let is_active = model == current_model;

        let prefix = if is_selected { " > " } else { "   " };
        let suffix = if is_active { " (active)" } else { "" };

        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD)
        } else if is_active {
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let suffix_style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(70, 130, 180))
        } else {
            Style::default().fg(Color::DarkGray)
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(model.as_str(), style),
            Span::styled(suffix, suffix_style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            " [Enter]",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Select  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Esc]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel", Style::default().fg(Color::White)),
    ]));

    f.render_widget(Clear, dialog_area);
    let dialog = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(70, 130, 180)))
            .title(Span::styled(
                " Select Model ",
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            )),
    );
    f.render_widget(dialog, dialog_area);
}

/// Render the usage stats dialog (centered overlay)
fn render_usage_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog_height = 12u16.min(area.height.saturating_sub(4));
    let dialog_width = 50u16.min(area.width.saturating_sub(4));

    // Center the dialog
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(dialog_height),
            Constraint::Min(0),
        ])
        .split(area);
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(dialog_width),
            Constraint::Min(0),
        ])
        .split(v_chunks[1]);
    let dialog_area = h_chunks[1];

    let session_name = app
        .current_session
        .as_ref()
        .and_then(|s| s.title.as_deref())
        .unwrap_or("New Session");

    let provider = app.provider_name();
    let model = app
        .current_session
        .as_ref()
        .and_then(|s| s.model.as_deref())
        .unwrap_or_else(|| app.provider_model());

    let message_count = app.messages.len();
    let tokens = app.total_tokens();
    let cost = app.total_cost();

    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Session:  ", label_style),
            Span::styled(session_name, value_style),
        ]),
        Line::from(vec![
            Span::styled("  Provider: ", label_style),
            Span::styled(provider, value_style),
        ]),
        Line::from(vec![
            Span::styled("  Model:    ", label_style),
            Span::styled(model, value_style),
        ]),
        Line::from(vec![
            Span::styled("  Messages: ", label_style),
            Span::styled(format!("{}", message_count), value_style),
        ]),
        Line::from(vec![
            Span::styled("  Tokens:   ", label_style),
            Span::styled(format!("{}", tokens), value_style),
        ]),
        Line::from(vec![
            Span::styled("  Cost:     ", label_style),
            Span::styled(format!("${:.4}", cost), value_style),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "         [Esc] Close",
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    f.render_widget(Clear, dialog_area);
    let dialog = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(70, 130, 180)))
            .title(Span::styled(
                " Session Usage ",
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            )),
    );
    f.render_widget(dialog, dialog_area);
}

/// Render restart confirmation dialog
fn render_restart_dialog(f: &mut Frame, app: &App, area: Rect) {
    let status = app
        .rebuild_status
        .as_deref()
        .unwrap_or("Build successful");

    let dialog_height = 8u16;
    let dialog_width = 50u16.min(area.width.saturating_sub(4));

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(dialog_height),
            Constraint::Percentage(40),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min((area.width.saturating_sub(dialog_width)) / 2),
            Constraint::Length(dialog_width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    let dialog_area = horizontal[1];
    f.render_widget(Clear, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", status),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Restart with new binary?"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [Enter] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("Restart  "),
            Span::styled("[Esc] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw("Cancel"),
        ]),
    ];

    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(" Rebuild Complete ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green)),
    );
    f.render_widget(dialog, dialog_area);
}

