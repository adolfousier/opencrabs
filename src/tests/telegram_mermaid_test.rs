//! Tests for the Telegram mermaid-diagram render path (#1044).
//!
//! Pure, network-free coverage of the building blocks in
//! [`crate::channels::telegram::rich::mermaid`]: base64url encoding, fence
//! detection, the HTTP accept/reject decision, the failure-note builder, the
//! image/failure HTML shapes, and the block-resolution walk (exercised only
//! on non-mermaid blocks so no HTTP is performed). The config-gated entry
//! point `should_render_mermaid` is not unit-tested because it reads the live
//! `Config`, whose values in tests depend on the embedded example config.

use crate::channels::telegram::rich::api::{
    build_body_markdown_media_target, multipart_scalar_fields,
};
use crate::channels::telegram::rich::ast::{Block, Inline, MermaidResult};
use crate::channels::telegram::rich::markdown_to_html_mermaid;
use crate::channels::telegram::rich::mermaid::{
    MediaEntry, base64url, cache_get, cache_put, classify_render_failure, error_note, failure_html,
    find_mermaid_fences, has_mermaid_fence, image_html, ink_url, is_image_response,
    looks_like_mermaid_source, markdown_failure_block, replacement_for, resolve_blocks,
    resolve_markdown_media,
};

// ---------------------------------------------------------------------------
// base64url
// ---------------------------------------------------------------------------

#[test]
fn base64url_matches_rfc4648_url_safe_no_pad() {
    // No special chars, padding stripped.
    assert_eq!(base64url("hello world"), "aGVsbG8gd29ybGQ");
    // Standard '+' maps to '-'.
    assert_eq!(base64url("~~~"), "fn5-");
    // Standard '/' maps to '_' (and padding still stripped).
    assert_eq!(base64url("????"), "Pz8_Pw");
}

#[test]
fn base64url_output_never_contains_forbidden_chars() {
    for input in [
        "a",
        "ab",
        "abc",
        "mermaid graph TD; A-->B",
        "héllo wörld",
        "????~~~~",
    ] {
        let out = base64url(input);
        assert!(!out.contains('+'), "unexpected '+' in {out}");
        assert!(!out.contains('/'), "unexpected '/' in {out}");
        assert!(!out.contains('='), "unexpected '=' padding in {out}");
    }
}

#[test]
fn base64url_round_trips() {
    use base64::Engine as _;
    for input in ["graph TD; A-->B;", "flowchart LR\n  X --> Y", "ünïcode ✓"] {
        let encoded = base64url(input);
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .expect("base64url must decode");
        assert_eq!(String::from_utf8(decoded).unwrap(), input);
    }
}

// ---------------------------------------------------------------------------
// has_mermaid_fence
// ---------------------------------------------------------------------------

#[test]
fn has_mermaid_fence_detects_tagged_fence() {
    assert!(has_mermaid_fence("```mermaid\ngraph TD;\n```"));
    assert!(has_mermaid_fence("before\n```mermaid\nA-->B\n```\nafter"));
}

#[test]
fn has_mermaid_fence_is_case_insensitive_and_tolerates_space() {
    assert!(has_mermaid_fence("```Mermaid\ngraph TD;\n```"));
    assert!(has_mermaid_fence("``` mermaid\ngraph TD;\n```"));
}

#[test]
fn has_mermaid_fence_rejects_other_or_missing_fences() {
    assert!(!has_mermaid_fence("```rust\nfn main() {}\n```"));
    assert!(!has_mermaid_fence("plain prose, no fences"));
    assert!(!has_mermaid_fence("I like mermaid diagrams"));
    assert!(!has_mermaid_fence("```\nuntagged fence\n```"));
}

// ---------------------------------------------------------------------------
// is_image_response
// ---------------------------------------------------------------------------

