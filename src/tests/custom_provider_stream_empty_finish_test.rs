//! Regression test for fork issue #105: empty-string `finish_reason` chunks in
//! SSE streams must be treated as continuation markers, not terminal reasons.
//!
//! Routes like `cbcn/glm-5.3-flash`, `cb/gpt-5.6-*` and `cb/kimi-k3` emit
//! `"finish_reason": ""` between content chunks. Before the fix, `Some("")`
//! passed the `is_some()` checks and (a) flushed PARTIAL tool calls mid-stream
//! and (b) set a premature terminal stop via MessageDelta. The `[DONE]`
//! sentinel and the usage-only chunk path remain the terminal authorities.

use crate::brain::provider::OpenAIProvider;
use crate::brain::provider::Provider;
use crate::brain::provider::{ContentBlock, ContentDelta, LLMRequest, Message, StreamEvent};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

/// Chunk with a role delta — opens the message.
fn chunk_role(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"chat.completion.chunk","model":"m","choices":[{{"index":0,"delta":{{"role":"assistant"}},"finish_reason":null}}]}}"#
    )
}

/// POISONED chunk: text content + EMPTY finish_reason (the #105 signature).
fn chunk_text_empty_finish(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"chat.completion.chunk","model":"m","choices":[{{"index":0,"delta":{{"content":"Hi "}},"finish_reason":""}}]}}"#
    )
}

/// POISONED chunk: PARTIAL tool-call args + EMPTY finish_reason.
/// Pre-fix this flushed the partial call as a ToolUse block.
fn chunk_tool_partial_empty_finish(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"chat.completion.chunk","model":"m","choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"id":"call_1","type":"function","function":{{"name":"get_weather","arguments":"{{\"city\":"}}}}]}},"finish_reason":""}}]}}"#
    )
}

/// Clean continuation: remaining args, no finish_reason.
fn chunk_tool_rest(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"chat.completion.chunk","model":"m","choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":"\"Paris\"}}"}}}}]}},"finish_reason":null}}]}}"#
    )
}

/// Terminal chunk: real finish_reason, no delta.
fn chunk_terminal(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"chat.completion.chunk","model":"m","choices":[{{"index":0,"delta":{{}},"finish_reason":"tool_calls"}}]}}"#
    )
}

/// Serve one HTTP request with the given SSE body, then close.
async fn serve_sse(listener: TcpListener, body: String) {
    let (mut sock, _) = listener.accept().await.expect("accept");
    let mut buf = [0u8; 8192];
    // Read the request head (we do not parse it — any POST is fine).
    let _ = timeout(Duration::from_secs(5), sock.read(&mut buf)).await;
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    sock.write_all(resp.as_bytes()).await.expect("write sse");
    sock.flush().await.ok();
}

async fn collect_events(provider: &OpenAIProvider, port: u16) -> Vec<StreamEvent> {
    let req = LLMRequest::new("test-model", vec![Message::user("weather?")]);
    let mut stream = provider.stream(req).await.expect("stream opens");
    let mut events = Vec::new();
    while let Some(ev) = futures::StreamExt::next(&mut stream).await {
        let ev = ev.expect("event ok");
        let done = matches!(ev, StreamEvent::MessageStop);
        events.push(ev);
        if done {
            break;
        }
    }
    events
}

/// The poisoned stream: empty finish_reason on text and partial-tool chunks,
/// real terminal reason only at the end, then [DONE].
#[tokio::test]
async fn empty_finish_reason_does_not_flush_partial_tool_calls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();

    let id = "chatcmpl-105";
    let sse = format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        chunk_role(id),
        chunk_text_empty_finish(id),
        chunk_tool_partial_empty_finish(id),
        chunk_tool_rest(id),
        chunk_terminal(id),
    );
    tokio::spawn(serve_sse(listener, sse));

    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"));
    let events = timeout(Duration::from_secs(10), collect_events(&provider, port))
        .await
        .expect("stream completes in time");

    // (1) Exactly ONE ToolUse block start — the COMPLETE call, flushed only at
    // the terminal chunk. Pre-fix, the empty-finish partial chunk emitted a
    // second (partial) ToolUse first.
    let tool_starts: Vec<&ContentBlock> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::ContentBlockStart { content_block, .. } => Some(content_block),
            _ => None,
        })
        .collect();
    assert_eq!(tool_starts.len(), 1, "tool starts: {tool_starts:?}");
    match &tool_starts[0] {
        ContentBlock::ToolUse { name, input, .. } => {
            assert_eq!(name, "get_weather");
            assert_eq!(input["city"], "Paris", "args must be complete: {input}");
        }
        other => panic!("expected ToolUse, got {other:?}"),
    }

    // (2) Exactly ONE MessageDelta with a stop reason — emitted only for the
    // real terminal chunk. Pre-fix, every empty-finish chunk emitted one.
    let deltas: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::MessageDelta { delta, .. } => Some(delta.stop_reason.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 1, "message deltas: {deltas:?}");
    assert!(deltas[0].is_some(), "terminal delta carries a stop reason");

    // (3) Text from the poisoned chunk still streamed through as content.
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::ContentBlockDelta { delta, .. } => match &delta {
                ContentDelta::TextDelta { text } => Some(text.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hi ");

    // (4) The stream terminates normally with MessageStop.
    assert!(matches!(events.last(), Some(StreamEvent::MessageStop)));
}
