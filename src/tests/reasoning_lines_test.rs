use crate::tui::render::reasoning_to_lines;

#[test]
fn single_line_produces_one_line() {
    let result = reasoning_to_lines("hello world");
    assert_eq!(result.len(), 1);
}

#[test]
fn newlines_produce_separate_lines() {
    let result = reasoning_to_lines("line one\nline two\nline three");
    assert_eq!(result.len(), 3);
}

#[test]
fn empty_string_produces_one_empty_line() {
    let result = reasoning_to_lines("");
    assert_eq!(result.len(), 1);
}

#[test]
fn trailing_newline_produces_extra_line() {
    let result = reasoning_to_lines("hello\n");
    assert_eq!(result.len(), 2);
}

#[test]
fn consecutive_newlines_produce_empty_lines() {
    let result = reasoning_to_lines("a\n\n\nb");
    assert_eq!(result.len(), 4);
}

#[test]
fn preserves_literal_newlines_unlike_markdown() {
    // The whole point: a single \n must produce a separate line,
    // not be collapsed like parse_markdown would do.
    let text = "First thought.\nSecond thought.";
    let result = reasoning_to_lines(text);
    assert_eq!(result.len(), 2);
}
