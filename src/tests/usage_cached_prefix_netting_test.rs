//! Regression guards for #1636: the cached prefix must be counted once.
//!
//! `prompt_tokens` on an OpenAI-compatible API is the GROSS prompt, cached
//! prefix included. [`TokenUsage::input_tokens`] is documented non-cached and
//! [`PricingConfig::calculate_cost_with_cache`] bills `cache_read_tokens`
//! separately on top of it, so handing the gross value through charges every
//! cached token twice and counts it twice in the token total.
//!
//! Every assignment site is covered end to end, driving the real provider
//! against a local socket rather than asserting on a parser in isolation:
//! the two streaming shapes (usage inlined on the finish chunk, and the
//! usage-only final chunk), the non-streaming response, and the
//! `nonstream_compat` synthesizer.

use crate::brain::provider::OpenAIProvider;
use crate::brain::provider::Provider;
use crate::brain::provider::nonstream_compat::synthesize_stream_events;
use crate::brain::provider::types::*;
use crate::brain::provider::{LLMRequest, Message, StreamEvent};
use crate::usage::pricing::{PricingConfig, PricingEntry, ProviderBlock};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

// ── The real GLM-5.3-Flash bench run, from its per-call [STREAM_USAGE] receipts ──
const FLASH_GROSS_INPUT: u32 = 22_020_984;
const FLASH_OUTPUT: u32 = 107_370;
const FLASH_CACHE_READ: u32 = 21_705_152;
const FLASH_NET_INPUT: u32 = FLASH_GROSS_INPUT - FLASH_CACHE_READ; // 315,832

/// One call out of that run, as logged on 2026-09-19 at 02:27:24.
const CALL_GROSS_INPUT: u32 = 34_906;
const CALL_OUTPUT: u32 = 71;
const CALL_CACHE_READ: u32 = 34_432;
const CALL_NET_INPUT: u32 = CALL_GROSS_INPUT - CALL_CACHE_READ; // 474

/// The published `zai` card for `glm-5.3-flash`.
fn zai_card() -> PricingConfig {
    let mut providers = HashMap::new();
    providers.insert(
        "zai".to_string(),
        ProviderBlock {
            entries: vec![PricingEntry {
                prefix: "glm-5.3-flash".to_string(),
                input_per_m: 0.15,
                output_per_m: 0.50,
                cache_write_per_m: Some(0.0),
                cache_read_per_m: Some(0.03),
            }],
        },
    );
    PricingConfig { providers }
}

/// The netted decomposition must reproduce the corrected bill, and feeding the
/// SAME row's gross prompt must reproduce the inflated one that was published.
/// Both halves matter: the first is the contract, the second is the size of the
/// defect, and it is the pair that makes a silent regression impossible to miss.
#[test]
fn cached_prefix_billed_once_reproduces_the_real_flash_run() {
    let card = zai_card();

    let netted = card.calculate_cost_with_cache(
        "glm-5.3-flash",
        FLASH_NET_INPUT,
        FLASH_OUTPUT,
        0,
        FLASH_CACHE_READ,
    );
    assert!(
        (netted - 0.752_214_36).abs() < 1e-6,
        "netted cost drifted: {netted}"
    );

    let double_counted = card.calculate_cost_with_cache(
        "glm-5.3-flash",
        FLASH_GROSS_INPUT,
        FLASH_OUTPUT,
        0,
        FLASH_CACHE_READ,
    );
    assert!(
        (double_counted - 4.007_987_16).abs() < 1e-6,
        "as-billed cost drifted: {double_counted}"
    );

    // 5.33x on this row. The cached prefix is almost the whole bill, which is
    // why the defect was invisible on low-cache-hit providers.
    assert!(
        double_counted / netted > 5.0,
        "ratio {}",
        double_counted / netted
    );
}

/// `total()` is documented as the non-cached count. With the gross prompt it
/// carried the entire cached prefix, which is what poisoned the context meter.
#[test]
fn token_total_excludes_the_cached_prefix() {
    let usage = TokenUsage {
        input_tokens: CALL_NET_INPUT,
        output_tokens: CALL_OUTPUT,
        cache_read_tokens: CALL_CACHE_READ,
        ..Default::default()
    };
    assert_eq!(usage.total(), CALL_NET_INPUT + CALL_OUTPUT);
    assert_eq!(
        usage.billable_input(),
        CALL_NET_INPUT + CALL_CACHE_READ,
        "billable input is the gross prompt, reassembled from net + cache"
    );
}

