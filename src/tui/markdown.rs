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

const TABLE_BORDER: Color = Color::DarkGray;
const TABLE_HEADER: Color = Color::Rgb(120, 120, 120);

/// Parse markdown and convert to styled lines for Ratatui.
///
/// `max_width` is the available content width in columns — used to decide
/// whether tables fit as columns or must collapse to card/row format.
pub fn parse_markdown(markdown: &str, max_width: usize) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(markdown, options);
    let mut lines = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_language = String::new();
    let mut code_content = String::new();
    let mut list_level: u32 = 0;
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
                            Span::styled("╭─ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                code_language.clone(),
                                Style::default()
                                    .fg(Color::Rgb(120, 120, 120))
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(" ─", Style::default().fg(Color::DarkGray)),
                        ]));
                    }
                }
                Tag::List(_) => {
                    list_level += 1;
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
                Tag::Strong | Tag::Emphasis => {}
                Tag::BlockQuote(_) if !current_line.is_empty() => {
                    lines.push(Line::from(std::mem::take(&mut current_line)));
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
                            .fg(Color::Rgb(120, 120, 120))
                            .add_modifier(Modifier::BOLD),
                    )];

                    for span in &mut current_line {
                        *span = span.clone().style(
                            Style::default()
                                .fg(Color::Rgb(120, 120, 120))
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
                                    Style::default().fg(Color::Gray),
                                )));
                            }
                        } else {
                            let highlighted_lines = highlight_code(&code_content, &code_language);
                            lines.extend(highlighted_lines);
                            lines.push(Line::from(Span::styled(
                                "╰────".to_string(),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                    }

                    lines.push(Line::from(""));
                    in_code_block = false;
                    code_language.clear();
                    code_content.clear();
                }
                TagEnd::List(_) => {
                    list_level = list_level.saturating_sub(1);
                    if list_level == 0 {
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::Paragraph => {
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }
                    lines.push(Line::from(""));
                }
                TagEnd::Item if !current_line.is_empty() => {
                    lines.push(Line::from(std::mem::take(&mut current_line)));
                }
                TagEnd::BlockQuote(_) => {
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
                    current_line.push(Span::styled(text_str, Style::default()));
                }
            }

            Event::Code(code) => {
                if in_table {
                    current_cell.push_str(&format!("`{code}`"));
                } else {
                    current_line.push(Span::styled(
                        format!("`{code}`"),
                        Style::default()
                            .fg(Color::Rgb(215, 100, 20))
                            .add_modifier(Modifier::BOLD),
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
            Event::SoftBreak if !current_line.is_empty() => {
                current_line.push(Span::raw(" "));
            }

            Event::Rule => {
                if !current_line.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_line)));
                }
                lines.push(Line::from(Span::styled(
                    "────────────────────────────────────────".to_string(),
                    Style::default().fg(Color::DarkGray),
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
                    current_line.push(Span::styled(html_str, Style::default()));
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

    let border_style = Style::default().fg(TABLE_BORDER);
    let header_style = Style::default()
        .fg(TABLE_HEADER)
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
                    Span::styled(": ", Style::default().fg(Color::DarkGray)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_text() {
        let md = "Hello world";
        let lines = parse_markdown(md, 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_parse_heading() {
        let md = "# Heading 1\n\nSome text";
        let lines = parse_markdown(md, 80);
        assert!(lines.len() > 1);
    }

    #[test]
    fn test_parse_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let lines = parse_markdown(md, 80);
        assert!(lines.len() > 2); // Header, code, footer
    }

    #[test]
    fn test_parse_inline_code() {
        let md = "Use `cargo build` to compile";
        let lines = parse_markdown(md, 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_parse_list() {
        let md = "- Item 1\n- Item 2\n- Item 3";
        let lines = parse_markdown(md, 80);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_parse_horizontal_rule() {
        let md = "Before\n\n---\n\nAfter";
        let lines = parse_markdown(md, 80);
        assert!(lines.len() > 2);
    }

    #[test]
    fn test_empty_markdown() {
        let md = "";
        let lines = parse_markdown(md, 80);
        assert!(lines.is_empty() || lines.iter().all(|l| l.spans.is_empty()));
    }

    #[test]
    fn test_table_wide_columnar() {
        let md = "| name | age |\n|---|---|\n| Alice | 30 |\n| Bob | 25 |";
        let lines = parse_markdown(md, 80);
        // Should contain box-drawing chars
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains('┌'), "Should have top border");
        assert!(text.contains('│'), "Should have cell borders");
        assert!(text.contains('└'), "Should have bottom border");
    }

    #[test]
    fn test_table_narrow_card() {
        let md = "| name | department | location | salary |\n|---|---|---|---|\n| Alice | Engineering | San Francisco | $145,000 |";
        let lines = parse_markdown(md, 30); // Too narrow for table
        // Should render as card format: "Header: Value"
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("name"), "Should have header as label");
        assert!(text.contains(": "), "Should have key:value separator");
        assert!(!text.contains('┌'), "Should NOT have box borders");
    }
}
