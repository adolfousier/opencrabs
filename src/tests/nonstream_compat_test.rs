use crate::brain::provider::nonstream_compat::{is_nonstream_response, synthesize_stream_events};
use crate::brain::provider::types::*;

#[test]
fn nonstream_text_only() {
    let json = r#"{"id":"gen-123","object":"chat.completion","created":0,"model":"test","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"Hello world"}}],"usage":{"prompt_tokens":10,"completion_tokens":2}}"#;
    assert!(is_nonstream_response(json));
    let events = synthesize_stream_events(json).unwrap();
    // MessageStart + ContentBlockStart + ContentBlockStop + MessageDelta + MessageStop
    assert_eq!(events.len(), 5);
    if let Ok(StreamEvent::ContentBlockStart {
        content_block: ContentBlock::Text { ref text },
        ..
    }) = events[1]
    {
        assert_eq!(text, "Hello world");
    } else {
        panic!("expected ContentBlockStart with text");
    }
    if let Ok(StreamEvent::MessageDelta { ref delta, .. }) = events[3] {
        assert_eq!(delta.stop_reason, Some(StopReason::EndTurn));
    } else {
        panic!("expected MessageDelta");
    }
}

#[test]
fn nonstream_with_tool_calls() {
    let json = r#"{"id":"gen-456","object":"chat.completion","created":0,"model":"test","choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","content":"Let me check.","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"bash","arguments":"{\"command\":\"ls\"}"}}]}}],"usage":{"prompt_tokens":100,"completion_tokens":20}}"#;
    let events = synthesize_stream_events(json).unwrap();
    // MessageStart + text Start/Stop + tool Start/Stop + MessageDelta + MessageStop
    assert_eq!(events.len(), 7);
    if let Ok(StreamEvent::ContentBlockStart {
        index,
        content_block: ContentBlock::ToolUse { ref name, .. },
    }) = events[3]
    {
        assert_eq!(index, 1);
        assert_eq!(name, "bash");
    } else {
        panic!("expected tool use ContentBlockStart");
    }
    if let Ok(StreamEvent::MessageDelta { ref delta, .. }) = events[5] {
        assert_eq!(delta.stop_reason, Some(StopReason::ToolUse));
    } else {
        panic!("expected MessageDelta with ToolUse stop");
    }
}

#[test]
fn nonstream_with_reasoning() {
    let json = r#"{"id":"gen-789","object":"chat.completion","created":0,"model":"test","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"\nI'm here.","reasoning":"The user is testing."}}],"usage":{"prompt_tokens":50,"completion_tokens":5}}"#;
    let events = synthesize_stream_events(json).unwrap();
    // MessageStart + ReasoningDelta + ContentBlockStart + ContentBlockStop + MessageDelta + MessageStop
    assert_eq!(events.len(), 6);
    if let Ok(StreamEvent::ContentBlockDelta {
        delta: ContentDelta::ReasoningDelta { ref text },
        ..
    }) = events[1]
    {
        assert_eq!(text, "The user is testing.");
    } else {
        panic!("expected ReasoningDelta, got {:?}", events[1]);
    }
    // Content should have leading newline stripped
    if let Ok(StreamEvent::ContentBlockStart {
        content_block: ContentBlock::Text { ref text },
        ..
    }) = events[2]
    {
        assert_eq!(text, "I'm here.");
    } else {
        panic!("expected content without leading newline");
    }
}

#[test]
fn not_nonstream() {
    assert!(!is_nonstream_response("data: {\"id\":\"123\"}"));
    assert!(!is_nonstream_response("not json at all"));
    assert!(!is_nonstream_response(
        "{\"object\":\"chat.completion.chunk\"}"
    ));
}

#[test]
fn nonstream_with_cache_usage() {
    let json = r#"{"id":"gen-cache","object":"chat.completion","created":0,"model":"test","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"cached"}}],"usage":{"prompt_tokens":1000,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":900}}}"#;
    let events = synthesize_stream_events(json).unwrap();
    if let Ok(StreamEvent::MessageDelta { ref usage, .. }) = events[events.len() - 2] {
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cache_read_tokens, 900);
    } else {
        panic!("expected MessageDelta with cache usage");
    }
}