// ── Streaming paths, driven against a local socket ──────────────────────────

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
    let req = LLMRequest::new("glm-5.3-flash", vec![Message::user("hi")]);
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

/// Drive the provider against `sse` and return the usage it reported.
async fn usage_from_sse(sse: String) -> TokenUsage {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_once(listener, "text/event-stream", sse));
    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"));
    timeout(Duration::from_secs(10), collect_usage(&provider))
        .await
        .expect("stream completes in time")
}

fn assert_netted(usage: &TokenUsage, label: &str) {
    assert_eq!(
        usage.input_tokens, CALL_NET_INPUT,
        "{label}: input_tokens must be the GROSS prompt minus the cached prefix, \
         not the gross prompt itself"
    );
    assert_eq!(
        usage.cache_read_tokens, CALL_CACHE_READ,
        "{label}: cache read"
    );
    assert_eq!(usage.output_tokens, CALL_OUTPUT, "{label}: output");
    assert_eq!(
        usage.billable_input(),
        CALL_GROSS_INPUT,
        "{label}: net + cache must reassemble the provider's gross prompt exactly"
    );
}

/// Shape 1: the finish chunk carries usage inline (zai, zhipu).
#[tokio::test]
async fn stream_inline_usage_nets_the_cached_prefix() {
    let chunk = format!(
        r#"{{"id":"c1","object":"chat.completion.chunk","model":"glm-5.3-flash","choices":[{{"index":0,"delta":{{"content":"ok"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":{CALL_GROSS_INPUT},"completion_tokens":{CALL_OUTPUT},"prompt_tokens_details":{{"cached_tokens":{CALL_CACHE_READ}}}}}}}"#
    );
    let sse = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    assert_netted(&usage_from_sse(sse).await, "inline");
}

/// Shape 2: a usage-only final chunk with empty `choices` (MiniMax, OpenAI with
/// `stream_options.include_usage`).
#[tokio::test]
async fn stream_usage_only_chunk_nets_the_cached_prefix() {
    let finish = r#"{"id":"c2","object":"chat.completion.chunk","model":"glm-5.3-flash","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
    let usage_chunk = format!(
        r#"{{"id":"c2","object":"chat.completion.chunk","model":"glm-5.3-flash","choices":[],"usage":{{"prompt_tokens":{CALL_GROSS_INPUT},"completion_tokens":{CALL_OUTPUT},"prompt_tokens_details":{{"cached_tokens":{CALL_CACHE_READ}}}}}}}"#
    );
    let sse = format!("data: {finish}\n\ndata: {usage_chunk}\n\ndata: [DONE]\n\n");
    assert_netted(&usage_from_sse(sse).await, "usage-only");
}

/// The Anthropic-shaped field name, which OpenRouter passes through, must net
/// identically — `effective_cache_read` reads either spelling.
#[tokio::test]
async fn anthropic_style_cache_read_field_also_nets() {
    let chunk = format!(
        r#"{{"id":"c3","object":"chat.completion.chunk","model":"glm-5.3-flash","choices":[{{"index":0,"delta":{{"content":"ok"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":{CALL_GROSS_INPUT},"completion_tokens":{CALL_OUTPUT},"cache_read_input_tokens":{CALL_CACHE_READ}}}}}"#
    );
    let sse = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    assert_netted(&usage_from_sse(sse).await, "anthropic-style field");
}

/// A provider reporting more cached tokens than prompt tokens must clamp to
/// zero, never wrap. `u32` subtraction panics in debug builds.
#[tokio::test]
async fn cache_read_larger_than_prompt_clamps_to_zero() {
    let chunk = r#"{"id":"c4","object":"chat.completion.chunk","model":"glm-5.3-flash","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":500}}}"#;
    let sse = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    let usage = usage_from_sse(sse).await;
    assert_eq!(usage.input_tokens, 0);
    assert_eq!(usage.cache_read_tokens, 500);
}

/// A provider that reports no cache split at all must pass the prompt through
/// untouched — the netting may not shrink an uncached row.
#[tokio::test]
async fn no_cache_split_leaves_the_prompt_untouched() {
    let chunk = r#"{"id":"c5","object":"chat.completion.chunk","model":"glm-5.3-flash","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1200,"completion_tokens":34}}"#;
    let sse = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    let usage = usage_from_sse(sse).await;
    assert_eq!(usage.input_tokens, 1200);
    assert_eq!(usage.cache_read_tokens, 0);
}

