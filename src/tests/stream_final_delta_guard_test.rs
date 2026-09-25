//! Regression guards for #1738: a stream must produce exactly ONE final
//! `MessageDelta` + `MessageStop` pair. The `[DONE]` fallback had no
//! already-emitted guard, so a stream whose finish_reason chunk carried
//! inline usage (zai) was finalized twice 0.3 ms apart: the second pair
//! carried a defaulted `EndTurn` over the real stop reason and zero
//! usage, and the TUI rendered the same text block twice.
//!
//! Collector deliberately does NOT stop at the first `MessageStop` —
//! the loop-side consumer does that, which is exactly why the duplicate
//! stayed invisible to `helpers.rs`. Draining to stream end sees what
//! the TUI sees.

use crate::brain::provider::types::StopReason;
use crate::brain::provider::{LLMRequest, Message, OpenAIProvider, Provider, StreamEvent};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

async fn serve_once(listener: TcpListener, sse: String) {
    let (mut sock, _) = listener.accept().await.expect("accept");
    let mut buf = [0u8; 8192];
    let _ = timeout(Duration::from_secs(5), sock.read(&mut buf)).await;
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        sse.len(),
        sse
    );
    sock.write_all(resp.as_bytes()).await.expect("write body");
    sock.flush().await.ok();
}

/// Drain the stream to exhaustion and keep every event, so a duplicate
/// finalization is COUNTED instead of hidden by an early break.
async fn events_from_sse(sse: String) -> Vec<StreamEvent> {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_once(listener, sse));
    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"));
    let req = LLMRequest::new("some-gateway-model", vec![Message::user("hi")]);
    let mut stream = timeout(Duration::from_secs(10), provider.stream(req))
        .await
        .expect("stream opens in time")
        .expect("stream opens");
    let mut events = Vec::new();
    timeout(Duration::from_secs(10), async {
        while let Some(ev) = futures::StreamExt::next(&mut stream).await {
            events.push(ev.expect("event ok"));
        }
    })
    .await
    .expect("stream drains in time");
    events
}

fn count_finals(events: &[StreamEvent]) -> (usize, usize) {
    let deltas = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::MessageDelta { .. }))
        .count();
    let stops = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::MessageStop))
        .count();
    (deltas, stops)
}

/// The #1738 incident shape: finish_reason chunk carries inline usage
/// (zai), then [DONE] arrives. Exactly one final pair, and it must be
/// the REAL one: stop ToolUse, the provider's token counts.
#[tokio::test]
async fn done_after_inline_usage_emits_exactly_one_final_pair() {
    let chunk = r#"{"id":"c1","object":"chat.completion.chunk","model":"some-gateway-model","choices":[{"index":0,"delta":{"content":"partial answer"},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":100,"completion_tokens":34}}"#;
    let sse = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    let events = events_from_sse(sse).await;
    let (deltas, stops) = count_finals(&events);
    assert_eq!(deltas, 1, "exactly one MessageDelta, got {deltas}");
    assert_eq!(stops, 1, "exactly one MessageStop, got {stops}");
    for e in &events {
        if let StreamEvent::MessageDelta { delta, usage } = e {
            assert_eq!(delta.stop_reason, Some(StopReason::ToolUse));
            assert_eq!(usage.output_tokens, 34);
            assert_eq!(usage.input_tokens, 100);
        }
    }
}

/// The usage-only-chunk finalize (MiniMax, include_usage) followed by
/// [DONE]: same guard, second emitter covered. One pair, real usage.
#[tokio::test]
async fn done_after_usage_only_chunk_emits_exactly_one_final_pair() {
    let finish = r#"{"id":"c2","object":"chat.completion.chunk","model":"some-gateway-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
    let usage_chunk = r#"{"id":"c2","object":"chat.completion.chunk","model":"some-gateway-model","choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":34}}"#;
    let sse = format!("data: {finish}\n\ndata: {usage_chunk}\n\ndata: [DONE]\n\n");
    let events = events_from_sse(sse).await;
    let (deltas, stops) = count_finals(&events);
    assert_eq!(deltas, 1, "exactly one MessageDelta, got {deltas}");
    assert_eq!(stops, 1, "exactly one MessageStop, got {stops}");
    for e in &events {
        if let StreamEvent::MessageDelta { usage, .. } = e {
            assert_eq!(usage.output_tokens, 34);
        }
    }
}

/// #694 preserved: a stream that never delivered usage inline or on a
/// usage-only chunk must STILL be finalized by the [DONE] fallback,
/// with the EndTurn default — otherwise the tool loop retried finished
/// turns forever.
#[tokio::test]
async fn done_without_prior_usage_still_finalizes() {
    let finish = r#"{"id":"c3","object":"chat.completion.chunk","model":"some-gateway-model","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":"stop"}]}"#;
    let sse = format!("data: {finish}\n\ndata: [DONE]\n\n");
    let events = events_from_sse(sse).await;
    let (deltas, stops) = count_finals(&events);
    assert_eq!(deltas, 1);
    assert_eq!(stops, 1);
    for e in &events {
        if let StreamEvent::MessageDelta { delta, .. } = e {
            assert_eq!(delta.stop_reason, Some(StopReason::EndTurn));
        }
    }
}
