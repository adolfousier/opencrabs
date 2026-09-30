//! #1792 regression guards for analyze_image failure surfacing.
//!
//! The chain used to surface only the LAST candidate's error ("All provider
//! vision candidates failed: Vision API error 400: {MissingSessionID}"), which
//! read like a blanket verdict on every provider while hiding the single
//! misconfigured entry. These pin the per-candidate aggregate and the
//! char-safe reason clip.

use crate::brain::tools::provider_vision::{
    ProviderVisionTool, VISION_FAILURE_REASON_LIMIT, clip_failure_reason, format_vision_failures,
};
use crate::brain::tools::{Tool, ToolExecutionContext};
use serde_json::json;
use uuid::Uuid;

#[test]
fn short_reasons_pass_through_the_clip() {
    let r = "Vision API error 400: {\"type\":\"error\"}";
    assert_eq!(clip_failure_reason(r), r);
    assert_eq!(clip_failure_reason(""), "");
}

#[test]
fn long_reasons_are_clipped_char_safely() {
    let huge = "x".repeat(VISION_FAILURE_REASON_LIMIT + 500);
    let clipped = clip_failure_reason(&huge);
    assert!(
        clipped.ends_with("... [truncated]"),
        "marker present: {clipped}"
    );
    assert_eq!(
        clipped.chars().count(),
        VISION_FAILURE_REASON_LIMIT + "... [truncated]".chars().count(),
        "clip is char-count based, not byte-sliced"
    );
    // A clip landing inside a multibyte char must not panic.
    let emo = "\u{1f980}".repeat(VISION_FAILURE_REASON_LIMIT + 10);
    let _ = clip_failure_reason(&emo);
}

#[test]
fn the_aggregate_names_every_candidate() {
    let failures = vec![
        "gpt-vision @ https://api.one.example/v1: Vision API error 401: bad key".to_string(),
        "stealth/m @ https://api.opencode.ai/v1: Vision API error 400: MissingSessionID"
            .to_string(),
    ];
    let out = format_vision_failures(&failures);
    assert!(
        out.contains("All 2 provider vision candidates failed:"),
        "count header present: {out}"
    );
    assert!(
        out.contains("[1] gpt-vision @ https://api.one.example/v1"),
        "first candidate visible: {out}"
    );
    assert!(
        out.contains("[2] stealth/m"),
        "last candidate visible: {out}"
    );
    assert!(
        out.contains("MissingSessionID"),
        "the 400 reason is kept: {out}"
    );
    assert!(
        out.contains("401"),
        "the earlier candidate's reason is kept too: {out}"
    );
}

#[tokio::test]
async fn a_dead_chain_surfaces_both_candidates() {
    // Two unreachable local endpoints (ports 1 and 2 are not listening);
    // the surfaced error must name BOTH, not just the final failure (#1792).
    let img = std::env::temp_dir().join(format!("pv1792-{}.png", Uuid::new_v4()));
    std::fs::write(&img, [0x89u8, 0x50, 0x4e, 0x47]).expect("temp image writes");
    let tool = ProviderVisionTool::with_candidates(vec![
        (
            "k".to_string(),
            "http://127.0.0.1:1/v1".to_string(),
            "model-a".to_string(),
        ),
        (
            "k".to_string(),
            "http://127.0.0.1:2/v1".to_string(),
            "model-b".to_string(),
        ),
    ]);
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    let res = tool
        .execute(
            json!({"image": img.to_string_lossy(), "question": "describe"}),
            &ctx,
        )
        .await
        .expect("chain failure is a ToolResult, not a transport error");
    let _ = std::fs::remove_file(&img);
    assert!(!res.success);
    let err = res.error.expect("aggregated failure message present");
    assert!(
        err.contains("All 2 provider vision candidates failed"),
        "{err}"
    );
    assert!(
        err.contains("[1] model-a @ http://127.0.0.1:1/v1"),
        "first candidate surfaced: {err}"
    );
    assert!(
        err.contains("[2] model-b @ http://127.0.0.1:2/v1"),
        "last candidate surfaced: {err}"
    );
}
