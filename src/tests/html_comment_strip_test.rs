//! Tests for strip_html_comments — ensures all HTML comment markers
//! are removed from LLM output to prevent tool artifacts leaking into TUI/channels.

use crate::brain::agent::AgentService;

#[test]
fn strips_proper_comment() {
    let input = "Hello <!-- this is a comment --> world";
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, "Hello  world");
}

#[test]
fn strips_tools_v2_marker() {
    let input = r#"Done.<!-- tools-v2: [{"d":"bash: ls","s":true,"o":"file.txt"}] -->Next"#;
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, "Done.Next");
}

#[test]
fn strips_lens_marker() {
    let input = "Done. Both tasks now complete:\n<!-- lens -->\n• task-a completed";
    let result = AgentService::strip_html_comments(input);
    assert!(result.contains("Done. Both tasks now complete:"));
    assert!(result.contains("• task-a completed"));
    assert!(!result.contains("lens"));
}

#[test]
fn preserves_malformed_close_tag() {
    // Unclosed comments are NOT stripped — doing so would silently eat
    // trailing response text during mid-stream rendering.
    let input = "Some text<!-- /tools-v2>";
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, input);
}

#[test]
fn preserves_unclosed_comment() {
    // Same rationale: unclosed comment must not swallow content to end-of-string.
    let input = "Before <!-- unclosed comment that never ends";
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, input);
}

#[test]
fn strips_multiple_comments() {
    let input = "A <!-- one --> B <!-- two --> C";
    let result = AgentService::strip_html_comments(input);
    assert!(result.contains("A"));
    assert!(result.contains("B"));
    assert!(result.contains("C"));
    assert!(!result.contains("one"));
    assert!(!result.contains("two"));
}

#[test]
fn strips_multiline_comment() {
    let input = "Start\n<!-- tools-v2: [\n{\"d\":\"bash\",\"s\":true}\n] -->\nEnd";
    let result = AgentService::strip_html_comments(input);
    assert!(result.contains("Start"));
    assert!(result.contains("End"));
    assert!(!result.contains("tools-v2"));
}

#[test]
fn preserves_text_without_comments() {
    let input = "Plain text with no HTML comments at all";
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, input);
}

#[test]
fn empty_input() {
    let result = AgentService::strip_html_comments("");
    assert_eq!(result, "");
}

#[test]
fn comment_only() {
    let input = "<!-- everything is a comment -->";
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, "");
}

#[test]
fn collapses_excessive_blank_lines() {
    let input = "Before\n\n\n<!-- removed -->\n\n\nAfter";
    let result = AgentService::strip_html_comments(input);
    assert!(!result.contains("\n\n\n"), "Should collapse 3+ blank lines");
    assert!(result.contains("Before"));
    assert!(result.contains("After"));
}

#[test]
fn tools_v2_with_rustc_arrow_in_output() {
    // Regression: rustc error output contains ` --> src/foo.rs:10:5` span
    // markers. A naive non-greedy `<!--.*?-->` terminates at the first
    // inner `-->` and leaks the rest of the JSON as visible text (TUI
    // screenshot 2026-04-17 16:58 — cargo check output bled into chat).
    let input = r#"Compiling.<!-- tools-v2: [{"d":"bash: cargo check","s":false,"o":"error[E0119]\n --> src/foo.rs:10:5\n"}] -->Done."#;
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, "Compiling.Done.");
    assert!(!result.contains("E0119"));
    assert!(!result.contains("\"d\":"));
}

#[test]
fn tools_v2_with_jsx_arrow_in_output() {
    // JSX/HTML fragments also contain `-->` in escaped content.
    let input = r#"Wrote.<!-- tools-v2: [{"d":"Write foo.tsx","s":true,"o":"<div>\n<!-- heading --> Hi\n</div>"}] -->!"#;
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, "Wrote.!");
}

#[test]
fn tools_v2_multiple_tools_each_with_arrows() {
    let input = r#"A<!-- tools-v2: [{"d":"bash","s":false,"o":"err\n --> f.rs:1\n"},{"d":"read","s":true,"o":"<!-- x -->"}] -->B<!-- tools-v2: [{"d":"more","s":true,"o":"--> more"}] -->C"#;
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, "ABC");
}
