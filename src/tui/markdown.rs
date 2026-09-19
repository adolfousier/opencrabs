//! Markdown Rendering
//!
//! Converts markdown text to styled Ratatui widgets.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use unicode_width::UnicodeWidthStr;

use super::highlight::highlight_code;
use crate::tui::render::theme::{self, Role};

/// Chrome: table rules, code-fence borders, the horizontal rule, the dim URL
/// suffix on a link, the `key: ` separator in a collapsed table card.
///
/// These were `Color::DarkGray`, an ANSI index the terminal resolves from its
/// own 16-colour palette, so no theme could move them. That is why table
/// borders rendered identically under every preset (#1634).
fn chrome() -> Color {
    theme::role(Role::GrayDim)
}

fn table_border() -> Color {
    chrome()
}
fn table_header() -> Color {
    theme::role(Role::Gray)
}
/// Dim gray used for list bullets, ordered numbers, and the blockquote gutter.
fn list_marker() -> Color {
    theme::role(Role::Gray)
}
/// Link text color (underlined). Distinct from inline-code amber.
fn link_color() -> Color {
    theme::role(Role::BlueLink)
}
/// Inline code (`` `like this` ``).
///
/// The footer's slate blue, and NOT bold. Inline code appears many times in a
/// single answer, so at the full-saturation brand orange it out-shouted the
/// prose it was embedded in and every other accent on screen. Desaturating the
/// orange only turned it brown, so it borrows a colour the UI already uses for
/// recessive text instead of inventing another warm one.
///
/// Kept a step brighter than the footer's own `Rgb(90, 110, 150)`: chrome can
/// sit back because nobody reads it closely, but inline code carries the
/// identifiers in a sentence and has to stay legible against body text at
/// `Rgb(200, 200, 210)`.
///
/// Less saturated than `link_color()` so the two blues stay distinguishable
/// beyond the link's underline.
///
/// Deliberately local to markdown rendering and not `theme::role(Role::Accent)`: the
/// brand colour still belongs to titles, spinners and selections, which appear
/// once each and are supposed to draw the eye.
fn inline_code() -> Color {
    theme::role(Role::BlueCode)
}

/// Body prose.
///
/// Seeding the emphasis fold with an explicit foreground is what makes plain
/// paragraphs themeable at all. Folding over a bare `Style::default()` shipped
/// every unemphasised span with `fg: None`, which the terminal resolves against
/// its own default foreground, so body text rendered identically under every
/// theme no matter what the palette declared (#1634).
///
/// `patch` keeps this recessive: any inner tag that sets its own colour (links,
/// inline code, headings) still wins, and tags that only set a modifier inherit
/// the prose colour instead of falling back to the terminal's.
fn body_text() -> Color {
    theme::role(Role::TextPrimary)
}

/// Fold an emphasis style stack (bold/italic/strikethrough/link) into a single
/// `Style`. Inner tags `patch` over outer ones so nesting composes (bold inside
/// a link keeps both the underline and the weight).
fn folded_style(stack: &[Style]) -> Style {
    stack
        .iter()
        .fold(Style::default().fg(body_text()), |acc, s| acc.patch(*s))
}

