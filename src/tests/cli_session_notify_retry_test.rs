//! CLI notify transport retry (#199 Leg B.3): a bounded retry on transport-class
//! failure only.
//!
//! The distinguishing property is what is NOT retried: a JSON-RPC `error` is a
//! DECISION — the gateway answered — so re-sending it would be pointless and, on
//! a `refused_in_flight`, actively wrong. Each test below fails loudly if the
//! wrong branch is taken, by arranging the SECOND scripted response to carry a
//! different outcome from the first: a retry that should not have happened
//! returns that second outcome and fails the assertion.

use crate::cli::session_notify::post_jsonrpc;
use std::time::{Duration, Instant};

/// The path the A2A gateway answers on. The `mock()` path and the dialled URL
/// MUST derive from this one string: `mockito::Server::url()` carries no path,
/// so a literal here and a different one on the wire leaves the request matching
/// NO mock, and mockito answers 501 — which `post_jsonrpc` correctly classifies
/// as bodiless (transport) and retries three times. The test then fails three
/// round-trips deep on a 501 detail instead of the scripted outcome (measured:
/// CI run 35549131537, both mockito tests in this file).
const GATEWAY_PATH: &str = "/a2a/v1";

/// The mocked gateway's endpoint — `Server::url()` plus the gateway path.
fn gateway_url(server: &mockito::ServerGuard) -> String {
    format!("{}{GATEWAY_PATH}", server.url())
}

/// Bind an ephemeral port and drop the listener, so the port is free to be
/// dialed but nothing is listening — the classic connection-refused case.
fn dead_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}{GATEWAY_PATH}")
}

fn rpc_result(outcome: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "outcome": outcome, "detail": "scripted" },
    })
    .to_string()
}

fn rpc_error() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "error": { "code": -32602, "message": "session not found" },
    })
    .to_string()
}

fn notify_body() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "session/notify",
        "params": { "session_id": "00000000-0000-0000-0000-000000000000", "message": "x" },
    })
}

#[tokio::test]
async fn a_bodiless_gateway_response_is_retried_and_the_notify_lands() {
    // Attempt 1: a gateway mid-reload answers 502 with no JSON-RPC body — the
    // exact transient class this leg exists for. Attempt 2: a real answer.
    let mut server = mockito::Server::new_async().await;
    let first = server
        .mock("POST", GATEWAY_PATH)
        .with_status(502)
        .with_body("upstream unavailable")
        .expect(1)
        .create_async()
        .await;
    let second = server
        .mock("POST", GATEWAY_PATH)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(rpc_result("delivered"))
        .expect(1)
        .create_async()
        .await;

    let result = post_jsonrpc(&gateway_url(&server), None, &notify_body())
        .await
        .expect("the retry must land the notify");

    assert_eq!(
        result.get("outcome").and_then(|v| v.as_str()),
        Some("delivered")
    );
    first.assert_async().await;
    second.assert_async().await;
}

#[tokio::test]
async fn a_jsonrpc_error_is_a_decision_and_is_not_retried() {
    // A parsed JSON-RPC error means the gateway ANSWERED. If the code wrongly
    // retried, the second scripted mock would answer with `deferred` and this
    // test would return Ok — so the Err assertion below is the discriminator,
    // not a formality.
    let mut server = mockito::Server::new_async().await;
    let decision = server
        .mock("POST", GATEWAY_PATH)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(rpc_error())
        .expect(1)
        .create_async()
        .await;
    let _would_only_serve_a_retry = server
        .mock("POST", GATEWAY_PATH)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(rpc_result("deferred"))
        .create_async()
        .await;

    let err = post_jsonrpc(&gateway_url(&server), None, &notify_body())
        .await
        .expect_err("a JSON-RPC error must surface, not be retried");

    assert!(err.contains("gateway error"), "unexpected detail: {err}");
    decision.assert_async().await;
}

#[tokio::test]
async fn an_unreachable_gateway_spends_the_whole_retry_budget() {
    // The transport branch: every attempt fails at `send`, so the loop must run
    // to exhaustion and hand the caller a transport-class detail. The elapsed
    // floor is the BACKOFF SUM — `tokio::time::sleep` never wakes early, so
    // this proves all three attempts were made without a flake-prone timing
    // guess.
    let started = Instant::now();
    let err = post_jsonrpc(&dead_endpoint(), None, &notify_body())
        .await
        .expect_err("nothing is listening on that port");
    let elapsed = started.elapsed();

    assert!(
        err.contains("cannot reach the A2A gateway"),
        "unexpected detail: {err}"
    );
    assert!(
        elapsed >= Duration::from_millis(1500),
        "3 attempts mean two backoffs (500 + 1000 ms); only {elapsed:?} elapsed"
    );
}

#[test]
fn the_retry_budget_is_bounded_and_the_backoff_table_matches_it() {
    // A guard on the file's own invariants: `post_jsonrpc` indexes
    // RETRY_BACKOFF_MS by `attempt - 1`, so a table shorter than
    // MAX_ATTEMPTS - 1 would panic at runtime on the last attempt, and a longer
    // one would mean a backoff that can never be used.
    assert_eq!(
        crate::cli::session_notify::RETRY_BACKOFF_MS.len(),
        crate::cli::session_notify::MAX_ATTEMPTS as usize - 1,
    );
}