#[test]
fn is_image_response_accepts_2xx_image() {
    assert!(is_image_response(200, "image/jpeg"));
    assert!(is_image_response(200, "image/png"));
    assert!(is_image_response(200, "image/svg+xml"));
    assert!(is_image_response(200, "image/png; charset=binary"));
    assert!(is_image_response(204, "image/webp"));
}

#[test]
fn is_image_response_is_case_insensitive_on_content_type() {
    assert!(is_image_response(200, "IMAGE/PNG"));
    assert!(is_image_response(200, "Image/Jpeg"));
}

#[test]
fn is_image_response_rejects_non_2xx_or_non_image() {
    assert!(!is_image_response(400, "image/jpeg"));
    assert!(!is_image_response(500, "image/png"));
    assert!(!is_image_response(200, "text/plain"));
    assert!(!is_image_response(200, "text/html"));
    assert!(!is_image_response(200, ""));
    // 300 is outside the 2xx success range.
    assert!(!is_image_response(300, "image/png"));
}

// ---------------------------------------------------------------------------
// error_note
// ---------------------------------------------------------------------------

#[test]
fn error_note_returns_body_when_present() {
    assert_eq!(
        error_note(400, "Parse error on line 2: got 'LINK'"),
        "Parse error on line 2: got 'LINK'"
    );
}

#[test]
fn error_note_trims_whitespace() {
    assert_eq!(error_note(400, "   some error   "), "some error");
}

#[test]
fn error_note_falls_back_to_status_on_empty_body() {
    assert_eq!(error_note(500, ""), "diagram renderer returned HTTP 500");
    assert_eq!(error_note(400, "   "), "diagram renderer returned HTTP 400");
}

#[test]
fn error_note_caps_length() {
    let long = "x".repeat(1000);
    let note = error_note(400, &long);
    assert_eq!(note.chars().count(), 400);
}

// ---------------------------------------------------------------------------
// image_html / failure_html
// ---------------------------------------------------------------------------

#[test]
fn image_html_wraps_url_in_figure() {
    assert_eq!(
        image_html("https://mermaid.ink/img/abc123"),
        "<figure><img src=\"https://mermaid.ink/img/abc123\"/></figure>"
    );
}

#[test]
fn image_html_escapes_url_entities() {
    assert_eq!(
        image_html("a&b<c>"),
        "<figure><img src=\"a&amp;b&lt;c&gt;\"/></figure>"
    );
}

#[test]
fn failure_html_contains_warning_error_and_source() {
    let html = failure_html("Parse error on line 2", "graph TD; A-->B");
    assert!(html.contains("<b>⚠️ Mermaid diagram could not be rendered</b>"));
    assert!(html.contains("<blockquote>Parse error on line 2</blockquote>"));
    assert!(html.contains("<pre><code>graph TD; A--&gt;B</code></pre>"));
}

#[test]
fn failure_html_escapes_error_and_source() {
    let html = failure_html("<script>alert(1)</script>", "a < b & c");
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("a &lt; b &amp; c"));
    assert!(!html.contains("<script>"));
}

// ---------------------------------------------------------------------------
// resolve_blocks (non-mermaid only — no network)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_blocks_passes_through_non_mermaid() {
    let blocks = vec![
        Block::Paragraph(vec![Inline::Text("hello".into())]),
        Block::Code {
            lang: Some("rust".into()),
            text: "fn main() {}".into(),
        },
    ];
    let resolved = resolve_blocks(blocks.clone()).await;
    assert_eq!(resolved, blocks, "non-mermaid blocks must be untouched");
}

#[tokio::test]
async fn resolve_blocks_empty_input() {
    let resolved = resolve_blocks(Vec::new()).await;
    assert!(resolved.is_empty());
}

// ---------------------------------------------------------------------------
// markdown_to_html_mermaid (no mermaid fence — exercises the full
// parse -> resolve -> render pipeline without any network call)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn markdown_to_html_mermaid_renders_plain_markdown() {
    let html = markdown_to_html_mermaid("# Hi\n\nSome **bold** text.").await;
    assert_eq!(html, "<b>Hi</b>\n\nSome <b>bold</b> text.");
}

