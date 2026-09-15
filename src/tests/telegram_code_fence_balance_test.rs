use crate::channels::telegram::rich::ast::Block;
use crate::channels::telegram::rich::parse::parse_markdown;
use crate::channels::telegram::rich::{balance_code_fences, normalize_tables};

#[test]
fn test_balance_code_fences_no_fences() {
    let input = "Hello world\n\nNo fences here.";
    assert_eq!(balance_code_fences(input), input);
    assert_eq!(normalize_tables(input), input);
}

#[test]
fn test_balance_code_fences_already_balanced() {
    let input = "```rust\nfn main() {}\n```\n\n## Next section";
    assert_eq!(balance_code_fences(input), input);
}

#[test]
fn test_balance_code_fences_odd_at_eof() {
    let input = "Some text\n```rust\nlet x = 42;";
    let expected = "Some text\n```rust\nlet x = 42;\n```";
    assert_eq!(balance_code_fences(input), expected);
}

#[test]
fn test_balance_code_fences_tilde_odd_at_eof() {
    let input = "Some text\n~~~python\nprint('hi')";
    let expected = "Some text\n~~~python\nprint('hi')\n~~~";
    assert_eq!(balance_code_fences(input), expected);
}

#[test]
fn test_balance_code_fences_bare_fence_before_heading() {
    let input = "## Section 1\n```\nsome raw text\n## Section 2\n\nMore text";
    let expected = "## Section 1\n```\nsome raw text\n```\n## Section 2\n\nMore text";
    assert_eq!(balance_code_fences(input), expected);
}

#[test]
fn test_balance_code_fences_bare_fence_before_table() {
    let input = "Here is info:\n```\n| Key | Value |\n| --- | --- |\n| a | b |";
    let expected = "Here is info:\n```\n```\n| Key | Value |\n| --- | --- |\n| a | b |";
    assert_eq!(balance_code_fences(input), expected);
}

#[test]
fn test_balance_code_fences_labeled_fence_keeps_comments() {
    // A python or rust block with comments like `# comment` or `## heading` should stay open.
    let input = "```python\n# This is a comment\n## Another comment\nx = 1\n```";
    assert_eq!(balance_code_fences(input), input);
}

#[test]
fn test_parse_markdown_recovers_runaway_bare_fence() {
    let input = "Intro\n```\n| Name | Status |\n| --- | --- |\n| task1 | done |\n\n## Section 2\n\n- item 1\n- item 2";
    let blocks = parse_markdown(input);
    // Should have paragraph, code block (empty), table, heading, list.
    assert!(blocks.iter().any(|b| matches!(b, Block::Table(_))));
    assert!(blocks.iter().any(|b| matches!(b, Block::Heading { .. })));
    assert!(blocks.iter().any(|b| matches!(b, Block::List(_))));
}

#[test]
fn test_parse_markdown_unclosed_fence_at_eof_becomes_code_block() {
    let input = "Intro\n```rust\nfn main() {";
    let blocks = parse_markdown(input);
    assert_eq!(blocks.len(), 2);
    assert!(matches!(blocks[0], Block::Paragraph(_)));
    assert!(matches!(
        blocks[1],
        Block::Code {
            lang: Some(_),
            ref text
        } if text == "fn main() {"
    ));
}
