//! Chat message rendering
//!
//! Main chat view and thinking indicator.

use super::super::app::App;
use super::super::markdown::parse_markdown;
use super::tools::{render_approve_menu, render_inline_approval, render_tool_group};
use super::utils::wrap_line_with_padding;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
};
use unicode_width::UnicodeWidthStr;

/// Render reasoning/thinking text as plain lines, preserving literal newlines.
/// Unlike `parse_markdown`, single `\n` is honoured instead of being collapsed.
pub(crate) fn reasoning_to_lines(text: &str, max_width: usize) -> Vec<Line<'static>> {
    let mut result = Vec::new();
    for l in text.split('\n') {
        let line = Line::from(Span::raw(l.to_string()));
        for wrapped in wrap_line_with_padding(line, max_width, "  ") {
            result.push(wrapped);
        }
    }
    result
}

/// Render the chat messages
pub(super) fn render_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    // Track which message index each rendered line belongs to (for click-to-copy)
    let mut line_to_msg: Vec<Option<usize>> = Vec::new();

    let content_width = area.width.saturating_sub(4) as usize; // borders + padding

    // Iterate by index to allow mutable access to render_cache while reading messages
    for msg_idx in 0..app.messages.len() {
        let lines_before = lines.len();

        // Render inline approval messages
        if let Some(ref approval) = app.messages[msg_idx].approval {
            render_inline_approval(&mut lines, approval, content_width);
            lines.push(Line::from(""));
            line_to_msg.resize(lines.len(), None);
            continue;
        }

        // Render /approve policy menu
        if let Some(ref menu) = app.messages[msg_idx].approve_menu {
            render_approve_menu(&mut lines, menu, content_width);
            lines.push(Line::from(""));
            line_to_msg.resize(lines.len(), None);
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
            line_to_msg.resize(lines.len(), None);
            continue;
        }

        // Render tool call groups (finalized)
        if let Some(ref group) = app.messages[msg_idx].tool_group {
            render_tool_group(&mut lines, group, false, app.animation_frame, content_width);
            lines.push(Line::from(""));
            line_to_msg.resize(lines.len(), None);
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
                let line = Line::from(spans);
                for wrapped in wrap_line_with_padding(line, content_width, "    ") {
                    lines.push(wrapped);
                }
            }

            // Show expanded details (e.g. tool output, compaction summary)
            if app.messages[msg_idx].expanded
                && let Some(ref details) = app.messages[msg_idx].details
            {
                for detail_line in details.lines() {
                    // Check for diff lines (+/-) and color accordingly
                    let (style, line_text): (Style, &str) =
                        if let Some(stripped) = detail_line.strip_prefix("+ ") {
                            (Style::default().fg(Color::Green), stripped)
                        } else if let Some(stripped) = detail_line.strip_prefix("- ") {
                            (Style::default().fg(Color::Red), stripped)
                        } else {
                            (Style::default().fg(Color::DarkGray), detail_line)
                        };

                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(line_text.to_string(), style),
                    ]));
                }
            }
            lines.push(Line::from(""));
            // System messages are mapped to their index for copy
            for _ in lines_before..lines.len() {
                line_to_msg.push(Some(msg_idx));
            }
            continue;
        }

        // Dot/arrow message differentiation (no role labels needed)
        let is_user = app.messages[msg_idx].role == "user";
        // Highlight selected message with subtle background
        let is_selected = app.selected_message_idx == Some(msg_idx);
        let msg_bg: Option<Color> = if is_selected {
            Some(Color::Rgb(40, 45, 55))
        } else if is_user {
            Some(Color::Rgb(40, 44, 56))
        } else {
            None
        };

        // Parse and render message content as markdown (cached per message + width)
        let msg_id = app.messages[msg_idx].id;
        let cache_key = (msg_id, content_width as u16);
        if !app.render_cache.contains_key(&cache_key) {
            let parsed = parse_markdown(
                &app.messages[msg_idx].content,
                content_width.saturating_sub(2),
            );
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
                            .fg(Color::Rgb(120, 120, 120))
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
                    // Apply bg to all spans and pad to full line width.
                    // Force white text on user messages so the dark bg
                    // remains readable on light terminal themes.
                    let mut spans: Vec<Span> = wrapped
                        .spans
                        .into_iter()
                        .map(|s| {
                            let style = if is_user {
                                s.style.bg(bg).fg(Color::White)
                            } else {
                                s.style.bg(bg)
                            };
                            Span::styled(s.content, style)
                        })
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

        // Render reasoning details on assistant messages (collapsible).
        // Only show the label when details actually contain visible content —
        // an empty/whitespace `Some("")` produces a phantom Thinking label
        // that the user can't ever expand into anything useful.
        let has_reasoning = app.messages[msg_idx]
            .details
            .as_ref()
            .is_some_and(|d| !d.trim().is_empty());
        if !is_user && has_reasoning {
            lines.push(Line::from(""));
            let hint_text = if app.messages[msg_idx].expanded {
                "  ▾ Thinking (ctrl+o to collapse)"
            } else {
                "  ▸ Thinking (ctrl+o to expand)"
            };
            // Thinking label — no visible background
            let hint_span = Span::styled(
                hint_text.to_string(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            );
            lines.push(Line::from(vec![hint_span]));
            if app.messages[msg_idx].expanded
                && let Some(ref details) = app.messages[msg_idx].details
            {
                lines.push(Line::from(""));
                // Reserve the outer 2-space indent in the wrap budget so
                // reasoning_to_lines produces lines that already fit
                // `content_width - 2`. Then we prepend the outer "  "
                // prefix and push as-is — no second wrap pass. Previously
                // the caller added the prefix, exceeded content_width,
                // triggered a re-wrap, and broke at wrong word boundaries
                // (e.g. `like.this.and.this` artifacts, inconsistent
                // 2-vs-4-space continuation indent).
                let inner_width = content_width.saturating_sub(2);
                let reasoning_lines = reasoning_to_lines(details, inner_width);
                let reasoning_style = Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC);
                for line in reasoning_lines {
                    let mut padded_spans = vec![Span::styled("  ", Style::default())];
                    for span in line.spans {
                        padded_spans.push(Span::styled(span.content.to_string(), reasoning_style));
                    }
                    lines.push(Line::from(padded_spans));
                }
            }
        }

        // Map all lines from this message to its index
        let msg_lines_end = lines.len();
        for _ in lines_before..msg_lines_end {
            line_to_msg.push(Some(msg_idx));
        }

        // Spacing between messages
        lines.push(Line::from(""));
        line_to_msg.push(None);
    }

    let has_pending_approval = app.has_pending_approval();

    // Add streaming response if present (hide when approval is pending)
    if !has_pending_approval && let Some(ref response) = app.streaming_response {
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
            // See comment at the expanded-message reasoning site above.
            let inner_width = content_width.saturating_sub(2);
            let reasoning_lines = reasoning_to_lines(reasoning, inner_width);
            let reasoning_style = Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC);
            for line in reasoning_lines {
                let mut padded_spans = vec![Span::styled("  ", Style::default())];
                for span in line.spans {
                    padded_spans.push(Span::styled(span.content.to_string(), reasoning_style));
                }
                lines.push(Line::from(padded_spans));
            }
            lines.push(Line::from("")); // separator between reasoning and response
        }

        // Cache parsed markdown for the streaming buffer — re-parse only when
        // the buffer grows. This avoids O(n²) markdown parsing during streaming
        // (previously re-parsed the entire growing buffer on every render frame).
        let current_len = response.len();
        let streaming_lines = if let Some((cached_len, ref cached)) = app.streaming_render_cache {
            if cached_len == current_len {
                cached.clone()
            } else {
                let clean = crate::utils::sanitize::strip_llm_artifacts(response);
                let parsed = parse_markdown(&clean, content_width.saturating_sub(2));
                app.streaming_render_cache = Some((current_len, parsed.clone()));
                parsed
            }
        } else {
            let clean = crate::utils::sanitize::strip_llm_artifacts(response);
            let parsed = parse_markdown(&clean, content_width);
            app.streaming_render_cache = Some((current_len, parsed.clone()));
            parsed
        };
        for line in streaming_lines {
            let mut padded_spans = vec![Span::raw("  ")];
            padded_spans.extend(line.spans);
            let padded_line = Line::from(padded_spans);
            for wrapped in wrap_line_with_padding(padded_line, content_width, "  ") {
                lines.push(wrapped);
            }
        }

        // Blank line to separate content from status spinner
        lines.push(Line::from(""));

        // Spinner at BOTTOM of streaming content so it's always visible
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
                    .fg(Color::Rgb(120, 120, 120))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "🦀 OpenCrabs ",
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "is responding...",
                Style::default().fg(Color::Rgb(215, 100, 20)),
            ),
        ];
        if elapsed > 0 || app.streaming_output_tokens > 0 {
            let mut meta = String::from(" (");
            if elapsed > 0 {
                meta.push_str(&format!("{}s", elapsed));
            }
            if app.streaming_output_tokens > 0 {
                if elapsed > 0 {
                    meta.push_str(" · ");
                }
                meta.push_str(&format!("{} tok", app.streaming_output_tokens));
            }
            meta.push(')');
            spans.push(Span::styled(meta, Style::default().fg(Color::DarkGray)));
        }
        lines.push(Line::from(spans));
    }

    // Render standalone reasoning during thinking-only phase
    // (before first text token — Kimi K2.5, DeepSeek-R1, etc.)
    // streaming_response=None but reasoning is already streaming in
    if !has_pending_approval
        && app.streaming_response.is_none()
        && let Some(ref reasoning) = app.streaming_reasoning
    {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "Thinking...",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC | Modifier::BOLD),
            ),
        ]));
        // See comment at the expanded-message reasoning site above.
        let inner_width = content_width.saturating_sub(2);
        let reasoning_lines = reasoning_to_lines(reasoning, inner_width);
        let reasoning_style = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);
        for rline in reasoning_lines {
            let mut padded_spans = vec![Span::styled("  ", Style::default())];
            for span in rline.spans {
                padded_spans.push(Span::styled(span.content.to_string(), reasoning_style));
            }
            lines.push(Line::from(padded_spans));
        }

        // Blank line to separate reasoning from status spinner
        lines.push(Line::from(""));

        // Spinner at BOTTOM of reasoning content so it's always visible
        let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = spinner_frames[app.animation_frame % spinner_frames.len()];
        let elapsed = app
            .processing_started_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        let mut header_spans = vec![
            Span::styled(
                format!("{} ", frame),
                Style::default()
                    .fg(Color::Rgb(120, 120, 120))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "🦀 OpenCrabs ",
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "is thinking...",
                Style::default().fg(Color::Rgb(215, 100, 20)),
            ),
        ];
        if elapsed > 0 || app.streaming_output_tokens > 0 {
            let mut meta = String::from(" (");
            if elapsed > 0 {
                meta.push_str(&format!("{}s", elapsed));
            }
            if app.streaming_output_tokens > 0 {
                if elapsed > 0 {
                    meta.push_str(" · ");
                }
                meta.push_str(&format!("{} tok", app.streaming_output_tokens));
            }
            meta.push(')');
            header_spans.push(Span::styled(meta, Style::default().fg(Color::DarkGray)));
        }
        lines.push(Line::from(header_spans));
    }

    // Inline "OpenCrabs is thinking..." spinner during tool execution / waiting
    // (no streaming text or reasoning yet). Renders ABOVE the tool group so the
    // user always sees the spinner on top of the processing indicator.
    if !has_pending_approval
        && app.is_processing
        && app.streaming_response.is_none()
        && app.streaming_reasoning.is_none()
    {
        let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = spinner_frames[app.animation_frame % spinner_frames.len()];
        let elapsed = app
            .processing_started_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        let mut spans = vec![
            Span::styled(
                format!("  {} ", frame),
                Style::default()
                    .fg(Color::Rgb(120, 120, 120))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "OpenCrabs is thinking...",
                Style::default().fg(Color::Rgb(215, 100, 20)),
            ),
        ];
        if elapsed > 0 || app.streaming_output_tokens > 0 {
            let mut meta = String::new();
            if elapsed > 0 {
                meta.push_str(&format!(" {}s", elapsed));
            }
            if app.streaming_output_tokens > 0 {
                if elapsed > 0 {
                    meta.push_str(" ·");
                }
                meta.push_str(&format!(" {} tok", app.streaming_output_tokens));
            }
            spans.push(Span::styled(
                meta,
                Style::default().fg(Color::Rgb(100, 100, 100)),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Render active tool group (live, during processing) — below spinner
    // so it's always visible at the bottom with auto-scroll
    if let Some(ref group) = app.active_tool_group {
        render_tool_group(&mut lines, group, true, app.animation_frame, content_width);
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

    // Show notification if present (auto-dismiss after 2s)
    if let Some(ref note) = app.notification {
        if app
            .notification_shown_at
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2))
        {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    note.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(""));
        } else {
            app.notification = None;
            app.notification_shown_at = None;
        }
    }

    // Show sudo password dialog inline (like approval dialogs)
    if let Some(ref sudo_req) = app.sudo_pending {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "  \u{1F512} ",
                Style::default().fg(Color::Rgb(215, 100, 20)),
            ),
            Span::styled(
                "sudo password required",
                Style::default()
                    .fg(Color::Rgb(215, 100, 20))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        // Show the command being run
        let cmd_display = if sudo_req.command.len() > 60 {
            format!(
                "{}...",
                sudo_req.command.chars().take(57).collect::<String>()
            )
        } else {
            sudo_req.command.clone()
        };
        lines.push(Line::from(vec![
            Span::styled("  Command: ", Style::default().fg(Color::DarkGray)),
            Span::styled(cmd_display, Style::default().fg(Color::Reset)),
        ]));
        // Password input (masked with dots)
        lines.push(Line::from(vec![
            Span::styled("  Password: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "\u{2022}".repeat(app.sudo_input.len()),
                Style::default().fg(Color::Reset),
            ),
            Span::styled("\u{2588}", Style::default().fg(Color::Rgb(120, 120, 120))),
        ]));
        // Help line
        lines.push(Line::from(vec![
            Span::styled(
                "  [Enter] ",
                Style::default()
                    .fg(Color::Cyan)
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

    // Pad line_to_msg for any remaining lines (streaming, errors, etc.)
    line_to_msg.resize(lines.len(), None);
    app.chat_line_to_msg = line_to_msg;

    // Calculate scroll offset — lines are pre-wrapped so count is accurate
    let total_lines = lines.len();
    // Only 1 row of top padding (Borders::NONE + Padding::new(1,1,1,0)); no border rows
    let visible_height = area.height.saturating_sub(1) as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let actual_scroll_offset = max_scroll.saturating_sub(app.scroll_offset);

    // Store render info for click-to-copy + drag-selection coordinate mapping
    app.chat_render_scroll = actual_scroll_offset;
    app.chat_area_y = area.y;
    app.chat_area_x = area.x;
    app.chat_area_width = area.width;
    app.chat_area_height = area.height;
    // Snapshot plain-text of each rendered line for drag-select extraction.
    app.chat_rendered_lines = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect();

    let chat = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::NONE)
                .padding(Padding::new(1, 1, 1, 0)),
        )
        .scroll(((actual_scroll_offset.min(u16::MAX as usize)) as u16, 0));

    // Clear the area first to prevent stale buffer content from bleeding through.
    // Ratatui's Paragraph only writes cells where it has text; cells beyond line
    // ends or below the last line retain old content from the double-buffer.
    f.render_widget(Clear, area);
    f.render_widget(chat, area);

    // Drag-selection highlight overlay: invert fg/bg for cells inside the
    // selection rectangle (reading-order), clipped to the chat area.
    if let (Some(a), Some(b)) = (app.drag_anchor, app.drag_current) {
        let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
            (a, b)
        } else {
            (b, a)
        };
        let buf = f.buffer_mut();
        let x0 = area.x;
        let y0 = area.y;
        let x1 = area.x.saturating_add(area.width);
        let y1 = area.y.saturating_add(area.height);
        let highlight = Style::default().add_modifier(Modifier::REVERSED);
        if start.1 == end.1 {
            let y = start.1;
            if y >= y0 && y < y1 {
                let sx = start.0.max(x0);
                let ex = end.0.min(x1.saturating_sub(1));
                for x in sx..=ex {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_style(highlight);
                    }
                }
            }
        } else {
            for y in start.1..=end.1 {
                if y < y0 || y >= y1 {
                    continue;
                }
                let sx = if y == start.1 { start.0 } else { x0 };
                let ex = if y == end.1 {
                    end.0.min(x1.saturating_sub(1))
                } else {
                    x1.saturating_sub(1)
                };
                for x in sx.max(x0)..=ex {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_style(highlight);
                    }
                }
            }
        }
    }
}
