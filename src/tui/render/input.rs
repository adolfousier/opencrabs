//! Input box rendering
//!
//! Text input area with cursor and slash command autocomplete dropdown.

use super::super::app::App;
use super::utils::{format_token_count_raw, wrap_line_with_padding};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
/// Render the input box
pub(super) fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let input_content_width = area.width.saturating_sub(2) as usize; // borders
    let mut input_lines: Vec<Line> = Vec::new();

    // Build input text with cursor highlight on the character (not inserting a block)
    let cursor_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(120, 120, 120));

    if app.input_buffer.is_empty() {
        // Empty input — just show prompt with cursor block
        input_lines.push(Line::from(vec![
            Span::styled("\u{276F} ", Style::default().fg(Color::Rgb(100, 100, 100))),
            Span::styled(" ", cursor_style),
        ]));
    } else {
        // Split input into lines, apply cursor highlight on the char at cursor_position
        let buf = &app.input_buffer;
        let cursor_pos = app.cursor_position;

        // Find which char the cursor is on
        let (before_cursor, cursor_char, after_cursor) = if cursor_pos >= buf.len() {
            // Cursor at end — highlight a space
            (buf.as_str(), None, "")
        } else {
            let next = buf[cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| cursor_pos + i)
                .unwrap_or(buf.len());
            (
                &buf[..cursor_pos],
                Some(&buf[cursor_pos..next]),
                &buf[next..],
            )
        };

        // Build full string with spans per line
        let full_text = format!(
            "{}{}{}",
            before_cursor,
            cursor_char.unwrap_or(""),
            after_cursor
        );

        for (line_idx, line) in full_text.lines().enumerate() {
            // Calculate where this line sits in the overall buffer
            let line_start_in_full: usize =
                full_text.lines().take(line_idx).map(|l| l.len() + 1).sum();
            let line_end_in_full = line_start_in_full + line.len();

            // Check if cursor falls within this line
            let cursor_in_line = cursor_pos >= line_start_in_full && cursor_pos < line_end_in_full;
            let cursor_at_end_of_last_line =
                cursor_pos >= buf.len() && line_idx == full_text.lines().count() - 1;

            let prefix = if line_idx == 0 {
                Span::styled("\u{276F} ", Style::default().fg(Color::Rgb(100, 100, 100)))
            } else {
                Span::raw("  ")
            };

            if cursor_in_line {
                let local_pos = cursor_pos - line_start_in_full;
                let before = &line[..local_pos];
                let (ch, after) = if local_pos < line.len() {
                    let next_boundary = line[local_pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| local_pos + i)
                        .unwrap_or(line.len());
                    (&line[local_pos..next_boundary], &line[next_boundary..])
                } else {
                    (" ", "")
                };
                let padded = Line::from(vec![
                    prefix,
                    Span::raw(before.to_string()),
                    Span::styled(ch.to_string(), cursor_style),
                    Span::raw(after.to_string()),
                ]);
                for wrapped in wrap_line_with_padding(padded, input_content_width, "  ") {
                    input_lines.push(wrapped);
                }
            } else if cursor_at_end_of_last_line {
                let padded = Line::from(vec![
                    prefix,
                    Span::raw(line.to_string()),
                    Span::styled(" ", cursor_style),
                ]);
                for wrapped in wrap_line_with_padding(padded, input_content_width, "  ") {
                    input_lines.push(wrapped);
                }
            } else {
                let padded = Line::from(vec![prefix, Span::raw(line.to_string())]);
                for wrapped in wrap_line_with_padding(padded, input_content_width, "  ") {
                    input_lines.push(wrapped);
                }
            }
        }

        // If cursor is at end of buffer and buffer ends with newline, add cursor on new line
        if cursor_pos >= buf.len() && buf.ends_with('\n') {
            input_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(" ", cursor_style),
            ]));
        }
    }

    let border_style = Style::default().fg(Color::Rgb(120, 120, 120));

    // Context usage indicator (right-side bottom title)
    let context_title = if let Some(input_tok) = app.last_input_tokens {
        let pct = app.context_usage_percent();
        let context_color = if pct > 80.0 {
            Color::Red
        } else if pct > 60.0 {
            Color::Rgb(215, 100, 20)
        } else {
            Color::Cyan
        };
        let ctx_label = format_token_count_raw(input_tok as i32);
        let max_label = format_token_count_raw(app.context_max_tokens as i32);
        let context_label = format!(" ctx: {}/{} ({:.0}%) ", ctx_label, max_label, pct);
        Line::from(Span::styled(
            context_label,
            Style::default()
                .fg(context_color)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Right)
    } else {
        Line::from(Span::styled(
            " Context: – ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Right)
    };

    // Build attachment indicator for the top-right title area
    let attach_title = if !app.attachments.is_empty() {
        let names: Vec<String> = app
            .attachments
            .iter()
            .enumerate()
            .map(|(i, att)| format!("IMG{}:{}", i + 1, att.name))
            .collect();
        Line::from(Span::styled(
            format!(" [{}] ", names.join(" | ")),
            Style::default()
                .fg(Color::Rgb(60, 185, 185))
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Right)
    } else {
        Line::from("")
    };

    let mut block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
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

/// Render slash command autocomplete dropdown above the input area
pub(super) fn render_slash_autocomplete(f: &mut Frame, app: &App, input_area: Rect) {
    let count = app.slash_filtered.len() as u16;
    if count == 0 {
        return;
    }

    // Position dropdown above the input box, auto-sized to fit content
    // Padding: 1 char each side (left/right inside border), 1 empty line top/bottom
    let pad_x: u16 = 1;
    let pad_y: u16 = 1;
    let height = count + 2 + pad_y * 2; // +2 for borders, +2 for top/bottom padding
    let max_content_width = app
        .slash_filtered
        .iter()
        .map(|&idx| {
            let desc = app.slash_command_description(idx).unwrap_or("");
            // pad + " " + 10-char name + " " + desc + " " + pad
            pad_x + 1 + 10 + 1 + desc.len() as u16 + 1 + pad_x
        })
        .max()
        .unwrap_or(40);
    // +2 for borders
    let width = (max_content_width + 2).max(40).min(input_area.width);
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
                    .bg(Color::Rgb(120, 120, 120))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let desc_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(120, 120, 120))
            } else {
                Style::default().fg(Color::DarkGray)
            };

            Line::from(vec![
                Span::styled(format!("  {:<10}", name), style),
                Span::styled(format!(" {} ", desc), desc_style),
            ])
        })
        .collect();

    // Wrap with empty lines for top/bottom padding
    let mut padded_lines = Vec::with_capacity(lines.len() + 2);
    padded_lines.push(Line::from(""));
    padded_lines.extend(lines);
    padded_lines.push(Line::from(""));

    // Clear the area and render the dropdown
    f.render_widget(Clear, dropdown_area);
    let dropdown = Paragraph::new(padded_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(120, 120, 120))),
    );
    f.render_widget(dropdown, dropdown_area);
}

