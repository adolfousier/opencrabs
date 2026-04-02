use crate::tui::render::reasoning_to_lines;

#[test]
fn single_line_produces_one_line() {
    let result = reasoning_to_lines("hello world", 200);
    assert_eq!(result.len(), 1);
}

#[test]
fn newlines_produce_separate_lines() {
    let result = reasoning_to_lines("line one\nline two\nline three", 200);
    assert_eq!(result.len(), 3);
}

#[test]
fn empty_string_produces_one_empty_line() {
    let result = reasoning_to_lines("", 200);
    assert_eq!(result.len(), 1);
}

#[test]
fn trailing_newline_produces_extra_line() {
    let result = reasoning_to_lines("hello\n", 200);
    assert_eq!(result.len(), 2);
}

#[test]
fn consecutive_newlines_produce_empty_lines() {
    let result = reasoning_to_lines("a\n\n\nb", 200);
    assert_eq!(result.len(), 4);
}

#[test]
fn preserves_literal_newlines_unlike_markdown() {
    let text = "First thought.\nSecond thought.";
    let result = reasoning_to_lines(text, 200);
    assert_eq!(result.len(), 2);
}

#[test]
fn long_lines_wrap_to_width() {
    // 50 chars wide, a line with 80 chars should wrap to 2 lines
    let long_line = "a".repeat(80);
    let result = reasoning_to_lines(&long_line, 50);
    assert_eq!(result.len(), 2);
}
