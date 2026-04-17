//! Tests for the Qwen-style text-based tool-call extractor.
//!
//! Local GGUF/MLX backends (llama.cpp, MLX, LM Studio, Ollama) serving Qwen3
//! often emit tool calls as `<tool_call>{json}</tool_call>` or
//! `<function=name>...<parameter=key>val</parameter>...</function>` inside
//! `message.content` instead of the structured `tool_calls` field. The
//! extractor recovers them so they execute; these tests lock in the
//! contract — especially the edge cases (nested braces, open-ended tags,
//! prose mentions, field aliases).

use crate::brain::provider::custom_openai_compatible::{
    extract_balanced_json, extract_text_tool_calls,
};

#[test]
fn balanced_json_simple() {
    assert_eq!(extract_balanced_json(r#"{"a":1}"#), Some(7));
    assert_eq!(extract_balanced_json(r#"{"a":{"b":2}} trailing"#), Some(13));
}

#[test]
fn balanced_json_strings_with_braces() {
    // Braces inside strings must not affect depth.
    let s = r#"{"cmd":"echo { nested } end"} trailing"#;
    let consumed = extract_balanced_json(s).expect("balanced");
    assert_eq!(&s[..consumed], r#"{"cmd":"echo { nested } end"}"#);
}

#[test]
fn balanced_json_escaped_quotes() {
    let s = r#"{"msg":"he said \"hi\" then left"}"#;
    let consumed = extract_balanced_json(s).expect("balanced");
    assert_eq!(consumed, s.len());
}

#[test]
fn balanced_json_unbalanced_returns_none() {
    assert_eq!(extract_balanced_json(r#"{"a":1"#), None);
    assert_eq!(extract_balanced_json("not json"), None);
}

#[test]
fn extract_tool_call_closed() {
    let text = r#"sure, running it. <tool_call>{"name":"bash","arguments":{"command":"ls"}}</tool_call> done."#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "bash");
    assert_eq!(calls[0].1["command"], "ls");
    assert!(!cleaned.contains("<tool_call>"));
    assert!(cleaned.contains("sure, running it"));
    assert!(cleaned.contains("done"));
}

#[test]
fn extract_tool_call_open_ended() {
    // Qwen frequently omits the closing tag.
    let text = r#"<tool_call>{"name":"web_search","arguments":{"query":"rust traits"}}"#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "web_search");
    assert_eq!(calls[0].1["query"], "rust traits");
    assert!(cleaned.trim().is_empty());
}

#[test]
fn extract_tool_call_nested_braces() {
    // Arguments with nested JSON must survive balanced-brace extraction.
    let text = r#"<tool_call>{"name":"set","arguments":{"obj":{"k":"v"},"n":1}}</tool_call>"#;
    let (calls, _) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "set");
    assert_eq!(calls[0].1["obj"]["k"], "v");
    assert_eq!(calls[0].1["n"], 1);
}

#[test]
fn extract_multiple_tool_calls() {
    let text = concat!(
        "first <tool_call>{\"name\":\"a\",\"arguments\":{}}</tool_call> ",
        "then <tool_call>{\"name\":\"b\",\"arguments\":{\"x\":2}}</tool_call>"
    );
    let (calls, _) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "a");
    assert_eq!(calls[1].0, "b");
    assert_eq!(calls[1].1["x"], 2);
}

#[test]
fn extract_tool_call_with_field_aliases() {
    // MiniMax-style `tool_name` + `args`.
    let text = r#"<tool_call>{"tool_name":"bash","args":{"command":"pwd"}}</tool_call>"#;
    let (calls, _) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "bash");
    assert_eq!(calls[0].1["command"], "pwd");
}

#[test]
fn extract_tool_call_stringified_arguments() {
    // Some runtimes emit arguments as a JSON-encoded string.
    let text = r#"<tool_call>{"name":"run","arguments":"{\"cmd\":\"go\"}"}</tool_call>"#;
    let (calls, _) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "run");
    assert_eq!(calls[0].1["cmd"], "go");
}

#[test]
fn extract_function_format() {
    let text = r#"<function=web_search><parameter=query>rust</parameter></function>"#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "web_search");
    assert_eq!(calls[0].1["query"], "rust");
    assert!(cleaned.trim().is_empty());
}

#[test]
fn skips_prose_mention_with_invalid_json() {
    // "the <tool_call> tag is special" — no JSON, so we must not strip it
    // and must not emit a tool call.
    let text = "the <tool_call> tag is special";
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 0);
    assert_eq!(cleaned, text);
}

#[test]
fn noop_without_markers() {
    let text = "just prose, no tool tags here";
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 0);
    assert_eq!(cleaned, text);
}

#[test]
fn ignores_tool_call_without_name() {
    // Malformed — no `name`/`tool_name` field → must not emit a call.
    let text = r#"<tool_call>{"arguments":{"x":1}}</tool_call>"#;
    let (calls, _) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 0);
}
