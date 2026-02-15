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
use unicode_width::UnicodeWidthStr;

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
                    // Account for "  " padding prefix using display width
                    (UnicodeWidthStr::width(line) + 2).div_ceil(terminal_width.max(1))
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
    let display_dir = if working_dir.width() > 60 {
        // Take the last ~57 display-width chars, ensuring we split at a char boundary
        let suffix_start = char_boundary_at_width_from_end(&working_dir, 57);
        format!("...{}", &working_dir[suffix_start..])
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
    // Use display width (not byte length) for wrapping decisions
    let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
    if total_width <= max_width {
        return vec![line];
    }

    let padding_width = padding.width();

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
                current_width = padding_width;
                continue;
            }

            let remaining_width = remaining.width();
            if remaining_width <= available {
                current_spans.push(Span::styled(remaining.to_string(), style));
                current_width += remaining_width;
                break;
            } else {
                // Find the byte index where cumulative display width reaches `available`
                let byte_limit = char_boundary_at_width(remaining, available);
                // Look for a word break (space) within that range
                let break_at = remaining[..byte_limit]
                    .rfind(' ')
                    .map(|p| p + 1)
                    .unwrap_or(byte_limit);
                let break_at = if break_at == 0 { byte_limit.max(remaining.ceil_char_boundary(1)) } else { break_at };
                let (chunk, rest) = remaining.split_at(break_at);
                current_spans.push(Span::styled(chunk.to_string(), style));
                remaining = rest.trim_start();
                result.push(Line::from(current_spans));
                current_spans = vec![Span::styled(padding.to_string(), Style::default())];
                current_width = padding_width;
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

/// Find the byte index in `s` where the cumulative display width first reaches or exceeds `target_width`.
/// Always returns a valid char boundary.
pub(super) fn char_boundary_at_width(s: &str, target_width: usize) -> usize {
    let mut width = 0;
    for (idx, ch) in s.char_indices() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > target_width {
            return idx;
        }
        width += ch_width;
    }
    s.len()
}

/// Find the byte index to start a suffix of approximately `target_width` display columns.
/// Always returns a valid char boundary.
fn char_boundary_at_width_from_end(s: &str, target_width: usize) -> usize {
    let mut width = 0;
    for (idx, ch) in s.char_indices().rev() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        width += ch_width;
        if width >= target_width {
            return idx;
        }
    }
    0
}

