//! #1687: a healthy stream must not inherit the transport total wall clock.
//!
//! reqwest's `.timeout()` covers the response BODY read, so a client built with
//! it puts a hard ceiling on an SSE stream no matter how many chunks are still
//! arriving. That is the defect, and it is older than #1635: #1635 saw the same
//! symptom at 60s on compat providers and raised the number to 300s
//! (`provider_stream_total_timeout_test.rs`), which moved the wall instead of
//! removing it.
//!
//! The incident: GLM 5.3 flash on z.ai, with NO timeout key set under
//! `[providers.zai]` — `factory.rs` applies `timeout_secs` only when the key is
//! present and > 0, so the ceiling in play was the family's hardcoded default
//! (60s from #217 until #1635 raised it to 300s). Every over-long turn died
//! mid-body, layer 2 re-sent the byte-identical request up to
//! `MAX_STREAM_RETRIES` times into the same wall, and only then did the
//! fallback chain get asked. Same account and keys on other harnesses ran 2h+
//! with no errors, because nothing else puts a total ceiling on a stream.
//!
//! The three tests below are the contract: the stream path loses the ceiling,
//! the non-streaming path keeps it, and a control proves the ceiling is real
//! when it is supposed to be.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::brain::provider::OpenAIProvider;
use crate::brain::provider::{
    ContentBlock, ContentDelta, LLMRequest, Message, Provider, StreamEvent,
};
use crate::utils::retry::RetryConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

/// One SSE frame, chunked-encoded the way a real event stream arrives.
async fn write_frame(sock: &mut tokio::net::TcpStream, data: &str) {
    let payload = format!("data: {data}\n\n");
    let head = format!("{:X}\r\n", payload.len());
    sock.write_all(head.as_bytes()).await.expect("chunk head");
    sock.write_all(payload.as_bytes())
        .await
        .expect("chunk body");
    sock.write_all(b"\r\n").await.expect("chunk crlf");
    sock.flush().await.ok();
}

/// Serve one request as a chunked SSE body, `gap` apart, then terminate.
///
/// `Content-Length` is deliberately absent: a real provider streams, it does not
/// announce a size up front. With `Transfer-Encoding: chunked` the framing is
/// identical to what `api.z.ai` sends.
async fn serve_slow_sse(listener: TcpListener, frames: Vec<String>, gap: Duration) {
    let (mut sock, _) = listener.accept().await.expect("accept");
    let mut buf = [0u8; 8192];
    let _ = timeout(Duration::from_secs(5), sock.read(&mut buf)).await;
    let head = "HTTP/1.1 200 OK\r\n\
                Content-Type: text/event-stream\r\n\
                Transfer-Encoding: chunked\r\n\r\n";
    sock.write_all(head.as_bytes()).await.expect("head");
    sock.flush().await.ok();
    for frame in &frames {
        write_frame(&mut sock, frame).await;
        tokio::time::sleep(gap).await;
    }
    sock.write_all(b"0\r\n\r\n").await.expect("last chunk");
    sock.flush().await.ok();
}

/// Block on a non-streaming response, then send it whole.
async fn serve_delayed_json(listener: TcpListener, body: String, delay: Duration) {
    let (mut sock, _) = listener.accept().await.expect("accept");
    let mut buf = [0u8; 8192];
    let _ = timeout(Duration::from_secs(5), sock.read(&mut buf)).await;
    tokio::time::sleep(delay).await;
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{}",
        body.len(),
        body
    );
    sock.write_all(resp.as_bytes()).await.expect("body");
    sock.flush().await.ok();
}

/// Accept connections forever, read each request head, then never answer it.
///
/// Records, per connection, how long the client held it open before hanging up
/// — measured on the SERVER side, which is the only place that can see the
/// transport ceiling fire. Nothing here writes a byte of body, so an EOF can
/// only come from the client giving the request up.
async fn serve_hanging_json(listener: TcpListener, hangups: Arc<Mutex<Vec<Duration>>>) {
    loop {
        let (mut sock, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => return,
        };
        let hangups = hangups.clone();
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let mut buf = [0u8; 8192];
            let _ = timeout(Duration::from_secs(5), sock.read(&mut buf)).await;
            // Poll for EOF for up to 5s. A total-timeout abort shows as the peer
            // closing; `WouldBlock` means the client is still waiting.
            for _ in 0..200 {
                tokio::time::sleep(Duration::from_millis(25)).await;
                match sock.try_read(&mut buf) {
                    Ok(0) => {
                        hangups.lock().unwrap().push(started.elapsed());
                        return;
                    }
                    Ok(_) => continue,
                    Err(_) => return,
                }
            }
        });
    }
}

fn role_frame(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"chat.completion.chunk","model":"m","choices":[{{"index":0,"delta":{{"role":"assistant"}},"finish_reason":null}}]}}"#
    )
}

fn content_frame(id: &str, text: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"chat.completion.chunk","model":"m","choices":[{{"index":0,"delta":{{"content":"{text}"}},"finish_reason":null}}]}}"#
    )
}

