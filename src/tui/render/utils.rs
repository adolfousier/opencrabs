//! Shared rendering utilities
//!
//! Text wrapping, character boundary helpers, and token formatting used across render modules.

use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

/// Pre-wrap a Line's text content to fit within max_width, preserving the style
/// of the first span and prepending `padding` to each continuation line.
pub(super) fn wrap_line_with_padding<'a>(
    line: Line<'a>,
    max_width: usize,
    padding: &'a str,
) -> Vec<Line<'a>> {
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
                let break_at = if break_at == 0 {
                    byte_limit.max(remaining.ceil_char_boundary(1))
                } else {
                    break_at
                };
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
pub(in crate::tui) fn char_boundary_at_width(s: &str, target_width: usize) -> usize {
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
pub(super) fn char_boundary_at_width_from_end(s: &str, target_width: usize) -> usize {
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

/// Format token count with a custom label (e.g. "1.2M total", "150K total")
pub(super) fn format_token_count_with_label(tokens: i32, label: &str) -> String {
    let tokens = tokens.max(0) as f64;
    if tokens >= 1_000_000.0 {
        format!("{:.1}M {}", tokens / 1_000_000.0, label)
    } else if tokens >= 1_000.0 {
        format!("{:.1}K {}", tokens / 1_000.0, label)
    } else if tokens > 0.0 {
        format!("{} {}", tokens as i32, label)
    } else {
        "new".to_string()
    }
}

/// Format token count as raw number without label (e.g. "150K", "1.2M")
pub(super) fn format_token_count_raw(tokens: i32) -> String {
    let tokens = tokens.max(0) as f64;
    if tokens >= 1_000_000.0 {
        format!("{:.1}M", tokens / 1_000_000.0)
    } else if tokens >= 1_000.0 {
        format!("{:.0}K", tokens / 1_000.0)
    } else if tokens > 0.0 {
        format!("{}", tokens as i32)
    } else {
        "0".to_string()
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
        assert!(
            result.len() > 1,
            "expected wrapping, got {} lines",
            result.len()
        );
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
}