/// Render the chat messages
fn render_chat(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Get the model name: session model → provider default → "AI"
    let model_name = app
        .current_session
        .as_ref()
        .and_then(|s| s.model.as_deref())
        .unwrap_or_else(|| {
            if app.default_model_name.is_empty() { "AI" } else { &app.default_model_name }
        });

    let content_width = area.width.saturating_sub(2) as usize; // borders

    for msg in &app.messages {
        // Render inline approval messages
        if let Some(ref approval) = msg.approval {
            render_inline_approval(&mut lines, approval, content_width);
            lines.push(Line::from(""));
            continue;
        }

        // Render inline plan approval selector
        if let Some(ref plan_approval) = msg.plan_approval {
            render_inline_plan_approval(&mut lines, plan_approval, content_width);
            lines.push(Line::from(""));
            continue;
        }

        // Render /approve policy menu
        if let Some(ref menu) = msg.approve_menu {
            render_approve_menu(&mut lines, menu, content_width);
            lines.push(Line::from(""));
            continue;
        }

        // Render tool call groups (finalized)
        if let Some(ref group) = msg.tool_group {
            render_tool_group(&mut lines, group, false);
            lines.push(Line::from(""));
            continue;
        }

        if msg.role == "system" {
            // System messages: compact, DarkGray italic, no separator
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    &msg.content,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
            ];
            // Show expand/collapse hint if this message has details
            if msg.details.is_some() {
                if msg.expanded {
                    spans.push(Span::styled(
                        " (ctrl+o to collapse)",
                        Style::default().fg(Color::Rgb(100, 100, 100)),
                    ));
                } else {
                    spans.push(Span::styled(
                        " (ctrl+o to expand)",
                        Style::default().fg(Color::Rgb(100, 100, 100)),
                    ));
                }
            }
            lines.push(Line::from(spans));
            // Show expanded details
            if msg.expanded
                && let Some(ref details) = msg.details {
                    for detail_line in details.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("    ", Style::default()),
                            Span::styled(
                                detail_line.to_string(),
                                Style::default().fg(Color::Rgb(120, 120, 120)),
                            ),
                        ]));
                    }
                }
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

    // Render active tool group (live, during processing)
    if let Some(ref group) = app.active_tool_group {
        render_tool_group(&mut lines, group, true);
    }

    let has_pending_approval = app.has_pending_approval();

    // Add streaming response if present (hide when approval is pending)
    if !has_pending_approval && let Some(ref response) = app.streaming_response {
        let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = spinner_frames[app.animation_frame % spinner_frames.len()];

        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", frame),
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} ", model_name),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("is responding...", Style::default().fg(Color::Rgb(184, 134, 11))),
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

    // Show processing indicator with animated spinner (hide when approval is pending)
    if app.is_processing && app.streaming_response.is_none() && !has_pending_approval {
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

    // Show error message if present
    if let Some(ref error) = app.error_message {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "  Error: ",
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                error.clone(),
                Style::default().fg(Color::Red),
            ),
        ]));
        lines.push(Line::from(""));
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

    // Insert cursor block at the current cursor position
    input_text.insert(app.cursor_position, '\u{2588}');

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
            " Processing... (Esc x2 to abort) ",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " Type here (enter = send | alt + enter = newline)",
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        )
    };

    // Always keep steel blue border
    let border_style = Style::default().fg(Color::Rgb(70, 130, 180));

    // Context usage indicator (right-side bottom title)
    let pct = app.context_usage_percent();
    let context_color = if pct > 80.0 {
        Color::Red
    } else if pct > 60.0 {
        Color::Yellow
    } else {
        Color::Green
    };
    let context_label = format!(" Context: {:.0}% ", pct);
    let context_title = Line::from(Span::styled(
        context_label,
        Style::default().fg(context_color).add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Right);

    // Build attachment indicator for the top-right title area
    let attach_title = if !app.attachments.is_empty() {
        let names: Vec<String> = app.attachments.iter().enumerate()
            .map(|(i, att)| format!("IMG{}:{}", i + 1, att.name))
            .collect();
        Line::from(Span::styled(
            format!(" [{}] ", names.join(" | ")),
            Style::default()
                .fg(Color::Rgb(70, 200, 130))
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Right)
    } else {
        Line::from("")
    };

    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(context_title)
        .border_style(border_style);

    if !app.attachments.is_empty() {
        block = block.title(attach_title);
    }

    let input = Paragraph::new(input_lines)
        .style(Style::default().fg(Color::White))
        .block(block);

    f.render_widget(input, area);
}

/// Render a grouped tool call display (● bullet with tree lines)
fn render_tool_group<'a>(
    lines: &mut Vec<Line<'a>>,
    group: &super::app::ToolCallGroup,
    is_active: bool,
) {
    // Header line: ● Processing or ● N tool calls
    let header = if is_active {
        "Processing".to_string()
    } else {
        let count = group.calls.len();
        format!("{} tool call{}", count, if count == 1 { "" } else { "s" })
    };

    let mut header_spans = vec![
        Span::styled(
            format!("  {} {}", "●", header),
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if !group.expanded {
        header_spans.push(Span::styled(
            " (ctrl+o to expand)",
            Style::default().fg(Color::Rgb(100, 100, 100)),
        ));
    }
    lines.push(Line::from(header_spans));

    if group.expanded {
        // Show all calls with tree lines
        for (i, call) in group.calls.iter().enumerate() {
            let connector = if i == group.calls.len() - 1 {
                "└─"
            } else {
                "├─"
            };
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
                Span::styled(
                    "    └─ ".to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(last.description.clone(), style),
            ]));
        }
    }
}