/// Shape 3: the non-streaming response body.
#[tokio::test]
async fn non_stream_response_nets_the_cached_prefix() {
    let body = format!(
        r#"{{"id":"n1","object":"chat.completion","created":0,"model":"glm-5.3-flash","choices":[{{"index":0,"finish_reason":"stop","message":{{"role":"assistant","content":"ok"}}}}],"usage":{{"prompt_tokens":{CALL_GROSS_INPUT},"completion_tokens":{CALL_OUTPUT},"prompt_tokens_details":{{"cached_tokens":{CALL_CACHE_READ}}}}}}}"#
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_once(listener, "application/json", body));

    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"));
    let req = LLMRequest::new("glm-5.3-flash", vec![Message::user("hi")]);
    let resp = timeout(Duration::from_secs(10), provider.complete(req))
        .await
        .expect("completes in time")
        .expect("response ok");
    assert_netted(&resp.usage, "non-stream");
}

/// Shape 4: the `nonstream_compat` synthesizer, for upstreams that answer a
/// stream request with a plain JSON blob.
#[test]
fn nonstream_compat_nets_the_cached_prefix() {
    let json = format!(
        r#"{{"id":"g1","object":"chat.completion","created":0,"model":"glm-5.3-flash","choices":[{{"index":0,"finish_reason":"stop","message":{{"role":"assistant","content":"ok"}}}}],"usage":{{"prompt_tokens":{CALL_GROSS_INPUT},"completion_tokens":{CALL_OUTPUT},"prompt_tokens_details":{{"cached_tokens":{CALL_CACHE_READ}}}}}}}"#
    );
    let events = synthesize_stream_events(&json).expect("synthesizes");
    let usage = events
        .iter()
        .find_map(|e| match e {
            Ok(StreamEvent::MessageDelta { usage, .. }) => Some(*usage),
            _ => None,
        })
        .expect("a MessageDelta carried usage");
    assert_netted(&usage, "nonstream_compat");
}

/// The `[DONE]` fallback USED to fire on every stream, after the real usage
/// delta, carrying a LOCAL GROSS estimate of the prompt. The consumer
/// reconciles usage across deltas with `max()`, so once `input_tokens` became
/// the netted value a gross estimate would put the whole cached prefix back
/// into the bill through a different door: #1636 tamed the duplicate to zero
/// tokens, #1738 removed the second delta outright — once the provider has
/// reported usage the stream is already finalized and `[DONE]` emits nothing
/// at all (the duplicate pair re-rendered the same text block twice in the
/// TUI).
///
/// The invariant, asserted over the raw event sequence rather than by
/// replaying the consumer's own reconciliation: exactly ONE usage delta
/// reaches the consumer, and it carries the provider's NETTED input, never
/// the gross prompt.
#[tokio::test]
async fn done_fires_nothing_after_reported_usage_and_input_stays_netted() {
    let chunk = format!(
        r#"{{"id":"c6","object":"chat.completion.chunk","model":"glm-5.3-flash","choices":[{{"index":0,"delta":{{"content":"ok"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":{CALL_GROSS_INPUT},"completion_tokens":{CALL_OUTPUT},"prompt_tokens_details":{{"cached_tokens":{CALL_CACHE_READ}}}}}}}"#
    );
    let sse = format!("data: {chunk}\n\ndata: [DONE]\n\n");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_once(listener, "text/event-stream", sse));
    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"));
    let req = LLMRequest::new("glm-5.3-flash", vec![Message::user("hi")]);
    let mut stream = provider.stream(req).await.expect("stream opens");
    let mut deltas = Vec::new();
    // Drain to the END of the stream, past the first MessageStop — pre-#1738
    // the fallback delta was emitted after it.
    while let Some(ev) = futures::StreamExt::next(&mut stream).await {
        if let StreamEvent::MessageDelta { usage, .. } = ev.expect("event ok") {
            deltas.push(usage);
        }
    }

    assert_eq!(
        deltas.len(),
        1,
        "expected exactly the reported delta, got {} (a second delta here is the #1738 duplicate finalize)",
        deltas.len()
    );
    assert_eq!(
        deltas[0].input_tokens, CALL_NET_INPUT,
        "the reported delta must carry the netted input, not the gross prompt: {:?}",
        deltas[0]
    );
}
