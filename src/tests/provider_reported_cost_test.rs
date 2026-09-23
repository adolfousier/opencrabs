//! Regression guards for #1707: where a provider reports what it charged,
//! the ledger must use that number, not the local pricing-table guess.
//!
//! OpenRouter (`usage.cost`), LiteLLM and similar gateways return the billed
//! dollars on the usage object. The local table's default cache multipliers
//! are Anthropic-shaped, and custom endpoints with off-table rates are exactly
//! where the guess goes wrong — the ~40x invoice drift that was reported.
//! Ingestion happens at every assignment site the #1636 netting guards cover:
//! the two streaming shapes, the non-streaming body, and the `nonstream_compat`
//! synthesizer. Preference over table math is `authoritative_cost`, which must
//! refuse a sum with any counted iteration missing.

use crate::brain::provider::OpenAIProvider;
use crate::brain::provider::nonstream_compat::synthesize_stream_events;
use crate::brain::provider::types::*;
use crate::brain::provider::{LLMRequest, Message, Provider, StreamEvent};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

const REPORTED_COST: f64 = 0.001234;

async fn serve_once(listener: TcpListener, content_type: &'static str, body: String) {
    let (mut sock, _) = listener.accept().await.expect("accept");
    let mut buf = [0u8; 8192];
    let _ = timeout(Duration::from_secs(5), sock.read(&mut buf)).await;
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        content_type,
        body.len(),
        body
    );
    sock.write_all(resp.as_bytes()).await.expect("write body");
    sock.flush().await.ok();
}

async fn collect_usage(provider: &OpenAIProvider) -> TokenUsage {
    let req = LLMRequest::new("some-gateway-model", vec![Message::user("hi")]);
    let mut stream = provider.stream(req).await.expect("stream opens");
    let mut usage = None;
    while let Some(ev) = futures::StreamExt::next(&mut stream).await {
        match ev.expect("event ok") {
            StreamEvent::MessageDelta { usage: u, .. } => usage = Some(u),
            StreamEvent::MessageStop => break,
            _ => {}
        }
    }
    usage.expect("a MessageDelta carried usage")
}

async fn usage_from_sse(sse: String) -> TokenUsage {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_once(listener, "text/event-stream", sse));
    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"));
    timeout(Duration::from_secs(10), collect_usage(&provider))
        .await
        .expect("stream completes in time")
}

/// Shape 1: usage inlined on the finish chunk (zai, OpenRouter streaming).
#[tokio::test]
async fn stream_inline_usage_carries_reported_cost() {
    let chunk = r#"{"id":"c1","object":"chat.completion.chunk","model":"some-gateway-model","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1200,"completion_tokens":34,"cost":0.001234}}"#;
    let sse = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    let usage = usage_from_sse(sse).await;
    assert_eq!(usage.cost_usd, Some(REPORTED_COST));
}

/// Shape 2: usage-only final chunk (MiniMax, include_usage).
#[tokio::test]
async fn stream_usage_only_chunk_carries_reported_cost() {
    let finish = r#"{"id":"c2","object":"chat.completion.chunk","model":"some-gateway-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
    let usage_chunk = r#"{"id":"c2","object":"chat.completion.chunk","model":"some-gateway-model","choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":34,"cost":0.001234}}"#;
    let sse = format!("data: {finish}\n\ndata: {usage_chunk}\n\ndata: [DONE]\n\n");
    let usage = usage_from_sse(sse).await;
    assert_eq!(usage.cost_usd, Some(REPORTED_COST));
}

/// Shape 3: the non-streaming response body.
#[tokio::test]
async fn non_stream_response_carries_reported_cost() {
    let body = r#"{"id":"n1","object":"chat.completion","created":0,"model":"some-gateway-model","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}],"usage":{"prompt_tokens":1200,"completion_tokens":34,"cost":0.001234}}"#;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_once(listener, "application/json", body.to_string()));
    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"));
    let req = LLMRequest::new("some-gateway-model", vec![Message::user("hi")]);
    let resp = timeout(Duration::from_secs(10), provider.complete(req))
        .await
        .expect("completes in time")
        .expect("response ok");
    assert_eq!(resp.usage.cost_usd, Some(REPORTED_COST));
}