/// Render the single-line status bar below the input box.
///
/// Layout:  provider / model  ·  [policy]          ⠙ OpenCrabs is thinking... (3s)
pub(super) fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let orange = Color::Rgb(215, 100, 20);

    // --- Session name (left) ---
    let session_name = app
        .current_session
        .as_ref()
        .and_then(|s| s.title.as_deref())
        .unwrap_or("Chat")
        .to_string();

    // --- Provider / model ---
    let provider_str = app
        .current_session
        .as_ref()
        .and_then(|s| s.provider_name.clone())
        .unwrap_or_else(|| app.agent_service.provider_name());
    let model_str = app
        .current_session
        .as_ref()
        .and_then(|s| s.model.as_deref())
        .unwrap_or(&app.default_model_name)
        .to_string();

    // Working directory (truncated if long)
    let raw_dir = app.working_directory.to_string_lossy();
    let display_dir = if raw_dir.len() > 40 {
        format!("...{}", &raw_dir[raw_dir.len().saturating_sub(37)..])
    } else {
        raw_dir.to_string()
    };

    let session_text = format!(" {}", session_name);
    let provider_model_dir_text =
        format!("  ·  {} / {}  ·  {}", provider_str, model_str, display_dir);
    let sep_text = "  ·  ";

    // --- Approval policy (centre-left) ---
    let (policy_text, policy_color) = if app.approval_auto_always {
        ("⚡ yolo", Color::Red)
    } else if app.approval_auto_session {
        ("⚡ auto (session)", orange)
    } else {
        ("🔒 approve", Color::DarkGray)
    };

    let spans = vec![
        Span::styled(
            session_text,
            Style::default().fg(orange).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            provider_model_dir_text,
            Style::default().fg(Color::Rgb(90, 110, 150)),
        ),
        Span::styled(sep_text, Style::default().fg(Color::DarkGray)),
        Span::styled(policy_text, Style::default().fg(policy_color)),
    ];

    let line = Line::from(spans);
    let para = Paragraph::new(line).alignment(Alignment::Left);
    f.render_widget(para, area);
}
