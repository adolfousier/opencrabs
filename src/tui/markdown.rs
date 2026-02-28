//! Markdown Rendering
//!
//! Converts markdown text to styled Ratatui widgets.

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::highlight::highlight_code;

/// Parse markdown and convert to styled lines for Ratatui
pub fn parse_markdown(markdown: &str) -> Vec<Line<'static>> {
    let parser = Parser::new(markdown);
    let mut lines = Vec::new();
    let mut current_line = Vec::new();
    let mut in_code_block = false;
    let mut code_language = String::new();
    let mut code_content = String::new();
    let mut list_level: u32 = 0;
    let mut heading_level = 1;

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
                Tag::Strong => {
                    // Bold text - will be handled in text event
                }
                Tag::Emphasis => {
                    // Italic text - will be handled in text event
                }
                Tag::BlockQuote(_) => {
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }
                }
                _ => {}
            },

            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    if !current_line.is_empty() {
                        // Add heading prefix and style
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
                            // Apply heading style to all spans in the line
                            *span = span.clone().style(
                                Style::default()
                                    .fg(Color::Rgb(120, 120, 120))
                                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                            );
                        }

                        styled_line.extend(std::mem::take(&mut current_line));
                        lines.push(Line::from(styled_line));
                        lines.push(Line::from("")); // Add spacing after heading
                    }
                }
                TagEnd::CodeBlock => {
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }

                    // Use syntax highlighting if we have code content
                    if !code_content.is_empty() {
                        let highlighted_lines = if !code_language.is_empty() {
                            highlight_code(&code_content, &code_language)
                        } else {
                            highlight_code(&code_content, "text")
                        };
                        lines.extend(highlighted_lines);
                    }

                    // Add footer if language was specified
                    if !code_language.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "╰────".to_string(),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }

                    lines.push(Line::from("")); // Add spacing after code block
                    in_code_block = false;
                    code_language.clear();
                    code_content.clear();
                }
                TagEnd::List(_) => {
                    list_level = list_level.saturating_sub(1);
                    if list_level == 0 {
                        lines.push(Line::from("")); // Add spacing after list
                    }
                }
                TagEnd::Paragraph => {
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }
                    lines.push(Line::from("")); // Add spacing after paragraph
                }
                TagEnd::Item => {
                    if !current_line.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut current_line)));
                    }
                }
                TagEnd::BlockQuote(_) => {
                    lines.push(Line::from("")); // Add spacing after blockquote
                }
                _ => {}
            },

            Event::Text(text) => {
                let text_str = text.to_string();

                if in_code_block {
                    // Accumulate code content for syntax highlighting
                    code_content.push_str(&text_str);
                } else {
                    // Regular text - add to current line
                    current_line.push(Span::styled(text_str, Style::default()));
                }
            }

            Event::Code(code) => {
                // Inline code
                current_line.push(Span::styled(
                    format!("`{}`", code),
                    Style::default()
                        .fg(Color::Rgb(215, 100, 20))
                        .add_modifier(Modifier::BOLD),
                ));
            }

            Event::HardBreak | Event::SoftBreak => {
                if !current_line.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_line)));
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_text() {
        let md = "Hello world";
        let lines = parse_markdown(md);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_parse_heading() {
        let md = "# Heading 1\n\nSome text";
        let lines = parse_markdown(md);
        assert!(lines.len() > 1);
    }

    #[test]
    fn test_parse_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let lines = parse_markdown(md);
        assert!(lines.len() > 2); // Header, code, footer
    }

    #[test]
    fn test_parse_inline_code() {
        let md = "Use `cargo build` to compile";
        let lines = parse_markdown(md);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_parse_list() {
        let md = "- Item 1\n- Item 2\n- Item 3";
        let lines = parse_markdown(md);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_parse_horizontal_rule() {
        let md = "Before\n\n---\n\nAfter";
        let lines = parse_markdown(md);
        assert!(lines.len() > 2);
    }

    #[test]
    fn test_empty_markdown() {
        let md = "";
        let lines = parse_markdown(md);
        assert!(lines.is_empty() || lines.iter().all(|l| l.spans.is_empty()));
    }
}
