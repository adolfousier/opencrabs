//! Input box rendering
//!
//! Text input area with cursor and slash command autocomplete dropdown.

use super::super::app::App;
use super::utils::format_token_count_raw;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

struct SelConfig<'a> {
    sel_from: usize,
    sel_to: usize,
    selection_style: &'a Style,
    cursor_style: &'a Style,
    cursor_local_byte: Option<usize>,
}

#[allow(clippy::too_many_arguments)]
/// Apply selection highlighting to a visual row's text at the character level.
fn spans_with_selection(
    text: &str,
    row_byte_start: usize,
    row_byte_end: usize,
    sel: &SelConfig<'_>,
    prefix: Span<'static>,
) -> Line<'static> {
    let has_sel = row_byte_end > sel.sel_from && row_byte_start < sel.sel_to;
    let has_cursor = sel.cursor_local_byte.is_some();

    if !has_sel && !has_cursor {
        return Line::from(vec![prefix, Span::raw(text.to_string())]);
    }

    let mut spans: Vec<Span<'static>> = vec![prefix];
    for (idx, ch) in text.char_indices() {
        let global_start = row_byte_start + idx;
        let ch_len = ch.len_utf8();
        let global_end = global_start + ch_len;

        let in_sel = global_start >= sel.sel_from && global_end <= sel.sel_to;
        let is_cursor = sel.cursor_local_byte == Some(idx);

        let span_text = if is_cursor && !in_sel {
            Span::styled(ch.to_string(), *sel.cursor_style)
        } else if in_sel {
            Span::styled(ch.to_string(), *sel.selection_style)
        } else if is_cursor && in_sel {
            Span::styled(
                ch.to_string(),
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(50, 80, 160)),
            )
        } else {
            Span::raw(ch.to_string())
        };
        spans.push(span_text);
    }

    if sel.cursor_local_byte == Some(text.len()) {
        let at_end_sel =
            row_byte_start + text.len() >= sel.sel_from && row_byte_start + text.len() < sel.sel_to;
        if at_end_sel {
            spans.push(Span::styled(" ", *sel.selection_style));
        } else {
            spans.push(Span::styled(" ", *sel.cursor_style));
        }
    }

    Line::from(spans)
}