// ---------------------------------------------------------------------------
// find_mermaid_fences
// ---------------------------------------------------------------------------

#[test]
fn find_mermaid_fences_locates_single_fence_with_range_and_source() {
    let text = "before\n```mermaid\ngraph TD;\nA-->B\n```\nafter";
    let fences = find_mermaid_fences(text);
    assert_eq!(fences.len(), 1);
    let f = &fences[0];
    // start points at the opening fence line, end just past the closing one.
    assert_eq!(&text[f.start..f.end], "```mermaid\ngraph TD;\nA-->B\n```\n");
    assert_eq!(f.source, "graph TD;\nA-->B\n");
}

#[test]
fn find_mermaid_fences_locates_multiple_and_orders_them() {
    let text = "```mermaid\nA\n```\nmid\n```mermaid\nB\n```";
    let fences = find_mermaid_fences(text);
    assert_eq!(fences.len(), 2);
    assert_eq!(fences[0].source, "A\n");
    assert_eq!(fences[1].source, "B\n");
    assert!(fences[0].start < fences[1].start);
}

#[test]
fn find_mermaid_fences_ignores_non_mermaid_and_unclosed() {
    assert!(find_mermaid_fences("```rust\nfn main() {}\n```").is_empty());
    assert!(find_mermaid_fences("no fences").is_empty());
    // Unclosed fence: no closing ``` so nothing is captured.
    assert!(find_mermaid_fences("```mermaid\ngraph TD;\n").is_empty());
}

// ---------------------------------------------------------------------------
// replacement_for (pure, no network)
// ---------------------------------------------------------------------------

#[test]
fn replacement_for_image_emits_media_reference_and_entry() {
    let outcome = MermaidResult::Image("https://mermaid.ink/img/xyz".into());
    let (md, entry) = replacement_for(&outcome, 0, "graph TD;");
    assert_eq!(md, "![diagram](tg://photo?id=diag0)");
    let e = entry.expect("image outcome must carry a media entry");
    assert_eq!(e.id, "diag0");
    assert_eq!(e.url, Some("https://mermaid.ink/img/xyz".into()));
    assert!(e.bytes.is_none());
}

#[test]
fn replacement_for_image_uses_fence_index_in_id() {
    let outcome = MermaidResult::Image("u".into());
    let (md, entry) = replacement_for(&outcome, 3, "src");
    assert_eq!(md, "![diagram](tg://photo?id=diag3)");
    assert_eq!(entry.unwrap().id, "diag3");
}

#[test]
fn replacement_for_image_bytes_carries_png_and_no_url() {
    let outcome = MermaidResult::ImageBytes(vec![0x89, b'P', b'N', b'G', 0, 0, 0, 0]);
    let (md, entry) = replacement_for(&outcome, 1, "graph TD;");
    assert_eq!(md, "![diagram](tg://photo?id=diag1)");
    let e = entry.expect("bytes outcome must carry a media entry");
    assert_eq!(e.id, "diag1");
    assert!(e.url.is_none());
    assert_eq!(
        e.bytes.as_deref(),
        Some(&[0x89, b'P', b'N', b'G', 0, 0, 0, 0][..])
    );
}

#[test]
fn replacement_for_failed_emits_failure_block_and_no_entry() {
    let outcome = MermaidResult::Failed("Parse error".into());
    let (md, entry) = replacement_for(&outcome, 0, "graph TD;");
    assert!(
        entry.is_none(),
        "failed outcome must not carry a media entry"
    );
    assert!(md.contains("Mermaid diagram could not be rendered"));
    assert!(md.contains("Parse error"));
    assert!(md.contains("graph TD;"));
}

