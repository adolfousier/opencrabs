//! Tests that a failed screenshot attach surfaces visibly in the
//! ToolResult — previously the `if let Some(img) = ...` pattern
//! silently dropped the image on failure, so the model got the text
//! reply with no indication that the expected visual was missing.
//!
//! We can't launch Chrome in a unit test, so we exercise
//! `attach_screenshot` against a fresh manager with no pages — the
//! `take_screenshot_for_session` call returns None (no page exists
//! for the session) and we verify the failure surface.

#![cfg(feature = "browser")]

use crate::brain::tools::ToolResult;
use crate::brain::tools::browser::BrowserManager;
use uuid::Uuid;

#[tokio::test]
async fn attach_screenshot_on_unknown_session_marks_failure() {
    let mgr = BrowserManager::new();
    let mut result = ToolResult::success("navigated".to_string());
    mgr.attach_screenshot(Uuid::new_v4(), &mut result).await;

    assert!(
        result.images.is_empty(),
        "no session page → no image attached"
    );
    assert_eq!(
        result.metadata.get("screenshot").map(String::as_str),
        Some("failed"),
        "metadata must flag the failure so callers / UIs can show it"
    );
    assert!(
        result.output.contains("[screenshot unavailable"),
        "output text must explain the absence to the model: {}",
        result.output
    );
}

#[tokio::test]
async fn attach_screenshot_preserves_existing_output_text() {
    // The failure note is appended, not substituted — the primary
    // reply ("navigated to X") must still reach the model intact.
    let mgr = BrowserManager::new();
    let mut result = ToolResult::success("Navigated to https://example.com".to_string());
    mgr.attach_screenshot(Uuid::new_v4(), &mut result).await;
    assert!(
        result
            .output
            .starts_with("Navigated to https://example.com")
    );
    assert!(result.output.contains("[screenshot unavailable"));
}