/// Parse markdown and convert to styled lines for Ratatui.
///
/// `max_width` is the available content width in columns — used to decide
/// whether tables fit as columns or must collapse to card/row format.
pub fn parse_markdown(markdown: &str, max_width: usize) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(markdown, options);
    let mut lines = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_language = String::new();
    let mut code_content = String::new();
    // List nesting: one entry per open list. `Some(n)` = ordered list whose
    // NEXT item number is `n`; `None` = unordered (bullet). Depth is the stack
    // length, used for indentation.
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    // Inline emphasis stack (bold / italic / strikethrough / link). Folded into
    // the active style for every text span so nesting composes.
    let mut style_stack: Vec<Style> = Vec::new();
    // URL of the link currently being rendered (appended dimly on close).
    let mut link_url: Option<String> = None;
    // Blockquote nesting depth — adds a `▌` gutter to quoted paragraphs.
    let mut blockquote_depth: u32 = 0;
    let mut heading_level = 1;

    // Table accumulation state
    let mut in_table = false;
    let mut table_headers: Vec<String> = Vec::new();
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_cell = String::new();
    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    heading_level = level as u32;
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    code_language = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };

                    // Add code block header if language is specified
                    if !code_language.is_empty() {
                        if !current_line.is_empty() {
                            lines.push(Line::from(std::mem::take(&mut current_line)));
                        }
                        lines.push(Line::from(vec![
                            Span::styled("╭─ ", Style::default().fg(chrome())),
                            Span::styled(
                                code_language.clone(),
                                Style::default()
                                    .fg(theme::role(Role::Gray))
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(" ─", Style::default().fg(chrome())),
                        ]));
                    }
                }
                Tag::List(first_num) => {
                    // `Some(start)` = ordered list, `None` = bullet list.
                    list_stack.push(first_num);
                }
                Tag::Item => {
                    // Each item starts a fresh visual line led by its marker.
                    // Flush any stray pending content first so the marker leads.
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }
                    let depth = list_stack.len().max(1);
                    let indent = "  ".repeat(depth - 1);
                    let marker = match list_stack.last_mut() {
                        Some(Some(n)) => {
                            let m = format!("{n}. ");
                            *n += 1;
                            m
                        }
                        _ => "• ".to_string(),
                    };
                    current_line.push(Span::styled(
                        format!("{indent}{marker}"),
                        Style::default().fg(list_marker()),
                    ));
                }
                Tag::Table(_alignments) => {
                    in_table = true;
                    table_headers.clear();
                    table_rows.clear();
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }
                }
                Tag::TableHead => {
                    current_row.clear();
                }
                Tag::TableRow => {
                    current_row.clear();
                }
                Tag::TableCell => {
                    current_cell.clear();
                }
                Tag::Strong => {
                    style_stack.push(Style::default().add_modifier(Modifier::BOLD));
                }
                Tag::Emphasis => {
                    style_stack.push(Style::default().add_modifier(Modifier::ITALIC));
                }
                Tag::Strikethrough => {
                    style_stack.push(Style::default().add_modifier(Modifier::CROSSED_OUT));
                }
                Tag::Link { dest_url, .. } => {
                    link_url = Some(dest_url.to_string());
                    style_stack.push(
                        Style::default()
                            .fg(link_color())
                            .add_modifier(Modifier::UNDERLINED),
                    );
                }
                Tag::BlockQuote(_) => {
                    blockquote_depth += 1;
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }
                }
                _ => {}
            },

            Event::End(tag) => match tag {
                TagEnd::Heading(_) if !current_line.is_empty() => {
                    let prefix = match heading_level {
                        1 => "# ",
                        2 => "## ",
                        3 => "### ",
                        _ => "",
                    };

                    let mut styled_line = vec![Span::styled(
                        prefix.to_string(),
                        Style::default()
                            .fg(theme::role(Role::Gray))
                            .add_modifier(Modifier::BOLD),
                    )];

                    for span in &mut current_line {
                        *span = span.clone().style(
                            Style::default()
                                .fg(theme::role(Role::Gray))
                                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                        );
                    }

                    styled_line.extend(std::mem::take(&mut current_line));
                    lines.push(Line::from(styled_line));
                    lines.push(Line::from(""));
                }
                TagEnd::CodeBlock => {
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }

                    if !code_content.is_empty() {
                        let is_plain = code_language.is_empty()
                            || matches!(
                                code_language.as_str(),
                                "text" | "plain" | "plaintext" | "txt"
                            );

                        if is_plain && looks_like_table(&code_content) {
                            // Pipe-style markdown table inside a code block —
                            // re-parse so the table renderer handles it.
                            let table_lines = parse_markdown(&code_content, max_width);
                            lines.extend(table_lines);
                        } else if is_plain
                            && let Some((hdrs, rws)) = parse_box_drawing_table(&code_content)
                        {
                            // Box-drawing table (┌│├└) — extract cells and
                            // render via render_table for responsive layout.
                            render_table(&mut lines, &hdrs, &rws, max_width);
                        } else if is_plain {
                            // Plain text: render without line numbers or
                            // syntax highlighting — just indented gray text.
                            for line_str in code_content.lines() {
                                lines.push(Line::from(Span::styled(
                                    format!("  {line_str}"),
                                    Style::default().fg(theme::role(Role::Gray)),
                                )));
                            }
                        } else {
                            let highlighted_lines = highlight_code(&code_content, &code_language);
                            lines.extend(highlighted_lines);
                            lines.push(Line::from(Span::styled(
                                "╰────".to_string(),
                                Style::default().fg(chrome()),
                            )));
                        }
                    }

                    lines.push(Line::from(""));
                    in_code_block = false;
                    code_language.clear();
                    code_content.clear();
                }
                TagEnd::List(_) => {
                    list_stack.pop();
                    // Blank line only after the OUTERMOST list closes.
                    if list_stack.is_empty() {
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::Paragraph => {
                    if blockquote_depth > 0 && !current_line.is_empty() {
                        current_line
                            .insert(0, Span::styled("▌ ", Style::default().fg(list_marker())));
                    }
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }
                    // Inside a list, keep items tight (no blank line between
                    // each loose-list item); blank-separate only top-level prose.
                    if list_stack.is_empty() {
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::Item if !current_line.is_empty() => {
                    lines.push(Line::from(std::mem::take(&mut current_line)));
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::Link => {
                    style_stack.pop();
                    // Surface the destination so a non-clickable terminal still
                    // shows where a link points, unless the visible text already
                    // contains it (autolinks like <https://x> or [url](url)).
                    if let Some(url) = link_url.take()
                        && !url.is_empty()
                    {
                        let already_shown = current_line
                            .last()
                            .map(|s| s.content.contains(url.as_str()))
                            .unwrap_or(false);
                        if !already_shown {
                            current_line.push(Span::styled(
                                format!(" ({url})"),
                                Style::default().fg(chrome()),
                            ));
                        }
                    }
                }
                TagEnd::BlockQuote(_) => {
                    blockquote_depth = blockquote_depth.saturating_sub(1);
                    lines.push(Line::from(""));
                }
                TagEnd::TableCell => {
                    current_row.push(std::mem::take(&mut current_cell));
                }
                TagEnd::TableHead => {
                    table_headers = std::mem::take(&mut current_row);
                }
                TagEnd::TableRow => {
                    table_rows.push(std::mem::take(&mut current_row));
                }
                TagEnd::Table => {
                    in_table = false;
                    render_table(&mut lines, &table_headers, &table_rows, max_width);
                    table_headers.clear();
                    table_rows.clear();
                    lines.push(Line::from(""));
                }
                _ => {}
            },

            Event::Text(text) => {
                let text_str = text.to_string();

                if in_table {
                    current_cell.push_str(&text_str);
                } else if in_code_block {
                    code_content.push_str(&text_str);
                } else {
                    current_line.push(Span::styled(text_str, folded_style(&style_stack)));
                }
            }

            // Task-list checkbox (`- [x]` / `- [ ]`), emitted right after the
            // item's bullet. Render a styled box so checked/unchecked is visible.
            Event::TaskListMarker(checked) => {
                let (glyph, color) = if checked {
                    ("[x] ", theme::role(Role::GreenCheck))
                } else {
                    ("[ ] ", list_marker())
                };
                current_line.push(Span::styled(glyph, Style::default().fg(color)));
            }

            Event::Code(code) => {
                if in_table {
                    current_cell.push_str(&format!("`{code}`"));
                } else {
                    current_line.push(Span::styled(
                        format!("`{code}`"),
                        Style::default().fg(inline_code()),
                    ));
                }
            }

            Event::HardBreak if !current_line.is_empty() => {
                lines.push(Line::from(std::mem::take(&mut current_line)));
            }

            // CommonMark: a soft break (single newline inside a paragraph)
            // renders as a space so the layout engine can reflow. Treating
            // it as a hard break baked the LLM's 72-col source wrap into
            // chat history, making replies appear narrow on wide terminals.
            //
            // Exception: pipe-row content. When pulldown-cmark fails to
            // recognise text as a markdown table (no `|---|---|` separator
            // after the header), pipe-delimited rows fall through here and
            // the soft-break-as-space turns multiple rows into one giant
            // wrapping line. Detect "current line ended with `|`" and emit
            // a hard break instead so each pseudo-table row stays on its
            // own visual line. Heuristic, but `|` at line boundaries is
            // almost always table syntax — false positives would require
            // legit prose ending one line and starting the next with `|`,
            // which is vanishingly rare.
            Event::SoftBreak if !current_line.is_empty() => {
                let last_ends_with_pipe = current_line
                    .last()
                    .map(|span| span.content.trim_end().ends_with('|'))
                    .unwrap_or(false);
                if last_ends_with_pipe {
                    lines.push(Line::from(std::mem::take(&mut current_line)));
                } else {
                    current_line.push(Span::raw(" "));
                }
            }

            Event::Rule => {
                if !current_line.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_line)));
                }
                lines.push(Line::from(Span::styled(
                    "────────────────────────────────────────".to_string(),
                    Style::default().fg(chrome()),
                )));
                lines.push(Line::from(""));
            }

            // Render HTML/inline-HTML as plain text so tags like <tool_use>
            // mentioned in prose are not silently swallowed.
            Event::Html(html) | Event::InlineHtml(html) => {
                let html_str = html.to_string();
                if in_code_block {
                    code_content.push_str(&html_str);
                } else {
                    current_line.push(Span::styled(html_str, Style::default().fg(body_text())));
                }
            }

            _ => {}
        }
    }

    // Add any remaining content
    if !current_line.is_empty() {
        lines.push(Line::from(current_line));
    }

    // Remove trailing empty lines
    while lines.last().is_some_and(|line| line.spans.is_empty()) {
        lines.pop();
    }

    lines
}

