//! Tests for XML tool-call fallback extraction.
//!
//! Some providers (e.g. MiniMax) emit tool calls as `<tool_call>` XML in the
//! text content instead of the structured `tool_calls` field. The fallback
//! parser extracts these so the tool loop can execute them.

use crate::brain::agent::service::AgentService;

// --- extract_xml_tool_calls ---

#[test]
fn extracts_single_tool_call() {
    let text = r#"You're right. Let me check the cron status:

<tool_call>
<invoke name="cron_manage">
<parameter name="action">list</parameter>
</invoke>
</tool_call>"#;

    let result = AgentService::extract_xml_tool_calls(text).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "cron_manage");
    assert_eq!(result[0].1["action"], "list");
}

#[test]
fn extracts_multiple_tool_calls() {
    let text = r#"Let me do two things:

<tool_call>
<invoke name="read_file">
<parameter name="path">/tmp/test.txt</parameter>
</invoke>
</tool_call>

And also:

<tool_call>
<invoke name="bash">
<parameter name="command">ls -la /tmp</parameter>
</invoke>
</tool_call>"#;

    let result = AgentService::extract_xml_tool_calls(text).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0, "read_file");
    assert_eq!(result[0].1["path"], "/tmp/test.txt");
    assert_eq!(result[1].0, "bash");
    assert_eq!(result[1].1["command"], "ls -la /tmp");
}

#[test]
fn extracts_multi_parameter_tool_call() {
    let text = r#"<tool_call>
<invoke name="write_file">
<parameter name="path">/tmp/output.txt</parameter>
<parameter name="content">Hello, world!</parameter>
</invoke>
</tool_call>"#;

    let result = AgentService::extract_xml_tool_calls(text).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "write_file");
    assert_eq!(result[0].1["path"], "/tmp/output.txt");
    assert_eq!(result[0].1["content"], "Hello, world!");
}

#[test]
fn returns_none_for_no_xml() {
    let text = "Just a normal response with no tool calls.";
    assert!(AgentService::extract_xml_tool_calls(text).is_none());
}

#[test]
fn returns_none_for_malformed_xml() {
    let text = "<tool_call><invoke name=\"broken\">no closing tags";
    assert!(AgentService::extract_xml_tool_calls(text).is_none());
}

#[test]
fn handles_compact_xml_whitespace() {
    let text = r#"<tool_call><invoke name="bash"><parameter name="command">echo hi</parameter></invoke></tool_call>"#;

    let result = AgentService::extract_xml_tool_calls(text).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "bash");
    assert_eq!(result[0].1["command"], "echo hi");
}

// --- strip_xml_tool_calls ---

#[test]
fn strips_xml_preserves_surrounding_text() {
    let text = r#"You're right. Let me check:

<tool_call>
<invoke name="cron_manage">
<parameter name="action">list</parameter>
</invoke>
</tool_call>"#;

    let stripped = AgentService::strip_xml_tool_calls(text);
    assert_eq!(stripped, "You're right. Let me check:");
    assert!(!stripped.contains("<tool_call>"));
}

#[test]
fn strips_multiple_xml_blocks() {
    let text = r#"First:

<tool_call>
<invoke name="a">
<parameter name="x">1</parameter>
</invoke>
</tool_call>

Middle text.

<tool_call>
<invoke name="b">
<parameter name="y">2</parameter>
</invoke>
</tool_call>

End."#;

    let stripped = AgentService::strip_xml_tool_calls(text);
    assert!(!stripped.contains("<tool_call>"));
    assert!(stripped.contains("First:"));
    assert!(stripped.contains("Middle text."));
    assert!(stripped.contains("End."));
}

#[test]
fn strip_returns_empty_for_only_xml() {
    let text = r#"<tool_call>
<invoke name="bash">
<parameter name="command">ls</parameter>
</invoke>
</tool_call>"#;

    let stripped = AgentService::strip_xml_tool_calls(text);
    assert!(stripped.is_empty());
}

// --- Real-world MiniMax reproduction ---

#[test]
fn minimax_cron_manage_reproduction() {
    // Exact pattern from the 2026-03-14T00:09:08 log entry
    let text = "You're right. Let me check the cron status:\n\n\n\n\n\n<tool_call>\n<invoke name=\"cron_manage\">\n\n<parameter name=\"action\">list</parameter>\n\n</invoke>\n</tool_call>";

    let result = AgentService::extract_xml_tool_calls(text).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "cron_manage");
    assert_eq!(result[0].1["action"], "list");

    let stripped = AgentService::strip_xml_tool_calls(text);
    assert!(!stripped.contains("<tool_call>"));
    assert!(!stripped.contains("cron_manage"));
    assert!(stripped.contains("cron status"));
}
