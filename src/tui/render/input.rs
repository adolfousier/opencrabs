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
    // Visual row (into input_lines) that contains the cursor. Used below
    // to scroll the paragraph so the cursor is always visible.
    let mut cursor_row: usize = 0;

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
        // Walk the buffer by logical lines, tracking the byte offset of each
        // line start so the O(lines²) "take().sum()" per-line recount is gone.
        let buf = &app.input_buffer;
        let cursor_pos = app.cursor_position;
        let is_queued = app.queued_message_preview.is_some();

        // Manually split on '\n' so we preserve the exact byte offsets
        // (buf.lines() strips newlines but its iteration order/positioning
        // makes accumulated offset tracking awkward).
        let mut line_start = 0usize;
        let mut line_idx = 0usize;
        let buf_len = buf.len();
        while line_start <= buf_len {
            let line_end = buf[line_start..]
                .find('\n')
                .map(|i| line_start + i)
                .unwrap_or(buf_len);
            let line = &buf[line_start..line_end];
            let next_start = line_end + 1;
            let is_last_line = line_end == buf_len;

            // Does the cursor sit in this line?
            // - strictly within: cursor_pos in [line_start, line_end)
            // - at end of buffer's last line: cursor_pos == buf_len and this
            //   is the last line
            let cursor_in_line = cursor_pos >= line_start && cursor_pos < line_end;
            let cursor_at_end_of_last_line = cursor_pos >= buf_len && is_last_line;

            let prefix = if line_idx == 0 {
                if is_queued {
                    Span::styled("⏳", Style::default().fg(Color::Rgb(215, 100, 20)))
                } else {
                    Span::styled("\u{276F} ", Style::default().fg(Color::Rgb(100, 100, 100)))
                }
            } else {
                Span::raw("  ")
            };

            let padded = if cursor_in_line {
                let raw_pos = cursor_pos - line_start;
                // Clamp to nearest char boundary to avoid panics from
                // cursor_position landing mid-character (issue #69).
                let local_pos = line.floor_char_boundary(raw_pos.min(line.len()));
                let before = &line[..local_pos];
                // Extract the full grapheme cluster under the cursor, not just
                // one codepoint — ZWJ-joined emoji sequences (🏳️‍🌈, 👨‍👩‍👧, flags)
                // are multiple codepoints glued together and splitting them
                // leaves orphan codepoints that render as fallback glyphs and
                // overflow past the border.
                let (ch, after) = if local_pos < line.len() {
                    use unicode_segmentation::UnicodeSegmentation;
                    let next_boundary = line[local_pos..]
                        .grapheme_indices(true)
                        .nth(1)
                        .map(|(i, _)| local_pos + i)
                        .unwrap_or(line.len());
                    (&line[local_pos..next_boundary], &line[next_boundary..])
                } else {
                    (" ", "")
                };
                Line::from(vec![
                    prefix,
                    Span::raw(before.to_string()),
                    Span::styled(ch.to_string(), cursor_style),
                    Span::raw(after.to_string()),
                ])
            } else if cursor_at_end_of_last_line {
                Line::from(vec![
                    prefix,
                    Span::raw(line.to_string()),
                    Span::styled(" ", cursor_style),
                ])
            } else {
                Line::from(vec![prefix, Span::raw(line.to_string())])
            };

            let before_push = input_lines.len();
            for wrapped in wrap_line_with_padding(padded, input_content_width, "  ") {
                input_lines.push(wrapped);
            }
            if cursor_in_line || cursor_at_end_of_last_line {
                // Land on the LAST wrapped row of this logical line so the
                // cursor is visible when the line soft-wraps past one row.
                cursor_row = input_lines.len().saturating_sub(1).max(before_push);
            }

            if is_last_line {
                break;
            }
            line_start = next_start;
            line_idx += 1;
        }

        // If cursor is at end of buffer and buffer ends with newline, add
        // cursor on a fresh visual row.
        if cursor_pos >= buf_len && buf.ends_with('\n') {
            input_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(" ", cursor_style),
            ]));
            cursor_row = input_lines.len() - 1;
        }
    }

    // Show queued message preview below the input (dimmed, with Up hint)
    if let Some(ref queued) = app.queued_message_preview {
        let flat = queued.replace('\n', " ");
        let max_preview = input_content_width.saturating_sub(25);
        let preview: String = if flat.chars().count() > max_preview {
            let truncated: String = flat.chars().take(max_preview).collect();
            format!("{}...", truncated)
        } else {
            flat
        };
        let dim_style = Style::default().fg(Color::Rgb(100, 100, 100));
        input_lines.push(Line::from(vec![
            Span::styled("  queued: ", dim_style),
            Span::styled(preview, dim_style.add_modifier(Modifier::ITALIC)),
            Span::styled(
                "  (Up to edit)",
                Style::default().fg(Color::Rgb(70, 70, 70)),
            ),
        ]));
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
        let spans: Vec<Span> = app
            .attachments
            .iter()
            .enumerate()
            .flat_map(|(i, _att)| {
                let focused = app.focused_attachment == Some(i);
                let label = format!("Image #{}", i + 1);
                let style = if focused {
                    // Highlight focused attachment — inverted colors
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(60, 185, 185))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Rgb(60, 185, 185))
                        .add_modifier(Modifier::BOLD)
                };
                let mut result = vec![Span::styled(label, style)];
                if i + 1 < app.attachments.len() {
                    result.push(Span::styled(
                        " | ",
                        Style::default().fg(Color::Rgb(60, 185, 185)),
                    ));
                }
                result
            })
            .collect();
        let mut all_spans = vec![Span::styled(
            " [",
            Style::default()
                .fg(Color::Rgb(60, 185, 185))
                .add_modifier(Modifier::BOLD),
        )];
        all_spans.extend(spans);
        all_spans.push(Span::styled(
            "] ",
            Style::default()
                .fg(Color::Rgb(60, 185, 185))
                .add_modifier(Modifier::BOLD),
        ));
        Line::from(all_spans).alignment(Alignment::Right)
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

    // Compute a vertical scroll so the cursor row is always inside the
    // visible viewport (area.height - 2 for top/bottom borders).
    let inner_rows = area.height.saturating_sub(2) as usize;
    let total_rows = input_lines.len();
    let scroll_y: u16 = if inner_rows == 0 || total_rows <= inner_rows {
        0
    } else if cursor_row >= inner_rows {
        // Keep cursor on the last visible row.
        (cursor_row + 1 - inner_rows) as u16
    } else {
        0
    };

    let input = Paragraph::new(input_lines)
        .style(Style::default().fg(Color::Reset))
        .scroll((scroll_y, 0))
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
                    .bg(Color::Gray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Reset)
            };

            let desc_style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Gray)
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

