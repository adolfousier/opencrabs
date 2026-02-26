//! Chat message rendering
//!
//! Main chat view and thinking indicator.

use super::super::app::App;
use super::super::markdown::parse_markdown;
use super::tools::{
    render_approve_menu, render_inline_approval, render_inline_plan_approval, render_tool_group,
};
use super::utils::wrap_line_with_padding;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
};
use unicode_width::UnicodeWidthStr;

/// Render the chat messages
pub(super) fn render_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    let content_width = area.width.saturating_sub(4) as usize; // borders + padding

    // Iterate by index to allow mutable access to render_cache while reading messages
    for msg_idx in 0..app.messages.len() {
        // Render inline approval messages
        if let Some(ref approval) = app.messages[msg_idx].approval {
            render_inline_approval(&mut lines, approval, content_width);
            lines.push(Line::from(""));
            continue;
        }

        // Render inline plan approval selector
        if let Some(ref plan_approval) = app.messages[msg_idx].plan_approval {
            render_inline_plan_approval(&mut lines, plan_approval, content_width);
            lines.push(Line::from(""));
            continue;
        }

        // Render /approve policy menu
        if let Some(ref menu) = app.messages[msg_idx].approve_menu {
            render_approve_menu(&mut lines, menu, content_width);
            lines.push(Line::from(""));
            continue;
        }

        // Render history paging marker
        if app.messages[msg_idx].role == "history_marker" {
            lines.push(Line::from(Span::styled(
                app.messages[msg_idx].content.clone(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
            lines.push(Line::from(""));
            continue;
        }

        // Render tool call groups (finalized)
        if let Some(ref group) = app.messages[msg_idx].tool_group {
            render_tool_group(&mut lines, group, false, app.animation_frame);
            lines.push(Line::from(""));
            continue;
        }

        if app.messages[msg_idx].role == "system" {
            // System messages: visible yellow label, split on newlines so
            // multi-line content actually renders (not clipped to one line).
            let system_style = Style::default()
                .fg(Color::Rgb(200, 170, 60))
                .add_modifier(Modifier::ITALIC);

            for (i, text_line) in app.messages[msg_idx].content.lines().enumerate() {
                let mut spans = vec![Span::styled("  ", Style::default())];
                if i == 0 {
                    spans.push(Span::styled("⚡ ", system_style));
                } else {
                    spans.push(Span::styled("   ", Style::default()));
                }
                spans.push(Span::styled(text_line.to_string(), system_style));

                // Show expand/collapse hint on the first line only
                if i == 0 && app.messages[msg_idx].details.is_some() {
                    let hint = if app.messages[msg_idx].expanded {
                        " (ctrl+o to collapse)"
                    } else {
                        " (ctrl+o to expand)"
                    };
                    spans.push(Span::styled(
                        hint,
                        Style::default().fg(Color::Rgb(120, 120, 120)),
                    ));
                }
                lines.push(Line::from(spans));
            }

            // Show expanded details (e.g. tool output, compaction summary)
            if app.messages[msg_idx].expanded
                && let Some(ref details) = app.messages[msg_idx].details
            {
                for detail_line in details.lines() {
                    // Check for diff lines (+/-) and color accordingly
                    let (style, line_text): (Style, &str) =
                        if let Some(stripped) = detail_line.strip_prefix("+ ") {
                            (
                                Style::default()
                                    .fg(Color::Rgb(80, 200, 120)) // Green for additions
                                    .bg(Color::Rgb(20, 30, 20)), // Dim green bg
                                stripped,
                            )
                        } else if let Some(stripped) = detail_line.strip_prefix("- ") {
                            (
                                Style::default()
                                    .fg(Color::Rgb(255, 100, 100)) // Red for deletions
                                    .bg(Color::Rgb(30, 20, 20)), // Dim red bg
                                stripped,
                            )
                        } else {
                            (
                                Style::default()
                                    .fg(Color::Rgb(180, 180, 180)) // Light gray text
                                    .bg(Color::Rgb(35, 35, 45)), // Gray background for context
                                detail_line,
                            )
                        };

                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(line_text.to_string(), style),
                    ]));
                }
            }
            lines.push(Line::from(""));
            continue;
        }

        // Dot/arrow message differentiation (no role labels needed)
        let is_user = app.messages[msg_idx].role == "user";
        // User messages: subtle lighter background across full line width
        let msg_bg = if is_user {
            Some(Color::Rgb(30, 30, 38))
        } else {
            None
        };

        // Parse and render message content as markdown (cached per message + width)
        let msg_id = app.messages[msg_idx].id;
        let cache_key = (msg_id, content_width as u16);
        if !app.render_cache.contains_key(&cache_key) {
            let parsed = parse_markdown(&app.messages[msg_idx].content);
            app.render_cache.insert(cache_key, parsed);
        }
        let content_lines = app.render_cache[&cache_key].clone();
        for (i, line) in content_lines.into_iter().enumerate() {
            let mut padded_spans = if i == 0 {
                if is_user {
                    // User: arrow prefix
                    vec![Span::styled(
                        "\u{276F} ",
                        Style::default().fg(Color::Rgb(100, 100, 100)),
                    )]
                } else {
                    // Assistant: colored dot prefix
                    vec![Span::styled(
                        "\u{25CF} ",
                        Style::default()
                            .fg(Color::Rgb(70, 130, 180))
                            .add_modifier(Modifier::BOLD),
                    )]
                }
            } else {
                vec![Span::raw("  ")]
            };
            padded_spans.extend(line.spans);
            let padded_line = Line::from(padded_spans);
            for wrapped in wrap_line_with_padding(padded_line, content_width, "  ") {
                if let Some(bg) = msg_bg {
                    // Apply bg to all spans and pad to full line width
                    let mut spans: Vec<Span> = wrapped
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content, s.style.bg(bg)))
                        .collect();
                    let line_width: usize = spans.iter().map(|s| s.content.width()).sum();
                    let remaining = content_width.saturating_sub(line_width);
                    if remaining > 0 {
                        spans.push(Span::styled(" ".repeat(remaining), Style::default().bg(bg)));
                    }
                    lines.push(Line::from(spans));
                } else {
                    lines.push(wrapped);
                }
            }
        }

        // Render reasoning details on assistant messages (collapsible)
        if !is_user && app.messages[msg_idx].details.is_some() {
            lines.push(Line::from(""));
            let hint_text = if app.messages[msg_idx].expanded {
                "  ▾ Thinking (ctrl+o to collapse)"
            } else {
                "  ▸ Thinking (ctrl+o to expand)"
            };
            lines.push(Line::from(vec![Span::styled(
                hint_text.to_string(),
                Style::default()
                    .fg(Color::Rgb(90, 90, 90))
                    .add_modifier(Modifier::ITALIC),
            )]));
            if app.messages[msg_idx].expanded
                && let Some(ref details) = app.messages[msg_idx].details
            {
                lines.push(Line::from(""));
                let reasoning_lines = parse_markdown(details);
                let reasoning_style = Style::default()
                    .fg(Color::Rgb(80, 80, 80))
                    .add_modifier(Modifier::ITALIC);
                for line in reasoning_lines {
                    let mut padded_spans = vec![Span::styled("  ", Style::default())];
                    for span in line.spans {
                        padded_spans.push(Span::styled(span.content.to_string(), reasoning_style));
                    }
                    let padded_line = Line::from(padded_spans);
                    for wrapped in wrap_line_with_padding(padded_line, content_width, "  ") {
                        lines.push(wrapped);
                    }
                }
            }
        }

        // Spacing between messages
        lines.push(Line::from(""));
    }

    let has_pending_approval = app.has_pending_approval();

    // Add streaming response if present (hide when approval is pending)
    if !has_pending_approval && let Some(ref response) = app.streaming_response {
        let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = spinner_frames[app.animation_frame % spinner_frames.len()];

        let elapsed = app
            .processing_started_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);

        let mut spans = vec![
            Span::styled(
                format!("{} ", frame),
                Style::default()
                    .fg(Color::Rgb(70, 130, 180))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "🦀 OpenCrabs ",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "is responding...",
                Style::default().fg(Color::Rgb(184, 134, 11)),
            ),
        ];
        if elapsed > 0 {
            spans.push(Span::styled(
                format!(" ({}s)", elapsed),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));

        // Render reasoning/thinking content above the response text (dimmed style)
        if let Some(ref reasoning) = app.streaming_reasoning {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    "Thinking...",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC | Modifier::BOLD),
                ),
            ]));
            let reasoning_lines = parse_markdown(reasoning);
            let reasoning_style = Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC);
            for line in reasoning_lines {
                let mut padded_spans = vec![Span::styled("  ", Style::default())];
                for span in line.spans {
                    padded_spans.push(Span::styled(span.content.to_string(), reasoning_style));
                }
                let padded_line = Line::from(padded_spans);
                for wrapped in wrap_line_with_padding(padded_line, content_width, "  ") {
                    lines.push(wrapped);
                }
            }
            lines.push(Line::from("")); // separator between reasoning and response
        }

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

    // Render active tool group (live, during processing) — below streaming text
    // so it's always visible at the bottom with auto-scroll
    if let Some(ref group) = app.active_tool_group {
        // Show thinking indicator inline above the tool group
        if app.is_processing && app.streaming_response.is_none() && !app.has_pending_approval() {
            let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame = spinner_frames[app.animation_frame % spinner_frames.len()];
            let elapsed = app
                .processing_started_at
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            let timer_str = if elapsed > 0 {
                format!(" ({}s)", elapsed)
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", frame),
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("OpenCrabs is thinking...{}", timer_str),
                    Style::default().fg(Color::Rgb(184, 134, 11)),
                ),
            ]));
        }
        render_tool_group(&mut lines, group, true, app.animation_frame);
    }

    // Show error message if present
    if let Some(ref error) = app.error_message {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "  Error: ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(error.clone(), Style::default().fg(Color::Red)),
        ]));
        lines.push(Line::from(""));
    }

    // Show sudo password dialog inline (like approval dialogs)
    if let Some(ref sudo_req) = app.sudo_pending {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "  \u{1F512} ",
                Style::default().fg(Color::Rgb(184, 134, 11)),
            ),
            Span::styled(
                "sudo password required",
                Style::default()
                    .fg(Color::Rgb(184, 134, 11))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        // Show the command being run
        let cmd_display = if sudo_req.command.len() > 60 {
            format!("{}...", &sudo_req.command[..57])
        } else {
            sudo_req.command.clone()
        };
        lines.push(Line::from(vec![
            Span::styled("  Command: ", Style::default().fg(Color::DarkGray)),
            Span::styled(cmd_display, Style::default().fg(Color::White)),
        ]));
        // Password input (masked with dots)
        lines.push(Line::from(vec![
            Span::styled("  Password: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "\u{2022}".repeat(app.sudo_input.len()),
                Style::default().fg(Color::White),
            ),
            Span::styled("\u{2588}", Style::default().fg(Color::Rgb(70, 130, 180))),
        ]));
        // Help line
        lines.push(Line::from(vec![
            Span::styled(
                "  [Enter] ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Submit  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Esc] ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cancel", Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(""));
    }

    // Calculate scroll offset — lines are pre-wrapped so count is accurate
    let total_lines = lines.len();
    // Reserve 1 extra line when thinking indicator is visible so it doesn't overlap content
    let thinking_visible =
        app.is_processing && app.streaming_response.is_none() && !app.has_pending_approval();
    let reserved = if thinking_visible { 4 } else { 3 }; // borders + top padding + indicator
    let visible_height = area.height.saturating_sub(reserved) as usize;
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
                .padding(Padding::new(1, 1, 1, 0))
                .title(Span::styled(
                    chat_title,
                    Style::default()
                        .fg(Color::Rgb(70, 130, 180))
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(Color::Rgb(70, 130, 180))),
        )
        .scroll(((actual_scroll_offset.min(u16::MAX as usize)) as u16, 0));

    f.render_widget(chat, area);
}

/// Render the thinking indicator STICKY at bottom of chat (above input field)
/// This ensures users can always see it's responding
pub(super) fn render_thinking_indicator(f: &mut Frame, app: &App, chat_area: Rect) {
    // Only show when processing and no streaming response
    // Skip when active tool group is visible (thinking is rendered inline above it)
    let has_pending_approval = app.has_pending_approval();
    if !app.is_processing
        || app.streaming_response.is_some()
        || has_pending_approval
        || app.active_tool_group.is_some()
    {
        return;
    }

    let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let frame = spinner_frames[app.animation_frame % spinner_frames.len()];

    let elapsed = app
        .processing_started_at
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    let timer_str = if elapsed > 0 {
        format!(" ({}s)", elapsed)
    } else {
        String::new()
    };

    // Create the thinking line
    let thinking_line = Line::from(vec![
        Span::styled(
            format!("{} ", frame),
            Style::default()
                .fg(Color::Rgb(70, 130, 180))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("🦀 OpenCrabs is thinking...{}", timer_str),
            Style::default().fg(Color::Rgb(184, 134, 11)),
        ),
    ]);

    // Render at the bottom of the chat area (sticky)
    // We use a paragraph that's positioned at the bottom of the chat area
    let para = Paragraph::new(thinking_line).style(Style::default().bg(Color::Rgb(20, 20, 28)));

    // Position at bottom of chat area, just above the bottom border
    let indicator_area = Rect {
        x: chat_area.x + 2,
        y: chat_area.y + chat_area.height.saturating_sub(2),
        width: chat_area.width.saturating_sub(4),
        height: 1,
    };

    f.render_widget(para, indicator_area);
}
