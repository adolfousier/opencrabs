//! Syntax Highlighting
//!
//! Provides syntax highlighting for code blocks using syntect.

use once_cell::sync::Lazy;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

/// Global syntax set (loaded once at startup)
static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(SyntaxSet::load_defaults_newlines);

/// Global theme set (loaded once at startup)
static THEME_SET: Lazy<ThemeSet> = Lazy::new(ThemeSet::load_defaults);

/// Get the default theme (base16-ocean.dark)
fn get_theme() -> &'static Theme {
    &THEME_SET.themes["base16-ocean.dark"]
}

/// Convert syntect color to ratatui color
fn syntect_to_ratatui_color(color: syntect::highlighting::Color) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

/// Convert syntect font style to ratatui style
fn syntect_style_to_ratatui(syntect_style: syntect::highlighting::Style) -> Style {
    let mut style = Style::default().fg(syntect_to_ratatui_color(syntect_style.foreground));

    if syntect_style.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if syntect_style.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    if syntect_style.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
    }

    style
}

/// Find syntax definition by language name
fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    // Try exact match first
    if let Some(syntax) = SYNTAX_SET.find_syntax_by_token(language) {
        return Some(syntax);
    }

    // Try case-insensitive match
    let language_lower = language.to_lowercase();
    SYNTAX_SET.syntaxes().iter().find(|s| {
        s.name.to_lowercase() == language_lower
            || s.file_extensions
                .iter()
                .any(|ext| ext.to_lowercase() == language_lower)
    })
}

/// Highlight code with syntax highlighting
///
/// Returns a vector of styled lines for rendering in Ratatui.
pub fn highlight_code(code: &str, language: &str) -> Vec<Line<'static>> {
    // Find appropriate syntax
    let syntax = match find_syntax(language) {
        Some(s) => s,
        None => {
            // Fallback: return plain text with basic styling
            return code
                .lines()
                .enumerate()
                .map(|(idx, line)| {
                    Line::from(vec![
                        Span::styled(
                            format!("{:3} ", idx + 1),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(format!("│ {}", line), Style::default().fg(Color::Gray)),
                    ])
                })
                .collect();
        }
    };

    let theme = get_theme();
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();

    for (line_num, line) in LinesWithEndings::from(code).enumerate() {
        let ranges = match highlighter.highlight_line(line, &SYNTAX_SET) {
            Ok(ranges) => ranges,
            Err(_) => {
                // On error, return plain styled line
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:3} ", line_num + 1),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("│ {}", line.trim_end()),
                        Style::default().fg(Color::Gray),
                    ),
                ]));
                continue;
            }
        };

        let mut styled_line = vec![Span::styled(
            format!("{:3} ", line_num + 1),
            Style::default().fg(Color::DarkGray),
        )];

        styled_line.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));

        for (style, text) in ranges {
            styled_line.push(Span::styled(
                text.to_string(),
                syntect_style_to_ratatui(style),
            ));
        }

        lines.push(Line::from(styled_line));
    }

    lines
}

/// Get a list of all supported languages
pub fn supported_languages() -> Vec<String> {
    SYNTAX_SET
        .syntaxes()
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

/// Check if a language is supported
pub fn is_language_supported(language: &str) -> bool {
    find_syntax(language).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_rust() {
        let code = "fn main() {\n    println!(\"Hello, world!\");\n}";
        let lines = highlight_code(code, "rust");
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].spans.is_empty());
    }

    #[test]
    fn test_highlight_python() {
        let code = "def hello():\n    print(\"Hello, world!\")";
        let lines = highlight_code(code, "python");
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_highlight_javascript() {
        let code = "function hello() {\n  console.log(\"Hello\");\n}";
        let lines = highlight_code(code, "javascript");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_highlight_unknown_language() {
        let code = "some code";
        let lines = highlight_code(code, "unknown_language");
        assert_eq!(lines.len(), 1);
        // Should still render, just without syntax highlighting
    }

    #[test]
    fn test_supported_languages() {
        let langs = supported_languages();
        assert!(!langs.is_empty());
        assert!(langs.contains(&"Rust".to_string()));
        assert!(langs.contains(&"Python".to_string()));
    }

    #[test]
    fn test_is_language_supported() {
        assert!(is_language_supported("rust"));
        assert!(is_language_supported("Rust"));
        assert!(is_language_supported("python"));
        assert!(is_language_supported("javascript"));
        assert!(!is_language_supported("not_a_real_language"));
    }

    #[test]
    fn test_empty_code() {
        let code = "";
        let lines = highlight_code(code, "rust");
        // Should handle empty code gracefully
        assert!(lines.is_empty() || lines.len() == 1);
    }

    #[test]
    fn test_code_with_special_characters() {
        let code = "let x = \"Hello, 世界!\";";
        let lines = highlight_code(code, "rust");
        assert_eq!(lines.len(), 1);
    }
}