/// Heuristic: does this text look like a markdown table?
/// Detects pipe tables (`| col |`) with a separator (`|---|`).
fn looks_like_table(text: &str) -> bool {
    let mut pipe_lines = 0;
    let mut has_separator = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 2 {
            pipe_lines += 1;
        }
        if trimmed.starts_with('|') && trimmed.contains("---") {
            has_separator = true;
        }
    }
    pipe_lines >= 3 && has_separator
}

/// Try to parse a box-drawing table (┌│├└ or +|-+ ASCII style) into headers
/// and rows. Returns `None` if the text doesn't look like a box-drawing table.
fn parse_box_drawing_table(text: &str) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let mut headers: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        // Skip border/separator lines
        if trimmed.is_empty()
            || trimmed.starts_with('┌')
            || trimmed.starts_with('├')
            || trimmed.starts_with('└')
            || trimmed.starts_with('┬')
            || trimmed.starts_with('┼')
            || trimmed.starts_with('┴')
            || trimmed.starts_with('+')
            || trimmed.chars().all(|c| {
                matches!(
                    c,
                    '─' | '-'
                        | '┬'
                        | '┼'
                        | '┴'
                        | '┌'
                        | '├'
                        | '└'
                        | '┐'
                        | '┤'
                        | '┘'
                        | '+'
                        | ' '
                )
            })
        {
            continue;
        }
        // Data lines: │ cell │ cell │  or  | cell | cell |
        if trimmed.starts_with('│') || trimmed.starts_with('|') {
            let cells: Vec<String> = trimmed
                .split('│')
                .chain(
                    // Also split on ASCII pipe if no box-drawing vertical found
                    if !trimmed.contains('│') {
                        trimmed.split('|').collect::<Vec<_>>()
                    } else {
                        vec![]
                    },
                )
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !cells.is_empty() {
                if headers.is_empty() {
                    headers = cells;
                } else {
                    rows.push(cells);
                }
            }
        }
    }

    if headers.is_empty() || rows.is_empty() {
        return None;
    }
    Some((headers, rows))
}