/// Render an inline approval request or resolved approval
fn render_inline_approval<'a>(
    lines: &mut Vec<Line<'a>>,
    approval: &super::app::ApprovalData,
    _content_width: usize,
) {
    use super::app::ApprovalState;

    match &approval.state {
        ApprovalState::Pending => {
            // Line 1: tool description
            let desc = super::app::App::format_tool_description(
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
                && let Some(obj) = approval.tool_input.as_object() {
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
            lines.push(Line::from(vec![
                Span::styled(
                    "  Do you approve?",
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            let options = [("Yes", Color::Green), ("Always", Color::Yellow), ("No", Color::Red)];
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
                        Span::styled(
                            label.to_string(),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
            }
        }
        ApprovalState::Approved(_option) => {
            // Silently skip — tool execution is already shown in the tool group
        }
        ApprovalState::Denied(reason) => {
            let desc = super::app::App::format_tool_description(
                &approval.tool_name,
                &approval.tool_input,
            );
            let suffix = if reason.is_empty() {
                String::new()
            } else {
                format!(": {}", reason)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} -- denied{}", desc, suffix),
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }
}

/// Render an inline plan approval selector (Approve / Reject / Request Changes / View Plan)
fn render_inline_plan_approval<'a>(
    lines: &mut Vec<Line<'a>>,
    plan: &super::app::PlanApprovalData,
    _content_width: usize,
) {
    use super::app::PlanApprovalState;

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
                        Span::styled(
                            label.to_string(),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
            }
        }
        PlanApprovalState::Approved => {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{2705} Plan '{}' approved — executing...", plan.plan_title),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
        PlanApprovalState::Rejected => {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{274C} Plan '{}' rejected", plan.plan_title),
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
        PlanApprovalState::RevisionRequested => {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  \u{1F504} Plan '{}' — revision requested", plan.plan_title),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }
}

/// Render the /approve policy selector menu
fn render_approve_menu<'a>(
    lines: &mut Vec<Line<'a>>,
    menu: &super::app::ApproveMenu,
    _content_width: usize,
) {
    use super::app::ApproveMenuState;

    match &menu.state {
        ApproveMenuState::Pending => {
            let gold = Color::Rgb(255, 200, 50);

            lines.push(Line::from(vec![
                Span::styled(
                    "  TOOL APPROVAL POLICY",
                    Style::default().fg(gold).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(""));

            let options = [
                ("Approve-only", "Always ask before executing tools"),
                ("Allow all (session)", "Auto-approve all tools for this session"),
                ("Yolo mode", "Execute everything without approval until reset"),
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
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
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
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
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
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Policy set: {}", label),
                    Style::default().fg(color).add_modifier(Modifier::ITALIC),
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

        let name = session.title.as_deref().unwrap_or("Untitled");
        let created = session.created_at.format("%Y-%m-%d %H:%M");

        // Format session history size (total stored tokens)
        let history_label = format_token_count(session.token_count);

        // For current session, show live context window usage
        let context_info = if is_current {
            let pct = app.context_usage_percent();
            format!(" [ctx: {:.0}%]", pct)
        } else {
            String::new()
        };

        let current_suffix = if is_current { " *" } else { "" };

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
                    format!(" - {}", created),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        } else {
            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Rgb(184, 134, 11))
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(Color::Blue)
            } else {
                Style::default().fg(Color::White)
            };

            let mut spans = vec![
                Span::styled(format!("{}{}", prefix, name), name_style),
                Span::styled(
                    format!(" - {} ", created),
                    Style::default().fg(Color::DarkGray),
                ),
            ];

            // History size badge
            if session.token_count > 0 {
                spans.push(Span::styled(
                    format!(" {}", history_label),
                    Style::default().fg(Color::Rgb(100, 100, 100)),
                ));
            }

            // Context usage for current session
            if !context_info.is_empty() {
                let ctx_pct = app.context_usage_percent();
                let ctx_color = if ctx_pct > 80.0 {
                    Color::Red
                } else if ctx_pct > 50.0 {
                    Color::Yellow
                } else {
                    Color::Green
                };
                spans.push(Span::styled(context_info, Style::default().fg(ctx_color)));
            }

            // Current marker
            if !current_suffix.is_empty() {
                spans.push(Span::styled(
                    current_suffix,
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ));
            }

            lines.push(Line::from(spans));
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
        kv("/approve", "Tool approval policy", blue),
        kv("/compact", "Compact context now", blue),
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

/// Format a token count as human-readable (e.g. "0", "850", "12.5K", "1.2M")
fn format_token_count(tokens: i32) -> String {
    let tokens = tokens.max(0) as f64;
    if tokens >= 1_000_000.0 {
        format!("{:.1}M tok", tokens / 1_000_000.0)
    } else if tokens >= 1_000.0 {
        format!("{:.1}K tok", tokens / 1_000.0)
    } else if tokens > 0.0 {
        format!("{} tok", tokens as i32)
    } else {
        "new".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── char_boundary_at_width ──────────────────────────────────────

    #[test]
    fn test_char_boundary_ascii() {
        assert_eq!(char_boundary_at_width("hello", 3), 3);
        assert_eq!(char_boundary_at_width("hello", 5), 5);
        assert_eq!(char_boundary_at_width("hello", 10), 5); // past end
    }

    #[test]
    fn test_char_boundary_multibyte() {
        // █ (U+2588) is 3 bytes, 1 display column
        let s = "ab█cd";
        // display widths: a=1, b=1, █=1, c=1, d=1 → total 5
        // byte positions: a=0, b=1, █=2..5, c=5, d=6
        assert_eq!(char_boundary_at_width(s, 2), 2); // after 'b'
        assert_eq!(char_boundary_at_width(s, 3), 5); // after '█'
        assert_eq!(char_boundary_at_width(s, 4), 6); // after 'c'
    }

    #[test]
    fn test_char_boundary_wide_chars() {
        // CJK character '中' is 3 bytes, 2 display columns
        let s = "a中b";
        // display widths: a=1, 中=2, b=1 → total 4
        // byte positions: a=0, 中=1..4, b=4
        assert_eq!(char_boundary_at_width(s, 1), 1); // after 'a'
        assert_eq!(char_boundary_at_width(s, 2), 1); // '中' won't fit in 1 remaining col
        assert_eq!(char_boundary_at_width(s, 3), 4); // after '中'
    }

    #[test]
    fn test_char_boundary_empty() {
        assert_eq!(char_boundary_at_width("", 5), 0);
        assert_eq!(char_boundary_at_width("hello", 0), 0);
    }

    // ── char_boundary_at_width_from_end ─────────────────────────────

    #[test]
    fn test_char_boundary_from_end_ascii() {
        let s = "hello world";
        let idx = char_boundary_at_width_from_end(s, 5);
        assert_eq!(&s[idx..], "world");
    }

    #[test]
    fn test_char_boundary_from_end_multibyte() {
        // "abc██" → display width 5, bytes: a=0, b=1, c=2, █=3..6, █=6..9
        let s = "abc██";
        let idx = char_boundary_at_width_from_end(s, 2); // last 2 display cols = "██"
        assert_eq!(&s[idx..], "██");
    }

    #[test]
    fn test_char_boundary_from_end_wider_than_string() {
        let s = "hi";
        assert_eq!(char_boundary_at_width_from_end(s, 100), 0);
    }

    // ── wrap_line_with_padding ──────────────────────────────────────

    #[test]
    fn test_wrap_ascii_fits() {
        let line = Line::from("short line");
        let result = wrap_line_with_padding(line, 80, "  ");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_wrap_ascii_wraps() {
        let line = Line::from("this is a longer line that should wrap");
        let result = wrap_line_with_padding(line, 20, "  ");
        assert!(result.len() > 1, "expected wrapping, got {} lines", result.len());
    }

    #[test]
    fn test_wrap_multibyte_no_panic() {
        // This is the exact scenario that caused the original panic
        let text = format!("some text with a block char █ at the end{}", "█");
        let line = Line::from(text);
        // Should not panic
        let result = wrap_line_with_padding(line, 30, "  ");
        assert!(!result.is_empty());
    }

    #[test]
    fn test_wrap_emoji_no_panic() {
        let line = Line::from("🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀");
        let result = wrap_line_with_padding(line, 10, "  ");
        assert!(!result.is_empty());
    }

    #[test]
    fn test_wrap_cjk_no_panic() {
        // CJK chars are 2 display columns each
        let line = Line::from("中文测试字符串需要正确换行处理");
        let result = wrap_line_with_padding(line, 10, "  ");
        assert!(result.len() > 1);
    }

    #[test]
    fn test_wrap_mixed_multibyte_and_spaces() {
        let line = Line::from("hello █ world █ test █ more █ text █ end");
        let result = wrap_line_with_padding(line, 15, "  ");
        assert!(result.len() > 1);
        // Verify all lines produce valid strings
        for l in &result {
            let _s: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
        }
    }

    #[test]
    fn test_wrap_zero_width() {
        let line = Line::from("test");
        let result = wrap_line_with_padding(line, 0, "  ");
        assert_eq!(result.len(), 1); // zero width returns original
    }

    #[test]
    fn test_wrap_cursor_char() {
        // Simulates the input buffer with cursor: the exact crash scenario
        let mut input = "next I just noticed something weird like if I keep on this window it is always super fast".to_string();
        input.push('\u{2588}'); // cursor char █
        let line = Line::from(format!("  {}", input));
        let result = wrap_line_with_padding(line, 170, "  ");
        assert!(!result.is_empty());
    }

    // ── format_token_count ────────────────────────────────────────────

    #[test]
    fn test_format_token_count_zero() {
        assert_eq!(format_token_count(0), "new");
    }

    #[test]
    fn test_format_token_count_small() {
        assert_eq!(format_token_count(500), "500 tok");
        assert_eq!(format_token_count(1), "1 tok");
    }

    #[test]
    fn test_format_token_count_thousands() {
        assert_eq!(format_token_count(1000), "1.0K tok");
        assert_eq!(format_token_count(12500), "12.5K tok");
        assert_eq!(format_token_count(150000), "150.0K tok");
    }

    #[test]
    fn test_format_token_count_millions() {
        assert_eq!(format_token_count(1_000_000), "1.0M tok");
        assert_eq!(format_token_count(2_100_000), "2.1M tok");
    }

    #[test]
    fn test_format_token_count_negative() {
        // Negative values clamp to 0
        assert_eq!(format_token_count(-100), "new");
    }
}

