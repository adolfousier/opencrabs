//! Tests for the mermaid render-request retry and the transient-failure
//! headline split (#1741).
//!
//! The connect stage used to be one-shot while the body stage retried once
//! (#189), so a single stalled handshake degraded a valid diagram to the
//! failure block, whose headline read as a parse rejection. These tests pin
//! the new contract: exactly one retry for transient failures (transport
//! errors, 5xx, 408, 429), no retry for a deterministic 4xx, and the
//! failure blocks say who is at fault (renderer vs syntax).

use crate::channels::telegram::rich::ast::MermaidResult;
use crate::channels::telegram::rich::mermaid::{
    failure_html, failure_html_transport, is_image_response, is_transient_status,
    markdown_failure_block, markdown_failure_block_transport, markdown_failure_block_with_link,
    replacement_for, send_with_retry,
};

const PNG_1PX: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

const SRC: &str = "flowchart TD\n    A --> B";

// is_transient_status: the transient split must mirror classify_render_failure
// (4xx is deterministic except 408/429).
#[test]
fn transient_status_split_mirrors_classify_convention() {
    assert!(is_transient_status(500));
    assert!(is_transient_status(503));
    assert!(is_transient_status(599));
    assert!(is_transient_status(408));
    assert!(is_transient_status(429));
    assert!(!is_transient_status(200));
    assert!(!is_transient_status(301));
    assert!(!is_transient_status(400));
    assert!(!is_transient_status(404));
    assert!(!is_transient_status(499));
}

// transport_note wording is pinned indirectly by the dead-port test below:
// the note lands verbatim in the failure block, so renaming it silently
// would churn every downstream consumer.

#[tokio::test]
async fn transient_5xx_retries_once_then_succeeds() {
    let mut server = mockito::Server::new_async().await;
    let failed = server
        .mock("GET", "/render")
        .with_status(500)
        .with_header("content-type", "text/plain")
        .with_body("kaboom")
        .expect(1)
        .create_async()
        .await;
    let ok = server
        .mock("GET", "/render")
        .with_status(200)
        .with_header("content-type", "image/png")
        .with_body(PNG_1PX)
        .expect(1)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/render", server.url());
    let resp = send_with_retry(&client, &url, SRC, "connect")
        .await
        .expect("retry must recover a transient 5xx");

    assert!(
        is_image_response(resp.status().as_u16(), "image/png"),
        "the retried request must return the rendered image"
    );
    failed.assert_async().await;
    ok.assert_async().await;
}

#[tokio::test]
async fn retry_budget_is_exactly_one() {
    let mut server = mockito::Server::new_async().await;
    // Exactly TWO hits total: the initial send plus one retry, then the
    // helper gives up (the caller classifies the still-failed response).
    let mock = server
        .mock("GET", "/render")
        .with_status(503)
        .with_header("content-type", "text/plain")
        .with_body("still down")
        .expect(2)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/render", server.url());
    let resp = send_with_retry(&client, &url, SRC, "connect")
        .await
        .expect("the exhausted-budget response is returned for classification");

    assert_eq!(resp.status().as_u16(), 503);
    mock.assert_async().await;
}

#[tokio::test]
async fn deterministic_4xx_is_never_retried() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/render")
        .with_status(400)
        .with_header("content-type", "text/plain")
        .with_body("Parse error on line 2")
        .expect(1)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/render", server.url());
    let resp = send_with_retry(&client, &url, SRC, "connect")
        .await
        .expect("a deterministic rejection is returned untouched");

    assert_eq!(resp.status().as_u16(), 400);
    mock.assert_async().await;
}

#[tokio::test]
async fn transport_failure_on_dead_port_yields_failed_with_transport_note() {
    // The `.invalid` TLD is reserved by RFC 2606 and never resolves, so both
    // the initial send and the retry hit a genuine transport error (DNS
    // failure, not a timeout) and the helper degrades to Failed. A dropped
    // mockito server is NOT reliable here: the freed port can be re-bound by
    // another local listener before the request lands.
    let url = "http://opencrabs-render-probe.invalid/render";

    let client = reqwest::Client::new();
    let outcome = send_with_retry(&client, url, SRC, "connect-clamp").await;

    match outcome {
        Err(MermaidResult::Failed(note)) => {
            // DNS failure is not a timeout, so the helper must pick the
            // unreachable wording.
            assert_eq!(note, "diagram renderer unreachable");
        }
        other => panic!("expected Failed with transport note, got {other:?}"),
    }
}

// Headline split: transient failures must not read as a syntax rejection.
#[test]
fn transient_headline_says_renderer_not_syntax() {
    let md = markdown_failure_block_transport("diagram renderer timed out", SRC);
    assert!(
        md.contains("Renderer failure, not a syntax error: your diagram was NOT modified"),
        "transport headline must tell the reader the diagram was not the problem, got: {md}"
    );
    assert!(
        md.contains("diagram renderer timed out") && md.contains(SRC),
        "note and source stay visible under the new headline"
    );
    assert!(
        !md.contains("could not be rendered"),
        "the parse-rejection headline must not leak into the transient block"
    );
}

#[test]
fn parse_error_headline_is_unchanged() {
    let md = markdown_failure_block("Parse error on line 2", SRC);
    assert!(md.contains("Mermaid diagram could not be rendered"));
    assert!(
        !md.contains("NOT modified"),
        "deterministic rejection keeps its own headline"
    );
}

#[test]
fn failed_outcome_gets_transport_headline_and_svg_hatch() {
    let (md, media) = replacement_for(
        &MermaidResult::Failed("diagram renderer timed out".into()),
        0,
        SRC,
    );
    assert!(md.contains("your diagram was NOT modified"));
    assert!(
        md.contains("[svg]("),
        "transient keeps the svg escape hatch"
    );
    assert!(media.is_none());
}

#[test]
fn markdown_failure_with_link_composes_transport_block_and_svg_link() {
    let out = markdown_failure_block_with_link("diagram renderer timed out", SRC);
    assert!(
        out.contains("Renderer failure, not a syntax error: your diagram was NOT modified"),
        "the with-link variant inherits the transport headline: {out}"
    );
    assert!(
        out.contains("```") && out.contains("diagram renderer timed out") && out.contains(SRC),
        "error and source stay fenced under the headline: {out}"
    );
    assert!(
        out.contains("[svg](https://mermaid.ink/svg/"),
        "the escape hatch link points at the svg endpoint: {out}"
    );
}

#[test]
fn parse_error_outcome_gets_old_headline_without_hatch() {
    let (md, media) = replacement_for(
        &MermaidResult::ParseError("Parse error on line 2".into()),
        0,
        SRC,
    );
    assert!(md.contains("could not be rendered"));
    assert!(
        !md.contains("[svg]("),
        "dead link never offered for parse errors"
    );
    assert!(media.is_none());
}

#[test]
fn html_transient_headline_matches_markdown_split() {
    let html = failure_html_transport("diagram renderer timed out", SRC);
    assert!(html.contains("Renderer failure, not a syntax error: your diagram was NOT modified"));
    let plain = failure_html("Parse error on line 2", SRC);
    assert!(plain.contains("could not be rendered"));
    assert!(!plain.contains("NOT modified"));
}
