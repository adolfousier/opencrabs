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

#[test]
fn extract_bare_tool_call_openai_envelope() {
    // The format Qwen3 leaks when the template isn't in reasoning mode —
    // seen in logs 2026-04-17 02:41:12 after phantom retries failed.
    let text = r#"tool_call:{"id":"call_001","type":"function","function":{"name":"bash","arguments":{"command":"ls -la"}}}"#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1, "must recover bare tool_call: prefix");
    assert_eq!(calls[0].0, "bash");
    assert_eq!(calls[0].1["command"], "ls -la");
    assert!(
        cleaned.trim().is_empty(),
        "the whole envelope must be stripped, got: {cleaned:?}"
    );
}

#[test]
fn extract_bare_tool_call_with_preceding_text() {
    let text = r#"I'll check that. tool_call:{"id":"c1","type":"function","function":{"name":"bash","arguments":{"command":"pwd"}}} done."#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "bash");
    assert!(cleaned.contains("I'll check that"));
    assert!(cleaned.contains("done"));
    assert!(!cleaned.contains("tool_call:"));
}

#[test]
fn bare_marker_rejects_non_boundary_prefix() {
    // `set_tool_call:{...}` is not a tool-call emission — the prefix is
    // embedded in an identifier. Must not extract.
    let text = r#"set_tool_call:{"x":1} and more"#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 0);
    assert_eq!(cleaned, text);
}

#[test]
fn extract_tool_calls_array_envelope() {
    let text = r#"{"tool_calls":[{"id":"c1","type":"function","function":{"name":"bash","arguments":{"command":"ls"}}},{"id":"c2","type":"function","function":{"name":"web_search","arguments":{"query":"rust"}}}]}"#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 2, "both envelope entries must be recovered");
    assert_eq!(calls[0].0, "bash");
    assert_eq!(calls[0].1["command"], "ls");
    assert_eq!(calls[1].0, "web_search");
    assert_eq!(calls[1].1["query"], "rust");
    assert!(cleaned.trim().is_empty());
}

#[test]
fn openai_nested_function_name_without_wrapper() {
    // Bare OpenAI object (no `tool_call:` prefix, no array wrapper).
    // Currently we don't attempt to recover unprefixed bare JSON — users
    // would want an extractor that checks every `{` prefix, which is far
    // too aggressive for prose. This test locks in the "no false positive"
    // expectation.
    let text =
        r#"{"id":"c1","type":"function","function":{"name":"bash","arguments":{"command":"ls"}}}"#;
    let (calls, _) = extract_text_tool_calls(text);
    assert_eq!(
        calls.len(),
        0,
        "bare OpenAI JSON without a marker must NOT be auto-extracted"
    );
}

#[test]
fn extract_singular_tool_call_envelope() {
    let text = r#"{"tool_call":{"name":"bash","arguments":{"command":"ls -la"}}}"#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "bash");
    assert_eq!(calls[0].1["command"], "ls -la");
    assert!(cleaned.trim().is_empty());
}

#[test]
fn extract_singular_envelope_with_malformed_json_missing_colons() {
    // Seen in logs 2026-04-17 03:07 — Qwen dropped the colons after keys
    // in its hallucinated envelope. Strict serde_json refuses this; the
    // regex fallback must still recover name + primitive-valued args so
    // the tool actually executes.
    let text = r#"{"tool_call" {"name" "bash" "arguments" {"command" "git status"}}}"#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(
        calls.len(),
        1,
        "malformed singular envelope must be recovered"
    );
    assert_eq!(calls[0].0, "bash");
    assert_eq!(calls[0].1["command"], "git status");
    assert!(cleaned.trim().is_empty());
}

#[test]
fn singular_envelope_rejects_plural_match() {
    // Sanity check — `"tool_calls"` (plural) must not be caught by the
    // singular branch. The plural has its own handling.
    let text = r#"{"tool_calls":[{"id":"c1","type":"function","function":{"name":"bash","arguments":{"command":"ls"}}}]}"#;
    let (calls, _) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "bash");
    assert_eq!(calls[0].1["command"], "ls");
}

#[test]
fn tool_calls_inside_prose_is_ignored() {
    // Prose like `the "tool_calls" field carries tool calls` must not
    // match — the wrapping `{` is far away and belongs to something else.
    let text = r#"The field called "tool_calls" is an array. Another sentence."#;
    let (calls, cleaned) = extract_text_tool_calls(text);
    assert_eq!(calls.len(), 0);
    assert_eq!(cleaned, text);
}