/// Render the emoji picker popup above the input box.
pub(super) fn render_emoji_picker(f: &mut Frame, app: &App, input_area: Rect) {
    let count = app.emoji_filtered.len() as u16;
    if count == 0 {
        return;
    }

    let height = count + 2 + 2; // items + borders + padding
    let width = 36u16.min(input_area.width);
    let dropdown_area = Rect {
        x: input_area.x + 1,
        y: input_area.y.saturating_sub(height),
        width,
        height,
    };

    let lines: Vec<Line> = app
        .emoji_filtered
        .iter()
        .enumerate()
        .map(|(i, &(emoji, shortcode))| {
            let is_selected = i == app.emoji_selected_index;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Gray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Reset)
            };
            let sc_style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Gray)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(vec![
                Span::styled(format!("  {} ", emoji), style),
                Span::styled(format!(":{}: ", shortcode), sc_style),
            ])
        })
        .collect();

    let mut padded = Vec::with_capacity(lines.len() + 2);
    padded.push(Line::from(""));
    padded.extend(lines);
    padded.push(Line::from(""));

    f.render_widget(Clear, dropdown_area);
    let dropdown = Paragraph::new(padded).block(
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
    let model_str = {
        let raw = app
            .current_session
            .as_ref()
            .and_then(|s| s.model.as_deref())
            .unwrap_or(&app.default_model_name);
        // Strip redundant "{provider}/" prefix so we don't render "opencode / opencode/foo"
        let prefix = format!("{}/", provider_str);
        let stripped = raw.strip_prefix(&prefix).unwrap_or(raw);
        crate::tui::provider_selector::model_display_label(stripped).to_string()
    };

    // Working directory — collapse $HOME to ~, then truncate if still long
    let raw_dir = app.working_directory.to_string_lossy();
    let home_dir = dirs::home_dir()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default();
    let short_dir = if !home_dir.is_empty() && raw_dir.starts_with(&home_dir) {
        format!("~{}", &raw_dir[home_dir.len()..])
    } else {
        raw_dir.to_string()
    };
    let display_dir = if short_dir.len() > 40 {
        format!("...{}", &short_dir[short_dir.len().saturating_sub(37)..])
    } else {
        short_dir
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

    let mut spans = vec![
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

    // Split pane indicator
    if app.pane_manager.is_split() {
        let pane_count = app.pane_manager.pane_count();
        let focused_idx = app
            .pane_manager
            .pane_ids_in_order()
            .iter()
            .position(|id| *id == app.pane_manager.focused)
            .map(|i| i + 1)
            .unwrap_or(1);
        spans.push(Span::styled("  ·  ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("[{}/{}]", focused_idx, pane_count),
            Style::default().fg(Color::Rgb(80, 200, 120)),
        ));
    }

    let line = Line::from(spans);
    let para = Paragraph::new(line).alignment(Alignment::Left);
    f.render_widget(para, area);
}
