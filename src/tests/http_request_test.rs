//! Tests for `HttpClientTool` — the `http_request` tool.
//!
//! These tests exist to pin the User-Agent default behaviour after
//! the `ffa1329` fix. Every http_request failure on the 2026-04-17
//! logs was GitHub returning `403 Forbidden` with "Request forbidden
//! by administrative rules. Please make sure your request has a
//! User-Agent header." reqwest ships with no default UA, and the
//! model had no way to know GitHub mandates one — the tool now sets
//! `opencrabs/<CARGO_PKG_VERSION>` automatically on every request.
//!
//! We use mockito to stand up a local HTTP server and assert the
//! UA header is present on the request reqwest actually sent.

use crate::brain::tools::http::HttpClientTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use serde_json::json;
use uuid::Uuid;

fn ctx() -> ToolExecutionContext {
    ToolExecutionContext::new(Uuid::new_v4()).with_auto_approve(true)
}

#[tokio::test]
async fn default_user_agent_is_opencrabs_with_version() {
    let mut server = mockito::Server::new_async().await;
    let url = server.url();
    let expected_ua = concat!("opencrabs/", env!("CARGO_PKG_VERSION"));

    let _mock = server
        .mock("GET", "/anything")
        .match_header("user-agent", expected_ua)
        .with_status(200)
        .with_body("ok")
        .create_async()
        .await;

    let tool = HttpClientTool;
    let input = json!({
        "method": "GET",
        "url": format!("{}/anything", url),
    });
    let result = tool.execute(input, &ctx()).await.expect("tool execute");
    assert!(
        result.success,
        "request should succeed when UA matches expected default, got: {:?}",
        result.output
    );
}

#[tokio::test]
async fn caller_supplied_user_agent_overrides_default() {
    let mut server = mockito::Server::new_async().await;
    let url = server.url();

    // mockito matches headers case-insensitively, so "User-Agent" vs
    // "user-agent" is not an issue here. We just need the VALUE to
    // be the caller's custom string, NOT the default opencrabs/X.Y.Z.
    let _mock = server
        .mock("GET", "/anything")
        .match_header("user-agent", "my-custom-agent/1.0")
        .with_status(200)
        .with_body("ok")
        .create_async()
        .await;

    let tool = HttpClientTool;
    let input = json!({
        "method": "GET",
        "url": format!("{}/anything", url),
        "headers": { "User-Agent": "my-custom-agent/1.0" },
    });
    let result = tool.execute(input, &ctx()).await.expect("tool execute");
    assert!(
        result.success,
        "caller-supplied UA should override default, got: {:?}",
        result.output
    );
}

#[tokio::test]
async fn forbidden_response_surfaces_body_to_caller() {
    // Regression: the original GitHub 403s included a helpful body
    // ("Please make sure your request has a User-Agent header"). The
    // tool needs to propagate that body to the LLM so when the fix
    // doesn't help (e.g. a future provider banning UA=opencrabs/*),
    // the model sees the actual reason instead of an opaque failure.
    let mut server = mockito::Server::new_async().await;
    let url = server.url();

    let _mock = server
        .mock("GET", "/forbidden")
        .with_status(403)
        .with_body("Request forbidden by administrative rules. Please make sure your request has a User-Agent header")
        .create_async()
        .await;

    let tool = HttpClientTool;
    let input = json!({
        "method": "GET",
        "url": format!("{}/forbidden", url),
    });
    let result = tool.execute(input, &ctx()).await.expect("tool execute");
    assert!(!result.success, "403 should produce a failed ToolResult");
    let body = result.error.as_deref().unwrap_or("");
    assert!(
        body.contains("403"),
        "error text should mention the status code: {}",
        body
    );
    assert!(
        body.contains("User-Agent header"),
        "error text should surface the server's explanation: {}",
        body
    );
}