/// Render a markdown table as either columnar (wide) or card (narrow) format.
///
/// Columnar: box-drawing borders, padded columns, header separator.
/// Card: each row rendered as "Header: Value" lines with horizontal rule separators.
fn render_table(
    lines: &mut Vec<Line<'static>>,
    headers: &[String],
    rows: &[Vec<String>],
    max_width: usize,
) {
    let ncols = headers.len();
    if ncols == 0 {
        return;
    }

    // Calculate column widths using display width (not byte length)
    let mut col_widths: Vec<usize> = (0..ncols)
        .map(|c| {
            let header_w = headers[c].width();
            let max_cell = rows
                .iter()
                .map(|r| r.get(c).map_or(0, |s| s.width()))
                .max()
                .unwrap_or(0);
            header_w.max(max_cell)
        })
        .collect();

    // Total table width: borders + padding (│ cell │ cell │)
    // = 1 (left border) + sum(col_width + 3) for each col (space + content + space + border)
    // But last col doesn't need trailing border counted separately
    let table_width: usize = 1 + col_widths.iter().map(|w| w + 3).sum::<usize>();

    let border_style = Style::default().fg(table_border());
    let header_style = Style::default()
        .fg(table_header())
        .add_modifier(Modifier::BOLD);

    if table_width <= max_width {
        // Distribute extra space proportionally — wider columns get more
        let extra = max_width.saturating_sub(table_width);
        if extra > 0 {
            let total_content: usize = col_widths.iter().sum::<usize>().max(1);
            let mut assigned = 0usize;
            for (i, w) in col_widths.iter_mut().enumerate() {
                let share = if i + 1 == ncols {
                    extra - assigned // last column gets remainder
                } else {
                    extra * *w / total_content
                };
                *w += share;
                assigned += share;
            }
        }

        // ── Columnar format ──
        // Top border: ┌───┬───┐
        let mut top = String::from("┌");
        for (i, w) in col_widths.iter().enumerate() {
            top.push_str(&"─".repeat(w + 2));
            top.push(if i + 1 < ncols { '┬' } else { '┐' });
        }
        lines.push(Line::from(Span::styled(top, border_style)));

        // Header row: │ h1 │ h2 │
        let mut hdr_spans: Vec<Span<'static>> = vec![Span::styled("│", border_style)];
        for (i, h) in headers.iter().enumerate() {
            hdr_spans.push(Span::styled(
                format!(" {:<width$} ", h, width = col_widths[i]),
                header_style,
            ));
            hdr_spans.push(Span::styled("│", border_style));
        }
        lines.push(Line::from(hdr_spans));

        // Header separator: ├───┼───┤
        let mut sep = String::from("├");
        for (i, w) in col_widths.iter().enumerate() {
            sep.push_str(&"─".repeat(w + 2));
            sep.push(if i + 1 < ncols { '┼' } else { '┤' });
        }
        lines.push(Line::from(Span::styled(sep, border_style)));

        // Data rows
        for row in rows {
            let mut row_spans: Vec<Span<'static>> = vec![Span::styled("│", border_style)];
            for (i, w) in col_widths.iter().enumerate() {
                let cell = row.get(i).map_or("", |s| s.as_str());
                row_spans.push(Span::raw(format!(" {:<width$} ", cell, width = *w)));
                row_spans.push(Span::styled("│", border_style));
            }
            lines.push(Line::from(row_spans));
        }

        // Bottom border: └───┴───┘
        let mut bot = String::from("└");
        for (i, w) in col_widths.iter().enumerate() {
            bot.push_str(&"─".repeat(w + 2));
            bot.push(if i + 1 < ncols { '┴' } else { '┘' });
        }
        lines.push(Line::from(Span::styled(bot, border_style)));
    } else {
        // ── Card format (narrow) ──
        // Each row becomes a card: "Header: Value" lines separated by ──
        let max_header_len = headers.iter().map(|h| h.width()).max().unwrap_or(0);

        for (row_idx, row) in rows.iter().enumerate() {
            for (c, header) in headers.iter().enumerate() {
                let value = row.get(c).map_or("", |s| s.as_str());
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:<width$}", header, width = max_header_len),
                        header_style,
                    ),
                    Span::styled(": ", Style::default().fg(chrome())),
                    Span::raw(value.to_string()),
                ]));
            }
            // Separator between cards (not after the last one)
            if row_idx + 1 < rows.len() {
                let rule_len = max_width.min(max_header_len + 30);
                lines.push(Line::from(Span::styled("─".repeat(rule_len), border_style)));
            }
        }
    }
}