/// Render the input box
pub(super) fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let input_content_width = area.width.saturating_sub(2) as usize; // borders
    let mut input_lines: Vec<Line> = Vec::new();
    let mut cursor_row: usize = 0;

    let cursor_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(120, 120, 120));
    let selection_style = Style::default()
        .bg(Color::Rgb(50, 80, 160))
        .fg(Color::White);

    // Compute selection byte range from drag coordinates
    let sel_from_to: Option<(usize, usize)> = if app.input_drag_selecting {
        let a = app.input_drag_anchor.unwrap_or((0, 0));
        let b = app.input_drag_current.unwrap_or(a);
        let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
            (a, b)
        } else {
            (b, a)
        };

        let input_top = app.input_area_y;
        let input_content_width = area.width.saturating_sub(2) as usize;
        let content_left = app.input_area_x + 2;

        if input_content_width == 0 || app.input_buffer.is_empty() {
            None
        } else {
            let start_visual = start.1.saturating_sub(input_top + 1) as usize;
            let end_visual = end.1.saturating_sub(input_top + 1) as usize;

            // Build a list of visual rows: each entry is (global_byte_start, global_byte_end)
            let mut visual_rows: Vec<(usize, usize)> = Vec::new();
            let buf = &app.input_buffer;
            let blen = buf.len();
            let mut ls = 0usize;
            while ls <= blen {
                let le = buf[ls..].find('\n').map(|i| ls + i).unwrap_or(blen);
                let line = &buf[ls..le];
                let lw = unicode_width::UnicodeWidthStr::width(line);
                let rows = if lw == 0 {
                    1
                } else {
                    lw.div_ceil(input_content_width)
                };
                for r in 0..rows {
                    let chunk_start = r * input_content_width;
                    let mut display_w = 0;
                    let mut char_start = 0;
                    let mut char_end = line.len();
                    let mut found_start = false;
                    for (idx, ch) in line.char_indices() {
                        let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                        if r > 0 && !found_start && display_w >= chunk_start {
                            char_start = idx;
                            found_start = true;
                        }
                        if display_w + ch_w > chunk_start + input_content_width {
                            char_end = idx;
                            break;
                        }
                        display_w += ch_w;
                    }
                    visual_rows.push((ls + char_start, ls + char_end));
                }
                if le == blen {
                    break;
                }
                ls = le + 1;
            }

            let map_vr_to_byte = |vr: usize, col: u16| -> usize {
                if vr >= visual_rows.len() {
                    return blen;
                }
                let (rs, re) = visual_rows[vr];
                let line_text = &buf[rs..re];
                let target_disp = col.saturating_sub(content_left) as usize;
                let mut w = 0;
                for (idx, ch) in line_text.char_indices() {
                    if w >= target_disp {
                        return rs + idx;
                    }
                    w += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                }
                re
            };

            let from = map_vr_to_byte(start_visual, start.0);
            let to = map_vr_to_byte(end_visual, end.0);
            Some((from.min(to), from.max(to)))
        }
    } else {
        None
    };

    if app.input_buffer.is_empty() {
        input_lines.push(Line::from(vec![
            Span::styled("\u{276F} ", Style::default().fg(Color::Rgb(100, 100, 100))),
            Span::styled(" ", cursor_style),
        ]));
    } else {
        let buf = &app.input_buffer;
        let cursor_pos = app.cursor_position;
        let is_queued = app.queued_message_preview.is_some();

        // Build visual rows from logical lines with wrapping
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

            // Build wrapped visual rows for this logical line
            let lw = unicode_width::UnicodeWidthStr::width(line);
            let num_visual_rows = if lw == 0 {
                1
            } else {
                lw.div_ceil(input_content_width)
            };

            let (sel_from, sel_to) = sel_from_to.unwrap_or((usize::MAX, usize::MAX));

            for vr in 0..num_visual_rows {
                let chunk_start_display = vr * input_content_width;
                let chunk_end_display = chunk_start_display + input_content_width;

                // Find the byte range within `line` that fits this visual row
                let mut disp_w = 0;
                let mut char_start = 0;
                let mut char_end = line.len();
                for (idx, ch) in line.char_indices() {
                    let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    if vr == 0 && disp_w + cw > chunk_end_display {
                        char_end = idx;
                        break;
                    }
                    if vr > 0 {
                        if disp_w < chunk_start_display {
                            char_start = idx;
                        }
                        if disp_w + cw > chunk_end_display {
                            char_end = idx;
                            break;
                        }
                    }
                    disp_w += cw;
                }

                let chunk_text = &line[char_start..char_end];
                let global_chunk_start = line_start + char_start;
                let global_chunk_end = line_start + char_end;

                // Cursor position within this visual row (local byte index)
                let local_cursor = if cursor_in_line {
                    let c = cursor_pos.saturating_sub(line_start);
                    if c >= char_start && c <= char_end {
                        Some(c - char_start)
                    } else if c == line.len() && vr == num_visual_rows - 1 {
                        // Cursor at end of line, show on last visual row
                        Some(chunk_text.len())
                    } else {
                        None
                    }
                } else if cursor_at_end_of_last_line && is_last_line && vr == num_visual_rows - 1 {
                    Some(chunk_text.len())
                } else {
                    None
                };

                let line_sel = SelConfig {
                    sel_from,
                    sel_to,
                    selection_style: &selection_style,
                    cursor_style: &cursor_style,
                    cursor_local_byte: local_cursor,
                };
                let padded_line = spans_with_selection(
                    chunk_text,
                    global_chunk_start,
                    global_chunk_end,
                    &line_sel,
                    if vr == 0 {
                        prefix.clone()
                    } else {
                        Span::raw("  ")
                    },
                );

                input_lines.push(padded_line);

                if local_cursor.is_some() {
                    cursor_row = input_lines.len() - 1;
                }
            }

            if is_last_line {
                break;
            }
            line_start = next_start;
            line_idx += 1;
        }

        // If buffer ends with newline, add a cursor line
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
    // Always read from the session record — every session has both fields.
    let session = app.current_session.as_ref();
    let provider_str = session
        .and_then(|s| s.provider_name.clone())
        .unwrap_or_else(|| app.agent_service.provider_name());
    let model_str = {
        let raw = session.and_then(|s| s.model.as_deref()).unwrap_or_else(|| {
            session
                .and_then(|s| s.provider_name.as_deref())
                .unwrap_or("")
        });
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
        let mut start = short_dir.len().saturating_sub(37);
        while start > 0 && !short_dir.is_char_boundary(start) {
            start -= 1;
        }
        format!("...{}", &short_dir[start..])
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