/// Shape 4: the `nonstream_compat` synthesizer.
#[test]
fn nonstream_compat_carries_reported_cost() {
    let json = r#"{"id":"g1","object":"chat.completion","created":0,"model":"some-gateway-model","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}],"usage":{"prompt_tokens":1200,"completion_tokens":34,"cost":0.001234}}"#;
    let events = synthesize_stream_events(json).expect("synthesizes");
    let usage = events
        .iter()
        .find_map(|e| match e {
            Ok(StreamEvent::MessageDelta { usage, .. }) => Some(*usage),
            _ => None,
        })
        .expect("a MessageDelta carried usage");
    assert_eq!(usage.cost_usd, Some(REPORTED_COST));
}

/// A provider that reports no `cost` must leave the field None so the table
/// computation still bills the turn — ingestion may not manufacture zero-cost
/// rows for the silence of a gateway (#1717 is the scoped fix for the table's
/// own zero hole; this test guards that the Option is not collapsed to 0.0).
#[tokio::test]
async fn reported_cost_defaults_to_none_without_the_field() {
    let chunk = r#"{"id":"c5","object":"chat.completion.chunk","model":"some-gateway-model","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1200,"completion_tokens":34}}"#;
    let sse = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    let usage = usage_from_sse(sse).await;
    assert_eq!(usage.cost_usd, None);
}

/// A free model legitimately costing zero is a REPORT (Some(0.0)), and a
/// report wins — it is not the same event as a missing field.
#[test]
fn zero_is_a_report_not_a_missing_field() {
    let json = r#"{"input_tokens":10,"output_tokens":5,"cost_usd":0.0}"#;
    let usage: TokenUsage = serde_json::from_str(json).expect("parses");
    assert_eq!(usage.cost_usd, Some(0.0));
    assert_eq!(authoritative_cost(Some(0.0), 0, 0.25), 0.0);
}

/// The preference contract: the reported sum wins only when complete.
#[test]
fn authoritative_cost_prefers_the_report() {
    // Complete report beats the table, in both directions of error.
    assert_eq!(authoritative_cost(Some(0.12), 0, 4.80), 0.12);
    assert_eq!(authoritative_cost(Some(4.80), 0, 0.12), 4.80);
    // One counted iteration missing its report: the partial sum under-bills,
    // so the table must win.
    assert_eq!(authoritative_cost(Some(0.12), 1, 4.80), 4.80);
    // Nothing reported at all: table.
    assert_eq!(authoritative_cost(None, 3, 4.80), 4.80);
    // No iterations at all (degenerate): table, never a fabricated 0.0.
    assert_eq!(authoritative_cost(None, 0, 4.80), 4.80);
}

/// The tool-loop accumulator fold, reproduced against the real arithmetic the
/// loop runs: additions per iteration, subtraction on dropped-stream retries,
/// and the missing-counter that disqualifies a sum that lost an iteration.
/// Decimal literals with tolerance: 0.1+0.2+0.05 is 0.35000000000000003 in
/// f64, and the ledger cares about dollars, not ULPs.
#[test]
fn multi_iteration_sum_mirrors_subtraction() {
    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }
    let mut sum: Option<f64> = None;
    let mut missing = 0u32;
    for c in [0.10, 0.20, 0.05] {
        sum = Some(sum.unwrap_or(0.0) + c);
    }
    // Dropped-stream retry mirrors it out (same call re-counts on retry).
    let dropped = 0.20;
    sum = sum.map(|s| s - dropped);
    assert!(approx(authoritative_cost(sum, missing, 9.9), 0.15));
    // An iteration without a report disqualifies the sum.
    missing += 1;
    assert!(approx(authoritative_cost(sum, missing, 9.9), 9.9));
    // …and its subtraction on the retry re-qualifies it.
    missing = missing.saturating_sub(1);
    assert!(approx(authoritative_cost(sum, missing, 9.9), 0.15));
}
