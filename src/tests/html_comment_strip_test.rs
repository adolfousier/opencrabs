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
fn strips_malformed_close_tag() {
    // LLM hallucinates <!-- /tools-v2> without proper --> close
    let input = "Some text<!-- /tools-v2>";
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, "Some text");
}

#[test]
fn strips_unclosed_comment() {
    let input = "Before <!-- unclosed comment that never ends";
    let result = AgentService::strip_html_comments(input);
    assert_eq!(result, "Before");
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