fn terminal_frame(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"chat.completion.chunk","model":"m","choices":[{{"index":0,"delta":{{}},"finish_reason":"stop"}}]}}"#
    )
}

/// Drive `stream()` to completion, returning the assembled text or the error
/// that cut it. The error is a VALUE here, not a panic: the pre-fix abort has
/// to be readable in the assertion message.
async fn collect_stream_text(provider: &OpenAIProvider) -> Result<String, String> {
    let req = LLMRequest::new("test-model", vec![Message::user("hello?")]);
    let mut stream = provider.stream(req).await.map_err(|e| e.to_string())?;
    let mut text = String::new();
    while let Some(ev) = futures::StreamExt::next(&mut stream).await {
        let ev = ev.map_err(|e| e.to_string())?;
        match ev {
            StreamEvent::ContentBlockDelta {
                delta: ContentDelta::TextDelta { text: chunk },
                ..
            } => text.push_str(&chunk),
            StreamEvent::ContentBlockStart {
                content_block: ContentBlock::Text { text: initial },
                ..
            } => text.push_str(&initial),
            StreamEvent::MessageStop => break,
            _ => {}
        }
    }
    Ok(text)
}

/// The #1687 contract: 12 frames 120ms apart is ~1.4s of perfectly healthy
/// streaming, every gap far below any idle guard, against a 250ms total
/// request timeout. Pre-fix reqwest aborts the body at 250ms and the caller
/// sees a dropped stream that becomes a fallback-chain hop; post-fix all 12
/// fragments arrive.
#[tokio::test]
async fn a_healthy_stream_outliving_the_request_timeout_is_not_cut() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();

    let id = "chatcmpl-1687";
    let mut frames = vec![role_frame(id)];
    for i in 0..12 {
        frames.push(content_frame(id, &format!("frag{i} ")));
    }
    frames.push(terminal_frame(id));
    frames.push("[DONE]".to_string());
    tokio::spawn(serve_slow_sse(listener, frames, Duration::from_millis(120)));

    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"))
        .with_timeout(Duration::from_millis(250));

    let text = timeout(Duration::from_secs(15), collect_stream_text(&provider))
        .await
        .expect("stream settles within 15s")
        .unwrap_or_else(|err| panic!("stream was cut: {err}"));

    for i in 0..12 {
        assert!(
            text.contains(&format!("frag{i}")),
            "fragment {i} lost. assembled: {text:?}"
        );
    }
}

/// The ceiling stays where it belongs. A non-streaming call has no chunks to
/// keep it alive, so the total request timeout is the only thing that can bound
/// it, and it must still fire.
#[tokio::test]
async fn the_request_timeout_still_bounds_non_streaming_calls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    let body = r#"{"id":"cmpl-1687","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"late"},"finish_reason":"stop"}],"usage":{}}"#.to_string();
    tokio::spawn(serve_delayed_json(
        listener,
        body,
        Duration::from_millis(900),
    ));

    // Retries OFF, so the measurement is of the clock and not of the retry
    // budget: complete() defaults to 4 attempts over ~15s (1+2+4+8), each one
    // individually cut by the ceiling, and the first run of this test reported
    // 14.86s for a 150ms ceiling for exactly that reason.
    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"))
        .with_timeout(Duration::from_millis(150))
        .with_retry_config(RetryConfig {
            max_attempts: 0,
            ..Default::default()
        });

    let req = LLMRequest::new("test-model", vec![Message::user("hello?")]);
    let started = std::time::Instant::now();
    let err = provider
        .complete(req)
        .await
        .expect_err("a 900ms body must not survive a 150ms request timeout");
    let elapsed = started.elapsed();
    // reqwest renders a total-timeout abort as `error sending request`, with no
    // "timeout" substring anywhere, so the assertion is on WHEN it came back:
    // well inside the 900ms the body needs, i.e. the ceiling fired rather than
    // the response arriving late and failing to parse.
    assert!(
        elapsed < Duration::from_millis(600),
        "complete() took {elapsed:?} to fail; the 150ms ceiling never fired (error: {err})"
    );
}

/// Control for the test above: the same server shape answering inside the
/// ceiling must succeed, so a pass there cannot be a malformed-body accident.
#[tokio::test]
async fn a_non_streaming_call_inside_the_ceiling_still_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    let body = r#"{"id":"cmpl-1687","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"on time"},"finish_reason":"stop"}],"usage":{}}"#.to_string();
    tokio::spawn(serve_delayed_json(
        listener,
        body,
        Duration::from_millis(30),
    ));

    let provider = OpenAIProvider::local(format!("http://127.0.0.1:{port}/chat/completions"))
        .with_timeout(Duration::from_secs(5));

    let req = LLMRequest::new("test-model", vec![Message::user("hello?")]);
    let resp = provider
        .complete(req)
        .await
        .expect("a 30ms body fits inside a 5s ceiling");
    let assembled: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(assembled, "on time", "body parsed: {resp:?}");
}