#[test]
fn replacement_for_parse_error_matches_failed_block_shape() {
    // #37: a deterministic parse rejection degrades to the same legible
    // failure block as a transient failure — the regen nudge is a
    // loop-side concern, delivery treats both identically.
    let outcome = MermaidResult::ParseError("Parse error on line 2: X".into());
    let (md, entry) = replacement_for(&outcome, 0, "graph TD;");
    assert!(
        entry.is_none(),
        "parse-error outcome must not carry a media entry"
    );
    assert!(md.contains("Mermaid diagram could not be rendered"));
    assert!(md.contains("Parse error on line 2: X"));
    assert!(md.contains("graph TD;"));
}

// ---------------------------------------------------------------------------
// classify_render_failure (#37)
// ---------------------------------------------------------------------------

#[test]
fn classify_render_failure_4xx_is_parse_error() {
    match classify_render_failure(400, "Parse error on line 3: syntax error") {
        MermaidResult::ParseError(note) => assert!(note.contains("Parse error on line 3")),
        other => panic!("expected ParseError, got {other:?}"),
    }
}

#[test]
fn classify_render_failure_transient_statuses_stay_failed() {
    // 408/429 are transient (timeout/rate limit) even though they are 4xx;
    // 5xx and anything odd is infra, not a model mistake.
    for status in [408u16, 429, 500, 502] {
        match classify_render_failure(status, "body") {
            MermaidResult::Failed(note) => assert!(note.contains("body")),
            other => panic!("status {status}: expected Failed, got {other:?}"),
        }
    }
}

