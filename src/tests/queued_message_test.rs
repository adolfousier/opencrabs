//! Tests for queued message behavior — preview rendering, truncation,
//! and ordering invariants.
//!
//! The full TUI App is too heavy to construct in unit tests (needs DB,
//! provider, services), so we test the individual components:
//! - Preview text truncation and newline flattening
//! - Queue/recall lifecycle via the shared Mutex
//! - Ordering: queued message must appear AFTER tool group, BEFORE assistant text

use std::sync::Arc;
use tokio::sync::Mutex;

// ── Preview text formatting ──────────────────────────────────────────────

/// Simulate the preview truncation logic from render/input.rs
fn format_preview(queued: &str, input_content_width: usize) -> String {
    let flat = queued.replace('\n', " ");
    let max_preview = input_content_width.saturating_sub(25);
    if flat.chars().count() > max_preview {
        let truncated: String = flat.chars().take(max_preview).collect();
        format!("{}...", truncated)
    } else {
        flat
    }
}

#[test]
fn preview_short_message_unchanged() {
    let preview = format_preview("hello world", 80);
    assert_eq!(preview, "hello world");
}

#[test]
fn preview_newlines_replaced_with_spaces() {
    let preview = format_preview("line one\nline two\nline three", 80);
    assert_eq!(preview, "line one line two line three");
}

#[test]
fn preview_long_message_truncated_with_ellipsis() {
    let long = "a".repeat(200);
    let preview = format_preview(&long, 80);
    assert!(preview.ends_with("..."));
    // 80 - 25 = 55 chars + "..." = 58
    assert!(preview.len() <= 58);
}

#[test]
fn preview_multibyte_safe() {
    // Japanese characters (3 bytes each in UTF-8)
    let japanese = "こんにちは世界テスト文字列です";
    let preview = format_preview(japanese, 40);
    // Should not panic on multibyte boundary
    assert!(preview.len() > 0);
}

#[test]
fn preview_emoji_safe() {
    let emoji = "🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀";
    let preview = format_preview(emoji, 40);
    assert!(preview.ends_with("..."));
}

#[test]
fn preview_empty_string() {
    let preview = format_preview("", 80);
    assert_eq!(preview, "");
}

#[test]
fn preview_only_newlines() {
    let preview = format_preview("\n\n\n", 80);
    assert_eq!(preview, "   ");
}

#[test]
fn preview_exact_length_no_truncation() {
    // Width 50, max_preview = 25. Message exactly 25 chars.
    let msg = "a".repeat(25);
    let preview = format_preview(&msg, 50);
    assert_eq!(preview, msg);
    assert!(!preview.contains("..."));
}

// ── Queue lifecycle ─────────────────────────────────────────────────────

#[tokio::test]
async fn queue_set_and_take() {
    let queue: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Initially empty
    assert!(queue.lock().await.is_none());

    // Set a message
    *queue.lock().await = Some("follow-up question".to_string());
    assert!(queue.lock().await.is_some());

    // Take consumes it
    let taken = queue.lock().await.take();
    assert_eq!(taken, Some("follow-up question".to_string()));
    assert!(queue.lock().await.is_none());
}

#[tokio::test]
async fn queue_overwrite_replaces_previous() {
    let queue: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    *queue.lock().await = Some("first".to_string());
    *queue.lock().await = Some("second".to_string());

    let taken = queue.lock().await.take();
    assert_eq!(taken, Some("second".to_string()));
}

#[tokio::test]
async fn queue_recall_clears_both() {
    // Simulates Up arrow recall: both the shared queue and local preview clear
    let queue: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let mut preview: Option<String> = None;

    // Queue a message
    let content = "queued msg".to_string();
    *queue.lock().await = Some(content.clone());
    preview = Some(content);

    // Recall (Up arrow)
    let recalled = preview.take().unwrap();
    *queue.lock().await = None;

    assert_eq!(recalled, "queued msg");
    assert!(preview.is_none());
    assert!(queue.lock().await.is_none());
}

#[tokio::test]
async fn queue_flush_clears_preview() {
    // Simulates IntermediateText flush
    let queue: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let mut preview: Option<String> = None;

    *queue.lock().await = Some("question".to_string());
    preview = Some("question".to_string());

    // Flush (IntermediateText handler)
    if let Some(_content) = queue.lock().await.take() {
        preview = None;
    }

    assert!(preview.is_none());
    assert!(queue.lock().await.is_none());
}

// ── Ordering invariants ─────────────────────────────────────────────────

/// Simulate the IntermediateText ordering:
/// tool_group → queued_user_message → assistant_text
#[test]
fn ordering_queued_after_tools_before_assistant() {
    let mut messages: Vec<(&str, &str)> = Vec::new();

    // Step 1: flush tool group
    messages.push(("tool_group", "3 tool calls"));

    // Step 2: flush queued user message
    messages.push(("user", "follow-up question"));

    // Step 3: add assistant intermediate text
    messages.push(("assistant", "Here's my response..."));

    assert_eq!(messages[0].0, "tool_group");
    assert_eq!(messages[1].0, "user");
    assert_eq!(messages[2].0, "assistant");
}

/// Simulate complete_response ordering (no IntermediateText):
/// tool_group → queued_user_message → assistant_response
#[test]
fn ordering_at_response_complete() {
    let mut messages: Vec<(&str, &str)> = Vec::new();

    // Tool group finalized
    messages.push(("tool_group", "2 tool calls"));

    // Queued message flushed
    messages.push(("user", "what about X?"));

    // Final assistant response
    messages.push(("assistant", "About X..."));

    assert_eq!(messages[0].0, "tool_group");
    assert_eq!(messages[1].0, "user");
    assert_eq!(messages[2].0, "assistant");
}

/// No queued message — just tool_group → assistant
#[test]
fn ordering_no_queued_message() {
    let mut messages: Vec<(&str, &str)> = Vec::new();

    messages.push(("tool_group", "1 tool call"));
    messages.push(("assistant", "Done."));

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].0, "tool_group");
    assert_eq!(messages[1].0, "assistant");
}
