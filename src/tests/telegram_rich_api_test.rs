//! Regression tests for the rich API client parameterised `api_url` (#1088).
//!
//! Verifies that the4 public functions in `crate::channels::telegram::rich::api`
//! route through a caller-supplied base URL instead of hardcoding
//! `api.telegram.org`. Uses `mockito` to intercept the HTTP call and confirm
//! the constructed endpoint is hit.

use crate::channels::telegram::rich::api;
use crate::channels::telegram::rich::ast::MermaidResult;
use crate::channels::telegram::rich::mermaid::cache_put;

#[tokio::test]
async fn send_rich_markdown_id_uses_custom_api_url() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"result":{"message_id":42}}"#)
        .create_async()
        .await;

    let result = api::send_rich_markdown_id(
        &server.url(),
        "TESTTOKEN",
        12345,
        None,
        "hello **world**",
        None,
        "test",
        "-",
    )
    .await;

    assert!(result.is_ok(), "send should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 42);
    mock.assert_async().await;
}

#[tokio::test]
async fn send_rich_html_id_uses_custom_api_url() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"result":{"message_id":99}}"#)
        .create_async()
        .await;

    let result = api::send_rich_html_id(
        &server.url(),
        "TESTTOKEN",
        67890,
        None,
        "<b>bold</b>",
        None,
        "test",
        "-",
    )
    .await;

    assert!(result.is_ok(), "send should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), 99);
    mock.assert_async().await;
}

#[tokio::test]
async fn edit_rich_html_uses_custom_api_url() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/botTESTTOKEN/editMessageText")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"result":true}"#)
        .create_async()
        .await;

    let result = api::edit_rich_html(
        &server.url(),
        "TESTTOKEN",
        12345,
        1,
        "<b>edited</b>",
        None,
        "test",
        "-",
    )
    .await;

    assert!(result.is_ok(), "edit should succeed: {:?}", result.err());
    mock.assert_async().await;
}

#[tokio::test]
async fn edit_rich_markdown_media_url_entry_uses_custom_api_url() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/botTESTTOKEN/editMessageText")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "chat_id": 12345,
            "message_id": 7,
            "rich_message": {
                "markdown": "![diagram](tg://photo?id=diag0)",
                "media": [
                    {"id": "diag0", "media": {"type": "photo", "media": "https://mermaid.ink/img/abc"}}
                ]
            }
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"result":true}"#)
        .create_async()
        .await;

    let kb = serde_json::json!({"inline_keyboard": [[{"text": "b", "callback_data": "cb0"}]]});
    let result = api::edit_rich_markdown_media(
        &server.url(),
        "TESTTOKEN",
        12345,
        7,
        "![diagram](tg://photo?id=diag0)",
        &[crate::channels::telegram::rich::mermaid::MediaEntry {
            id: "diag0".to_string(),
            url: Some("https://mermaid.ink/img/abc".to_string()),
            bytes: None,
            kind: crate::channels::telegram::rich::mermaid::MediaKind::Photo,
            name: None,
        }],
        Some(&kb),
        "test",
        "-",
    )
    .await;

    assert!(
        result.is_ok(),
        "media edit should succeed: {:?}",
        result.err()
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn send_rich_markdown_media_target_id_uses_custom_api_url() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"result":{"message_id":77}}"#)
        .create_async()
        .await;

    let result = api::send_rich_markdown_media_target_id(
        &server.url(),
        "TESTTOKEN",
        11111,
        None,
        None,
        "![img](tg://photo?id=1)",
        &[crate::channels::telegram::rich::mermaid::MediaEntry {
            id: "1".to_string(),
            url: Some("https://example.com/img.png".to_string()),
            bytes: None,
            kind: crate::channels::telegram::rich::mermaid::MediaKind::Photo,
            name: None,
        }],
        "test",
        "-",
    )
    .await;

    assert!(
        result.is_ok(),
        "media send should succeed: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), 77);
    mock.assert_async().await;
}

// ── Base-URL normalisation (#1117) ───────────────────────────────────
//
// The tests above pass `mockito::Server::url()`, which has no trailing
// slash. Production passes `Bot::api_url().as_str()`, and the URL spec
// normalises an empty path to `/`, so that value DOES end in one. String
// concatenation then produced `https://api.telegram.org//bot<token>/method`,
// Telegram rejected it, and every rich send fell back to plain HTML — tool
// blocks stopped rendering rich and completions arrived as separate
// messages. These pin the shape production actually uses.