#[test]
fn classify_render_failure_empty_body_names_status() {
    match classify_render_failure(400, "   ") {
        MermaidResult::ParseError(note) => assert!(note.contains("HTTP 400")),
        other => panic!("expected ParseError, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// render cache (#37)
// ---------------------------------------------------------------------------

#[test]
fn render_cache_hit_returns_cached_outcome() {
    let source = "graph TD\n    CacheHitProbe --> A";
    let outcome = MermaidResult::ParseError("Parse error on line 2".into());
    cache_put(source, &outcome);
    assert_eq!(cache_get(source), Some(outcome));
}

#[test]
fn render_cache_miss_on_unknown_source() {
    assert_eq!(cache_get("graph TD\n    NeverCachedProbe --> Z"), None);
}

#[test]
fn render_cache_never_stores_transient_failures() {
    let source = "graph TD\n    TransientProbe --> B";
    cache_put(
        source,
        &MermaidResult::Failed("diagram renderer timed out".into()),
    );
    assert_eq!(cache_get(source), None);
}

// ---------------------------------------------------------------------------
// markdown_failure_block
// ---------------------------------------------------------------------------

#[test]
fn markdown_failure_block_contains_warning_error_and_source() {
    let md = markdown_failure_block("Parse error on line 2", "graph TD; A-->B");
    assert!(md.contains("> ⚠️ **Mermaid diagram could not be rendered**"));
    assert!(md.contains("Parse error on line 2"));
    assert!(md.contains("graph TD; A-->B"));
    assert!(md.contains("Source:"));
}

// ---------------------------------------------------------------------------
// build_body_markdown_media_target (JSON shape, matches validated prototype 1073)
// ---------------------------------------------------------------------------

#[test]
fn build_body_markdown_media_target_matches_prototype_shape() {
    let media = vec![MediaEntry {
        id: "diag0".into(),
        url: Some("https://mermaid.ink/img/abc".into()),
        bytes: None,
    }];
    let body = build_body_markdown_media_target(-100, None, None, "text", &media);
    assert_eq!(body["chat_id"], -100);
    assert_eq!(body["rich_message"]["markdown"], "text");
    let arr = body["rich_message"]["media"]
        .as_array()
        .expect("media array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "diag0");
    assert_eq!(arr[0]["media"]["type"], "photo");
    assert_eq!(arr[0]["media"]["media"], "https://mermaid.ink/img/abc");
    assert!(body.get("message_thread_id").is_none());
}

#[test]
fn build_body_markdown_media_target_includes_thread_id_when_present() {
    use teloxide::types::{MessageId, ThreadId};
    let body =
        build_body_markdown_media_target(-100, Some(ThreadId(MessageId(249))), None, "m", &[]);
    assert_eq!(body["message_thread_id"], 249);
}

#[test]
fn build_body_markdown_media_target_bytes_entry_uses_attach_reference() {
    let media = vec![MediaEntry {
        id: "diag1".into(),
        url: None,
        bytes: Some(vec![0x89, b'P']),
    }];
    let body = build_body_markdown_media_target(-100, None, None, "text", &media);
    let arr = body["rich_message"]["media"]
        .as_array()
        .expect("media array");
    assert_eq!(arr[0]["id"], "diag1");
    assert_eq!(arr[0]["media"]["type"], "photo");
    assert_eq!(arr[0]["media"]["media"], "attach://diag1");
}

// ---------------------------------------------------------------------------
// resolve_markdown_media (no mermaid fence — no network)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_markdown_media_passes_through_without_fences() {
    let text = "# Title\n\nSome **bold** text and a table:\n\n| a | b |\n|---|---|\n| 1 | 2 |";
    let (resolved, media) = resolve_markdown_media(text).await;
    assert_eq!(resolved, text, "no-fence text must be byte-identical");
    assert!(media.is_empty());
}

// ---------------------------------------------------------------------------
// untagged fences (bare ```) classified by content
// ---------------------------------------------------------------------------

#[test]
fn has_mermaid_fence_detects_untagged_mermaid_bodies() {
    assert!(has_mermaid_fence("```\ngraph TD\nA-->B\n```"));
    assert!(has_mermaid_fence("```\nflowchart LR\nA-->B\n```"));
    assert!(has_mermaid_fence("```\nsequenceDiagram\nA->>B: hi\n```"));
    // Blank and %%-comment lines are skipped before classification.
    assert!(has_mermaid_fence("```\n\n%%{init: {}}%%\ngitGraph\n```"));
}

#[test]
fn has_mermaid_fence_rejects_untagged_non_mermaid_bodies() {
    // Plain code.
    assert!(!has_mermaid_fence("```\nSELECT * FROM t;\n```"));
    // DOT notation: digraph, or graph without a mermaid direction word.
    assert!(!has_mermaid_fence("```\ndigraph G { a -> b }\n```"));
    assert!(!has_mermaid_fence("```\ngraph G {\n  a -- b\n}\n```"));
    assert!(!has_mermaid_fence("```\ngraph\n```"));
}

#[test]
fn find_mermaid_fences_locates_untagged_fences_with_ranges() {
    let text = "before\n```\ngraph TD\nA-->B\n```\nafter";
    let fences = find_mermaid_fences(text);
    assert_eq!(fences.len(), 1);
    let f = &fences[0];
    // start points at the opening fence line, end just past the closing one.
    assert_eq!(&text[f.start..f.end], "```\ngraph TD\nA-->B\n```\n");
    assert_eq!(f.source, "graph TD\nA-->B\n");
}

#[test]
fn find_mermaid_fences_mixes_tagged_and_untagged_in_order() {
    let text = "```mermaid\nA\n```\nmid\n```\nflowchart TD\nB\n```";
    let fences = find_mermaid_fences(text);
    assert_eq!(fences.len(), 2);
    assert_eq!(fences[0].source, "A\n");
    assert_eq!(fences[1].source, "flowchart TD\nB\n");
    assert!(fences[0].start < fences[1].start);
}

#[test]
fn find_mermaid_fences_ignores_untagged_non_mermaid_and_unclosed() {
    assert!(find_mermaid_fences("```\nprint('hi')\n```").is_empty());
    // Unclosed untagged fence: nothing captured, same as tagged.
    assert!(find_mermaid_fences("```\ngraph TD\n").is_empty());
}

#[test]
fn looks_like_mermaid_source_matches_known_openers() {
    assert!(looks_like_mermaid_source("graph BT\na-->b"));
    assert!(looks_like_mermaid_source("flowchart LR\na-->b"));
    assert!(looks_like_mermaid_source("stateDiagram-v2\n[*] --> s1"));
    assert!(looks_like_mermaid_source(
        "erDiagram\nUSER ||--o{ POST : has"
    ));
    assert!(looks_like_mermaid_source(
        "%% comment\nclassDiagram\nclass A"
    ));
    assert!(!looks_like_mermaid_source("let x = 1;"));
    assert!(!looks_like_mermaid_source(""));
}

/// A fence carrying an explicit non-mermaid info string is never
/// content-classified (#1208 review).
///
/// Content classification is for BARE fences: the doc contract says "untagged
/// fences, whose info string is empty", and `resolve_block` enforces exactly
/// that with `lang.is_none()`. The two line scanners tested the info string of
/// the CLOSING fence instead, which is bare on essentially every block, so any
/// tagged fence with diagram-shaped content classified as a diagram and the
/// three render paths disagreed with each other.
#[test]
fn test_tagged_non_mermaid_fence_is_never_content_classified() {
    use crate::channels::telegram::rich::mermaid::{find_mermaid_fences, has_mermaid_fence};

    for text in [
        "```dot\ngraph TD\n  A --> B\n```",
        "```text\nsequenceDiagram\n  A->>B: hi\n```",
        "```rust\n// pie\nlet x = 1;\n```",
    ] {
        assert!(
            !has_mermaid_fence(text),
            "explicit info string must suppress content classification: {text:?}"
        );
        assert!(
            find_mermaid_fences(text).is_empty(),
            "and the extractor must agree with the gate: {text:?}"
        );
    }
}

/// The bare-fence case the feature exists for still works, and both scanners
/// agree on it.
#[test]
fn test_bare_fence_with_diagram_body_is_still_classified() {
    use crate::channels::telegram::rich::mermaid::{find_mermaid_fences, has_mermaid_fence};

    let text = "```\ngraph TD\n  A --> B\n```";
    assert!(has_mermaid_fence(text));
    assert_eq!(find_mermaid_fences(text).len(), 1);

    // And an explicit mermaid tag is unaffected by any of this.
    let tagged = "```mermaid\ngraph TD\n  A --> B\n```";
    assert!(has_mermaid_fence(tagged));
    assert_eq!(find_mermaid_fences(tagged).len(), 1);
}

// ---------------------------------------------------------------------------
// natural-size PNG embed URLs (#1238): the request carries no width/scale
// overrides, so mermaid.ink returns the diagram at its intrinsic size — the
// dimension ladder keeps that size inside Telegram's photo box.

#[test]
fn ink_url_requests_natural_size_png() {
    let url = ink_url("graph TD\n    A --> B");
    assert!(url.starts_with("https://mermaid.ink/img/"));
    assert!(
        url.ends_with("?type=png"),
        "expected natural-size params: {url}"
    );
    assert!(!url.contains("scale=") && !url.contains("width="));
}

#[test]
fn ink_url_payload_is_base64url_without_padding() {
    // The payload segment must be exactly what base64url() produces:
    // URL-safe alphabet, no '=' padding (mermaid.ink 404s on standard b64).
    let url = ink_url("pie\n    \"a\": 1");
    let payload = url
        .trim_start_matches("https://mermaid.ink/img/")
        .split('?')
        .next()
        .expect("payload before query string");
    assert!(!payload.contains('='), "padding leaked: {payload}");
    assert!(!payload.contains('+') && !payload.contains('/'));
}

#[test]
fn photo_fits_matches_the_measured_photo_box() {
    use crate::channels::telegram::rich::mermaid::photo_fits;
    // Measured live (#1238): natural 1611×3727 accepted, prod-params
    // 3200×7404 refused; the box edge is 9600 combined px.
    assert!(photo_fits(1611, 3727));
    assert!(photo_fits(1200, 2776));
    assert!(photo_fits(800, 1851));
    assert!(!photo_fits(3200, 7404));
    assert!(photo_fits(4800, 4800)); // exactly 9600: fits
    assert!(!photo_fits(4800, 4801)); // 9601: busts
}

// ---------------------------------------------------------------------------
// is_no_media_found: the NO_MEDIA_FOUND classifier that gates the single
// rich-send retry (#tg-mermaid-delivery-hardening). Mirrors the api.rs error
// shape — "Telegram rich API error ({status}): {desc}".
// ---------------------------------------------------------------------------

#[test]
fn no_media_found_detects_renderer_fetch_failure() {
    use crate::channels::telegram::delivery::is_no_media_found;
    let e = anyhow::anyhow!(
        "Telegram rich API error (400): RICH_MESSAGE_PHOTO_NO_MEDIA_FOUND: media fetch failed"
    );
    assert!(is_no_media_found(&e));
}

#[test]
fn no_media_found_rejects_structural_and_unrelated_errors() {
    use crate::channels::telegram::delivery::is_no_media_found;
    let structural =
        anyhow::anyhow!("Telegram rich API error (400): RICH_MESSAGE_CONTENT_REQUIRED");
    assert!(!is_no_media_found(&structural));
    let unrelated = anyhow::anyhow!("network unreachable");
    assert!(!is_no_media_found(&unrelated));
}

#[test]
fn no_media_found_sees_through_anyhow_context_wraps() {
    use crate::channels::telegram::delivery::is_no_media_found;
    let inner = anyhow::anyhow!("Telegram rich API error (400): RICH_MESSAGE_PHOTO_NO_MEDIA_FOUND");
    let wrapped = inner.context("while delivering final response");
    assert!(is_no_media_found(&wrapped));
}

// ---------------------------------------------------------------------------
// png_dims (bytes-delivery oversize guard)
// ---------------------------------------------------------------------------

#[test]
fn png_dims_parses_ihdr() {
    use crate::channels::telegram::rich::mermaid::png_dims;

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    png.extend_from_slice(&[0, 0, 0, 13]); // IHDR chunk length
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&1611u32.to_be_bytes());
    png.extend_from_slice(&3727u32.to_be_bytes());
    assert_eq!(png_dims(&png), Some((1611, 3727)));
}

#[test]
fn png_dims_rejects_non_png_and_short_buffers() {
    use crate::channels::telegram::rich::mermaid::png_dims;

    assert_eq!(png_dims(b"not a png at all...."), None);
    assert_eq!(png_dims(&[0x89, b'P']), None);
    let mut hdr = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    hdr.extend_from_slice(&[0, 0, 0, 13]);
    hdr.extend_from_slice(b"IDAT"); // wrong chunk type
    hdr.extend_from_slice(&[0u8; 16]);
    assert_eq!(png_dims(&hdr), None);
}

#[test]
fn multipart_scalar_fields_carries_message_id_for_edits_only() {
    let edit_body = serde_json::json!({
        "chat_id": -100,
        "message_id": 40827,
        "rich_message": { "markdown": "text", "media": [] },
    });
    let edit_parts = multipart_scalar_fields(&edit_body);
    assert!(edit_parts.contains(&("message_id".to_string(), "40827".to_string())));
    assert!(edit_parts.contains(&("chat_id".to_string(), "-100".to_string())));
    assert!(edit_parts.iter().any(|(name, _)| name == "rich_message"));

    let send_body = serde_json::json!({
        "chat_id": -100,
        "rich_message": { "markdown": "text", "media": [] },
    });
    let send_parts = multipart_scalar_fields(&send_body);
    assert!(!send_parts.iter().any(|(name, _)| name == "message_id"));
}
