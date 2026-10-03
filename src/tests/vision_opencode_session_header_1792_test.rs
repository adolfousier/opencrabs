//! #1792 item 1: `analyze_image` must reach a gateway with the same client
//! identity the chat path sends.
//!
//! The reported failure: the vision chain rolling onto an OpenCode Go
//! candidate returned `400 {"type":"MissingSessionID","message":"Request is
//! missing x-opencode-session and cannot be routed efficiently."}` while chat
//! on that same provider worked. Cause: the chat path calls
//! [`crate::brain::provider::identity::headers_for`] and
//! `try_vision_candidate` hand-rolled `Content-Type` + `Authorization` only,
//! so vision was the anonymous path. OpenCode documents the contract at
//! <https://opencode.ai/docs/go/>: a stable session id per conversation in
//! `x-opencode-session`, and a `User-Agent` that names the client rather than
//! an HTTP library.
//!
//! These pin the header set the vision call builds. The wire-level proof that
//! the set is actually attached is [`vision_headers_reach_the_request`].

use crate::brain::provider::identity;
use crate::brain::tools::provider_vision::vision_headers;
use uuid::Uuid;

const OPENCODE: &str = "https://opencode.ai/zen/go/v1/chat/completions";
/// A gateway with no identity contract: nothing may be added for it.
const UNKNOWN: &str = "https://gateway.invalid.example/v1/chat/completions";

fn get<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// The header the endpoint 400s without is present, and carries the
/// conversation id we pass in.
#[test]
fn vision_opencode_session_header_matches_the_conversation_id() {
    let sid = Uuid::new_v4();
    let h = vision_headers(OPENCODE, "test-key", Some(sid));
    assert_eq!(
        get(&h, "X-Opencode-Session"),
        Some(sid.to_string().as_str()),
        "the session header must name this conversation"
    );
    let ua = get(&h, "User-Agent").unwrap_or_default();
    assert!(
        ua.starts_with("OpenCrabs/"),
        "the docs ask for a self-identifying User-Agent, got: {ua}"
    );
}

/// "Stable per conversation" is the whole point: the same conversation keeps
/// one value, different conversations get different ones.
#[test]
fn vision_opencode_session_header_is_stable_per_conversation() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let first = vision_headers(OPENCODE, "k", Some(a));
    let again = vision_headers(OPENCODE, "k", Some(a));
    assert_eq!(
        get(&first, "X-Opencode-Session"),
        get(&again, "X-Opencode-Session"),
        "one conversation must not present two ids"
    );
    assert_ne!(
        get(&first, "X-Opencode-Session"),
        get(
            &vision_headers(OPENCODE, "k", Some(b)),
            "X-Opencode-Session"
        ),
        "two conversations must not share an id"
    );
}

/// A call with no conversation (the pinned-candidate path) still identifies
/// itself, because the gateway rejects an unidentified request either way.
#[test]
fn vision_opencode_session_header_survives_a_missing_conversation() {
    let h = vision_headers(OPENCODE, "k", None);
    assert!(
        !get(&h, "X-Opencode-Session").unwrap_or_default().is_empty(),
        "no session must not mean no header"
    );
}

/// Parity, stated as an equality the day someone forks the two paths will
/// break: the identity half of the vision request IS the chat path's result.
#[test]
fn vision_opencode_session_header_is_the_chat_paths_identity() {
    let sid = Uuid::new_v4();
    for url in [OPENCODE, "https://api.z.ai/api/prediction/v1", UNKNOWN] {
        let vision = vision_headers(url, "k", Some(sid));
        let identity_part = &vision[2..];
        assert_eq!(
            identity_part,
            identity::headers_for(url, Some(sid)).as_slice(),
            "vision drifted from the chat path for {url}"
        );
    }
}

/// Auth is still sent, and still first, after moving every header through
/// one builder.
#[test]
fn vision_headers_keep_bearer_auth_on_every_host() {
    for url in [OPENCODE, UNKNOWN] {
        let h = vision_headers(url, "secret", Some(Uuid::new_v4()));
        assert_eq!(h[0].0, "Content-Type");
        assert_eq!(h[0].1, "application/json");
        assert_eq!(h[1].0, "Authorization");
        assert_eq!(h[1].1, "Bearer secret");
    }
}

/// A gateway that never asked gets exactly the two auth headers. `headers_for`
/// is opt-in per host on purpose (it documents gateway fingerprinting), so a
/// blanket header dump here would be its own bug.
#[test]
fn vision_headers_add_nothing_for_a_gateway_without_a_contract() {
    let h = vision_headers(UNKNOWN, "k", Some(Uuid::new_v4()));
    assert_eq!(h.len(), 2, "unexpected headers: {h:?}");
    assert!(get(&h, "X-Opencode-Session").is_none());
    assert!(get(&h, "User-Agent").is_none());
}

/// The seam is not decorative: a request built by `try_vision_candidate`
/// carries what `vision_headers` returns. Asserted against a real socket on a
/// host with no identity contract, so auth is what must arrive.
#[tokio::test]
async fn vision_headers_reach_the_request() {
    use crate::brain::tools::provider_vision::ProviderVisionTool;
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use serde_json::json;

    let mut server = mockito::Server::new_async().await;
    let url = format!("{}/v1/chat/completions", server.url());
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer wire-key")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_body(r#"{"choices":[{"message":{"content":"a red square, seen over the wire"}}]}"#)
        .create_async()
        .await;

    let img = std::env::temp_dir().join(format!("vision-wire-{}.png", std::process::id()));
    // A real 1x1 PNG so detect_mime_type sees an image, not text.
    std::fs::write(
        &img,
        [
            0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D',
            b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
        ],
    )
    .unwrap();

    let tool = ProviderVisionTool::with_candidates(vec![(
        "wire-key".to_string(),
        url.clone(),
        "vision-model".to_string(),
    )]);
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    let res = tool
        .execute(
            json!({ "image": img.to_string_lossy(), "question": "what is this" }),
            &ctx,
        )
        .await
        .expect("a wire success is a ToolResult, not a transport error");
    let _ = std::fs::remove_file(&img);
    mock.assert_async().await;

    assert!(res.success, "vision call failed: {:?}", res.error);
    assert_eq!(res.output.trim(), "a red square, seen over the wire");
}