#[tokio::test]
async fn a_base_with_a_trailing_slash_does_not_double_the_separator() {
    let mut server = mockito::Server::new_async().await;
    // Exactly what `Bot::api_url().as_str()` yields: a trailing slash.
    let base_with_slash = format!("{}/", server.url());

    let hit = server
        .mock("POST", "/botTOKEN/sendRichMessage")
        .with_status(200)
        .with_body(r#"{"ok":true,"result":{"message_id":1}}"#)
        .create_async()
        .await;

    let _ = api::send_rich_html_id(
        &base_with_slash,
        "TOKEN",
        123,
        None,
        "<b>hi</b>",
        None,
        "test",
        "-",
    )
    .await;

    // Asserts the single-slash path. A double slash would miss this mock.
    hit.assert_async().await;
}

#[tokio::test]
async fn a_base_without_a_trailing_slash_still_works() {
    // The mockito shape, kept so trimming cannot regress the other direction.
    let mut server = mockito::Server::new_async().await;
    let hit = server
        .mock("POST", "/botTOKEN/sendRichMessage")
        .with_status(200)
        .with_body(r#"{"ok":true,"result":{"message_id":1}}"#)
        .create_async()
        .await;

    let _ = api::send_rich_html_id(
        &server.url(),
        "TOKEN",
        123,
        None,
        "<b>hi</b>",
        None,
        "test",
        "-",
    )
    .await;

    hit.assert_async().await;
}

/// #629 — the HTML fallback must render with the paragraph-wrapping variant.
///
/// The rich HTML dialect treats a bare newline as INSIGNIFICANT whitespace, so
/// a fallback rendered by the bare `markdown_to_html_mermaid` joins every block
/// with a bare newline and the whole reply arrives as one wall of text. This
/// drives the real fallback leg end to end: the primary markdown+media send is
/// failed by the mock, and the assertion is on the body the fallback sent.
///
/// The media leg is primed through the render cache rather than the network.
/// `markdown_to_html_mermaid_p` resolves each fence through `resolve_blocks`,
/// which calls `resolve`, which consults `cache_get` first — so a
/// `cache_put(ImageBytes)` makes the fence resolve offline and keeps the test
/// hermetic. `MERMAID_FENCE_SOURCE` is shared with that `cache_put`: a mismatch
/// would leave the cache cold, the resolver would reach for the network, and
/// the test would fail on a timeout rather than on the assertion.
#[tokio::test]
async fn the_html_fallback_wraps_each_block_in_its_own_p_tag() {
    const MERMAID_FENCE_SOURCE: &str = "graph TD\n    Fallback629Probe --> B";

    // Serialize against the other suites that swap the process-wide config
    // mirror (governor_gates, governor_spacing_floor, stale_topic_eviction):
    // all four take this same guard. Restore the pre-test mirror on the way
    // out so the swap cannot leak sideways. The cooldown lock is the
    // cross-family token: the Discord and Slack suites swap this same
    // mirror under their own registries, so this guard alone cannot
    // serialize against them (#2018).
    let _guard = crate::channels::telegram::governor::test_support::registry_guard().await;
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let previous = crate::config::Config::current();

    // `should_render_mermaid` reads `rich_messages && mermaid_render` from the
    // live mirror, and `send_rich_with_mermaid_target_id` early-returns to the
    // no-media path — which has no HTML fallback at all — when it is false. Pin
    // both on explicitly rather than relying on their `default_true` serde
    // default, so the test exercises the fallback it names whatever the ambient
    // mirror holds.
    let mut pinned = (*previous).clone();
    pinned.channels.telegram.rich_messages = true;
    pinned.channels.telegram.mermaid_render = true;
    crate::config::Config::set_current(pinned);

    cache_put(
        MERMAID_FENCE_SOURCE,
        &MermaidResult::ImageBytes(png_ihdr(800, 600)),
    );

    let body = "First paragraph.\n\nSecond paragraph.\n\n\
                ```mermaid\ngraph TD\n    Fallback629Probe --> B\n```";

    let mut server = mockito::Server::new_async().await;

    // Primary leg: identified by its `media` array, which the HTML body never
    // carries. 400 (not 429) so `post_rich` reports it without retrying.
    let primary = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .match_body(mockito::Matcher::Regex(r#""media"\s*:\s*\["#.to_string()))
        .with_status(400)
        .with_body(r#"{"ok":false,"description":"Bad Request: can't parse rich message"}"#)
        .expect(1)
        .create_async()
        .await;

    // Fallback leg: its html body must carry one <p> per block.
    let fallback = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .match_body(mockito::Matcher::Regex(
            r#"<p>First paragraph\.</p>"#.to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"result":{"message_id":7}}"#)
        .expect(1)
        .create_async()
        .await;

    let id = api::send_rich_with_mermaid_target_id(
        &server.url(),
        "TESTTOKEN",
        12345,
        None,
        None,
        body,
        "test",
        "-",
    )
    .await;

    assert_eq!(
        id.expect("the html fallback must deliver the message"),
        7,
        "the fallback leg must return its own message id"
    );
    // `expect(1)` on the fallback mock is the guard against the no-media early
    // return: if that return is taken the fallback is never called and this
    // assertion fails rather than the test passing vacuously.
    fallback.assert_async().await;
    primary.assert_async().await;

    // Restore the mirror the guard is still holding.
    crate::config::Config::set_current((*previous).clone());
}

/// A minimal PNG carrying a real IHDR width/height — enough for `png_dims`.
fn png_ihdr(w: u32, h: u32) -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&w.to_be_bytes());
    png.extend_from_slice(&h.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&[0, 0, 0, 0]);
    png
}

// ── Document entries (#1918) ─────────────────────────────────────────
//
// A file reference in a report is not a photo: the media array's type
// literal must say "document", and inline bytes must upload as a part
// named after the file with its own MIME, which is what Telegram renders
// in the document bubble.

#[tokio::test]
async fn a_document_entry_announces_itself_as_a_document() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "rich_message": {
                "markdown": "![📎 q3 report](tg://document?id=doc0)",
                "media": [
                    {"id": "doc0", "media": {"type": "document", "media": "https://example.com/q3.pdf"}}
                ]
            }
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"result":{"message_id":81}}"#)
        .create_async()
        .await;

    let result = api::send_rich_markdown_media_target_id(
        &server.url(),
        "TESTTOKEN",
        11111,
        None,
        None,
        "![📎 q3 report](tg://document?id=doc0)",
        &[crate::channels::telegram::rich::mermaid::MediaEntry {
            id: "doc0".to_string(),
            url: Some("https://example.com/q3.pdf".to_string()),
            bytes: None,
            kind: crate::channels::telegram::rich::mermaid::MediaKind::Document,
            name: Some("q3.pdf".to_string()),
        }],
        "test",
        "-",
    )
    .await;

    assert!(
        result.is_ok(),
        "document send should succeed: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), 81);
    mock.assert_async().await;
}

#[tokio::test]
async fn document_bytes_upload_as_a_named_multipart_part() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/botTESTTOKEN/sendRichMessage")
        .match_body(mockito::Matcher::Regex(
            r#"filename="q3\.pdf"[\s\S]*application/pdf"#.to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"result":{"message_id":82}}"#)
        .create_async()
        .await;

    let result = api::send_rich_markdown_media_target_id(
        &server.url(),
        "TESTTOKEN",
        11111,
        None,
        None,
        "![📎 q3 report](tg://document?id=doc0)",
        &[crate::channels::telegram::rich::mermaid::MediaEntry {
            id: "doc0".to_string(),
            url: None,
            bytes: Some(b"%PDF-1.7\nfake bytes\n".to_vec()),
            kind: crate::channels::telegram::rich::mermaid::MediaKind::Document,
            name: Some("q3.pdf".to_string()),
        }],
        "test",
        "-",
    )
    .await;

    assert!(
        result.is_ok(),
        "multipart document send should succeed: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), 82);
    mock.assert_async().await;
}

#[test]
fn document_mime_covers_the_types_a_session_produces() {
    use crate::channels::telegram::rich::api::document_mime;
    assert_eq!(document_mime("report.pdf"), "application/pdf");
    assert_eq!(
        document_mime("notes.TXT"),
        "text/plain",
        "extension match is case-blind"
    );
    assert_eq!(document_mime("events.json"), "application/json");
    assert_eq!(document_mime("main.rs"), "text/rust");
    assert_eq!(document_mime("lib.py"), "text/x-python");
    assert_eq!(document_mime("page.html"), "text/html");
    assert_eq!(
        document_mime("archive.tar.gz"),
        "application/gzip",
        "the last extension wins"
    );
    assert_eq!(
        document_mime("blob"),
        "application/octet-stream",
        "no extension: generic"
    );
}

#[test]
fn a_part_identity_follows_the_entry_kind() {
    use crate::channels::telegram::rich::api::media_part_identity;
    use crate::channels::telegram::rich::mermaid::{MediaEntry, MediaKind};

    let photo = MediaEntry {
        id: "d1".to_string(),
        url: None,
        bytes: Some(Vec::new()),
        kind: MediaKind::Photo,
        name: None,
    };
    assert_eq!(
        media_part_identity(&photo),
        ("d1.png".to_string(), "image/png")
    );

    let named = MediaEntry {
        id: "doc0".to_string(),
        url: None,
        bytes: Some(Vec::new()),
        kind: MediaKind::Document,
        name: Some("q3.pdf".to_string()),
    };
    assert_eq!(
        media_part_identity(&named),
        ("q3.pdf".to_string(), "application/pdf")
    );

    let unnamed = MediaEntry {
        id: "doc1".to_string(),
        url: None,
        bytes: Some(Vec::new()),
        kind: MediaKind::Document,
        name: None,
    };
    assert_eq!(
        media_part_identity(&unnamed),
        ("doc1.bin".to_string(), "application/octet-stream")
    );
}
