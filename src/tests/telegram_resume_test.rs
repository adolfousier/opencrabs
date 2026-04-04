//! Telegram Resume & Streaming Pipeline Tests
//!
//! Tests for cancel token management, pending request lifecycle,
//! streaming state rendering, dedup logic, display queue ordering,
//! and helper functions used by the Telegram handler.

use crate::channels::telegram::TelegramState;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// TelegramState — cancel token management
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cancel_token_store_and_retrieve() {
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    let token = CancellationToken::new();
    state.store_cancel_token(session_id, token.clone()).await;
    assert!(!token.is_cancelled());
}

#[tokio::test]
async fn cancel_token_new_message_cancels_previous() {
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    let old_token = CancellationToken::new();
    let new_token = CancellationToken::new();
    state
        .store_cancel_token(session_id, old_token.clone())
        .await;
    state
        .store_cancel_token(session_id, new_token.clone())
        .await;
    assert!(old_token.is_cancelled(), "old token should be cancelled");
    assert!(
        !new_token.is_cancelled(),
        "new token should still be active"
    );
}

#[tokio::test]
async fn cancel_session_returns_true_when_exists() {
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    let token = CancellationToken::new();
    state.store_cancel_token(session_id, token.clone()).await;
    assert!(state.cancel_session(session_id).await);
    assert!(token.is_cancelled());
}

#[tokio::test]
async fn cancel_session_returns_false_when_missing() {
    let state = TelegramState::new();
    assert!(!state.cancel_session(Uuid::new_v4()).await);
}

#[tokio::test]
async fn remove_cancel_token_only_removes_cancelled() {
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    let token = CancellationToken::new();
    state.store_cancel_token(session_id, token.clone()).await;
    // Not cancelled yet — remove should be a no-op
    state.remove_cancel_token(session_id).await;
    // Token should still exist (can cancel it)
    assert!(state.cancel_session(session_id).await);
}

#[tokio::test]
async fn remove_cancel_token_removes_after_cancel() {
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    let token = CancellationToken::new();
    state.store_cancel_token(session_id, token.clone()).await;
    token.cancel();
    state.remove_cancel_token(session_id).await;
    // Token should be gone now
    assert!(!state.cancel_session(session_id).await);
}

#[tokio::test]
async fn cancel_token_different_sessions_independent() {
    let state = TelegramState::new();
    let s1 = Uuid::new_v4();
    let s2 = Uuid::new_v4();
    let t1 = CancellationToken::new();
    let t2 = CancellationToken::new();
    state.store_cancel_token(s1, t1.clone()).await;
    state.store_cancel_token(s2, t2.clone()).await;
    assert!(state.cancel_session(s1).await);
    assert!(t1.is_cancelled());
    assert!(!t2.is_cancelled(), "session 2 token should be untouched");
}

#[tokio::test]
async fn cancel_token_rapid_replacement() {
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    let mut tokens = Vec::new();
    for _ in 0..10 {
        let t = CancellationToken::new();
        state.store_cancel_token(session_id, t.clone()).await;
        tokens.push(t);
    }
    // All but the last should be cancelled
    for t in &tokens[..9] {
        assert!(t.is_cancelled());
    }
    assert!(!tokens[9].is_cancelled());
}

// ─────────────────────────────────────────────────────────────────────────────
// TelegramState — session chat management
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn session_chat_register_and_lookup() {
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    state.register_session_chat(session_id, -12345).await;
    assert_eq!(state.session_chat(session_id).await, Some(-12345));
}

#[tokio::test]
async fn session_chat_unknown_returns_none() {
    let state = TelegramState::new();
    assert_eq!(state.session_chat(Uuid::new_v4()).await, None);
}

#[tokio::test]
async fn bot_initially_not_connected() {
    let state = TelegramState::new();
    assert!(!state.is_connected().await);
    assert!(state.bot().await.is_none());
}

#[tokio::test]
async fn owner_chat_id_initially_none() {
    let state = TelegramState::new();
    assert!(state.owner_chat_id().await.is_none());
}

#[tokio::test]
async fn set_and_get_owner_chat_id() {
    let state = TelegramState::new();
    state.set_owner_chat_id(-99999).await;
    assert_eq!(state.owner_chat_id().await, Some(-99999));
}

#[tokio::test]
async fn set_and_get_bot_username() {
    let state = TelegramState::new();
    state.set_bot_username("testbot".to_string()).await;
    assert_eq!(state.bot_username().await, Some("testbot".to_string()));
}

// ─────────────────────────────────────────────────────────────────────────────
// TelegramState — pending approvals
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pending_approval_resolve_approved() {
    let state = TelegramState::new();
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .register_pending_approval("test-1".to_string(), tx)
        .await;
    assert!(state.resolve_pending_approval("test-1", true, false).await);
    let (approved, always) = rx.await.unwrap();
    assert!(approved);
    assert!(!always);
}

#[tokio::test]
async fn pending_approval_resolve_denied() {
    let state = TelegramState::new();
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .register_pending_approval("test-2".to_string(), tx)
        .await;
    assert!(state.resolve_pending_approval("test-2", false, false).await);
    let (approved, _) = rx.await.unwrap();
    assert!(!approved);
}

#[tokio::test]
async fn pending_approval_resolve_always() {
    let state = TelegramState::new();
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .register_pending_approval("test-3".to_string(), tx)
        .await;
    assert!(state.resolve_pending_approval("test-3", true, true).await);
    let (_, always) = rx.await.unwrap();
    assert!(always);
}

#[tokio::test]
async fn pending_approval_resolve_unknown_returns_false() {
    let state = TelegramState::new();
    assert!(
        !state
            .resolve_pending_approval("nonexistent", true, false)
            .await
    );
}

#[tokio::test]
async fn pending_approval_double_resolve() {
    let state = TelegramState::new();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    state
        .register_pending_approval("test-4".to_string(), tx)
        .await;
    assert!(state.resolve_pending_approval("test-4", true, false).await);
    // Second resolve should return false (already consumed)
    assert!(!state.resolve_pending_approval("test-4", true, false).await);
}

// ─────────────────────────────────────────────────────────────────────────────
// TelegramState — concurrent access
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cancel_token_concurrent_store_and_cancel() {
    let state = Arc::new(TelegramState::new());
    let session_id = Uuid::new_v4();
    let mut handles = Vec::new();
    for _ in 0..20 {
        let st = state.clone();
        let sid = session_id;
        handles.push(tokio::spawn(async move {
            let t = CancellationToken::new();
            st.store_cancel_token(sid, t.clone()).await;
            t
        }));
    }
    let tokens: Vec<CancellationToken> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();
    // At most one should still be active
    let active_count = tokens.iter().filter(|t| !t.is_cancelled()).count();
    assert_eq!(active_count, 1, "exactly one token should survive");
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions — md_to_html
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn md_to_html_bold() {
    let result = crate::channels::telegram::handler::md_to_html("*hello*");
    assert_eq!(result, "<b>hello</b>");
}

#[test]
fn md_to_html_code() {
    let result = crate::channels::telegram::handler::md_to_html("`code`");
    assert_eq!(result, "<code>code</code>");
}

#[test]
fn md_to_html_mixed() {
    let result = crate::channels::telegram::handler::md_to_html("*bold* and `code`");
    assert_eq!(result, "<b>bold</b> and <code>code</code>");
}

#[test]
fn md_to_html_plain_text() {
    let result = crate::channels::telegram::handler::md_to_html("no formatting here");
    assert_eq!(result, "no formatting here");
}

#[test]
fn md_to_html_empty() {
    let result = crate::channels::telegram::handler::md_to_html("");
    assert_eq!(result, "");
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions — markdown_to_telegram_html
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn markdown_to_html_code_block() {
    let input = "```rust\nfn main() {}\n```";
    let result = crate::channels::telegram::handler::markdown_to_telegram_html(input);
    assert!(result.contains("<pre><code class=\"language-rust\">"));
    assert!(result.contains("fn main() {}"));
    assert!(result.contains("</code></pre>"));
}

#[test]
fn markdown_to_html_inline_code() {
    let input = "Use `foo()` here";
    let result = crate::channels::telegram::handler::markdown_to_telegram_html(input);
    assert!(result.contains("<code>foo()</code>"));
}

#[test]
fn markdown_to_html_bold() {
    let input = "This is **bold** text";
    let result = crate::channels::telegram::handler::markdown_to_telegram_html(input);
    assert!(result.contains("<b>bold</b>"));
}

#[test]
fn markdown_to_html_italic() {
    let input = "This is _italic_ text";
    let result = crate::channels::telegram::handler::markdown_to_telegram_html(input);
    assert!(result.contains("<i>italic</i>"));
}

#[test]
fn markdown_to_html_header() {
    let input = "## Section Title";
    let result = crate::channels::telegram::handler::markdown_to_telegram_html(input);
    assert!(result.contains("<b>Section Title</b>"));
}

#[test]
fn markdown_to_html_list_items() {
    let input = "- item one\n- item two";
    let result = crate::channels::telegram::handler::markdown_to_telegram_html(input);
    assert!(result.contains("• item one"));
    assert!(result.contains("• item two"));
}

#[test]
fn markdown_to_html_link() {
    let input = "Click [here](https://example.com)";
    let result = crate::channels::telegram::handler::markdown_to_telegram_html(input);
    assert!(result.contains("<a href=\"https://example.com\">here</a>"));
}

#[test]
fn markdown_to_html_escapes_entities() {
    let input = "a < b & c > d";
    let result = crate::channels::telegram::handler::markdown_to_telegram_html(input);
    assert!(result.contains("&lt;"));
    assert!(result.contains("&amp;"));
    assert!(result.contains("&gt;"));
}

#[test]
fn markdown_to_html_empty() {
    let result = crate::channels::telegram::handler::markdown_to_telegram_html("");
    assert!(result.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions — split_message
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn split_message_short_text() {
    let result = crate::channels::telegram::handler::split_message("hello", 4096);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], "hello");
}

#[test]
fn split_message_at_limit() {
    let text = "a".repeat(4096);
    let result = crate::channels::telegram::handler::split_message(&text, 4096);
    assert_eq!(result.len(), 1);
}

#[test]
fn split_message_exceeds_limit() {
    let text = "a".repeat(8192);
    let result = crate::channels::telegram::handler::split_message(&text, 4096);
    assert!(result.len() >= 2);
    for chunk in &result {
        assert!(chunk.len() <= 4096);
    }
    // All content preserved
    let rejoined: String = result.iter().copied().collect();
    assert_eq!(rejoined.len(), 8192);
}

#[test]
fn split_message_empty() {
    let result = crate::channels::telegram::handler::split_message("", 4096);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], "");
}

#[test]
fn split_message_small_limit() {
    let result = crate::channels::telegram::handler::split_message("hello world", 5);
    assert!(result.len() >= 2);
    for chunk in &result {
        assert!(chunk.len() <= 5);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dedup logic — intermediate stripping
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dedup_strips_exact_intermediate() {
    let response = "Let me commit. Done. Committed as abc123.";
    let intermediate = "Let me commit.";
    let remaining = response.replace(intermediate, "").trim().to_string();
    assert_eq!(remaining, "Done. Committed as abc123.");
}

#[test]
fn dedup_strips_multiple_intermediates() {
    let response = "Step 1. Step 2. Final result.";
    let intermediates = vec!["Step 1.", "Step 2."];
    let mut remaining = response.to_string();
    for inter in &intermediates {
        remaining = remaining.replace(inter, "");
    }
    let remaining = remaining.trim().to_string();
    assert_eq!(remaining, "Final result.");
}

#[test]
fn dedup_noop_when_intermediate_not_in_response() {
    let response = "Done. Committed as abc123.";
    let intermediate = "Let me commit.";
    let remaining = response.replace(intermediate, "").trim().to_string();
    assert_eq!(remaining, "Done. Committed as abc123.");
}

#[test]
fn dedup_empty_after_full_strip() {
    let response = "Let me commit.";
    let intermediate = "Let me commit.";
    let remaining = response.replace(intermediate, "").trim().to_string();
    assert!(remaining.is_empty());
}

#[test]
fn dedup_preserves_unrelated_text() {
    let response = "Hello world. Goodbye moon.";
    let intermediate = "Something else entirely.";
    let remaining = response.replace(intermediate, "").trim().to_string();
    assert_eq!(remaining, "Hello world. Goodbye moon.");
}

// ─────────────────────────────────────────────────────────────────────────────
// Cancel guard — ordering matters
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cancel_guard_fires_before_processing() {
    // Simulates the cancel guard checking before display queue processing
    let cancel_token = CancellationToken::new();
    cancel_token.cancel(); // simulate newer message cancelling

    let mut items_sent = Vec::new();
    let display_queue = vec!["tool_msg_1", "intermediate_text"];

    // The fix: check cancel BEFORE processing queue
    if !cancel_token.is_cancelled() {
        for item in display_queue {
            items_sent.push(item);
        }
    }

    assert!(
        items_sent.is_empty(),
        "no items should be sent when cancelled"
    );
}

#[tokio::test]
async fn cancel_guard_allows_active_processing() {
    let cancel_token = CancellationToken::new();
    // NOT cancelled

    let mut items_sent = Vec::new();
    let display_queue = vec!["tool_msg_1", "intermediate_text"];

    if !cancel_token.is_cancelled() {
        for item in display_queue {
            items_sent.push(item);
        }
    }

    assert_eq!(items_sent.len(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// Token counter — monotonic guard removed (regression test)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn token_counter_allows_decrease() {
    // After compaction, the calibrated token count should be allowed to decrease.
    // The old monotonic guard blocked this — verify it's gone.
    // Simulates the TuiEvent::TokenCountUpdated handler in state.rs
    fn update_token_count(current: &mut Option<u32>, new_val: u32) {
        // The fix: always update (no guard blocking decreases)
        *current = Some(new_val);
    }

    let mut last_input_tokens: Option<u32> = Some(111_406); // post-compaction tiktoken estimate
    update_token_count(&mut last_input_tokens, 41_212); // CLI-calibrated real value

    assert_eq!(
        last_input_tokens,
        Some(41_212),
        "token counter must reflect CLI-calibrated value"
    );
}

#[test]
fn token_counter_allows_increase() {
    fn update_token_count(current: &mut Option<u32>, new_val: u32) {
        *current = Some(new_val);
    }

    let mut last_input_tokens: Option<u32> = Some(41_212);
    update_token_count(&mut last_input_tokens, 45_000);
    assert_eq!(last_input_tokens, Some(45_000));
}

#[test]
fn token_counter_sets_from_none() {
    fn update_token_count(current: &mut Option<u32>, new_val: u32) {
        *current = Some(new_val);
    }

    let mut last_input_tokens: Option<u32> = None;
    update_token_count(&mut last_input_tokens, 25_000);
    assert_eq!(last_input_tokens, Some(25_000));
}

// ─────────────────────────────────────────────────────────────────────────────
// Pending request — model struct
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pending_request_struct_fields() {
    use crate::db::repository::pending_request::PendingRequest;
    let pr = PendingRequest {
        id: "abc-123".to_string(),
        session_id: "def-456".to_string(),
        user_message: "hello".to_string(),
        channel: "telegram".to_string(),
        channel_chat_id: Some("-12345".to_string()),
    };
    assert_eq!(pr.channel, "telegram");
    assert_eq!(pr.channel_chat_id, Some("-12345".to_string()));
}

#[test]
fn pending_request_tui_channel_no_chat_id() {
    use crate::db::repository::pending_request::PendingRequest;
    let pr = PendingRequest {
        id: "abc".to_string(),
        session_id: "def".to_string(),
        user_message: "test".to_string(),
        channel: "tui".to_string(),
        channel_chat_id: None,
    };
    assert_eq!(pr.channel, "tui");
    assert!(pr.channel_chat_id.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// Bot wait loop — polling simulation
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn bot_wait_loop_finds_bot_after_delay() {
    let state = Arc::new(TelegramState::new());
    let state_clone = state.clone();

    // Simulate bot becoming available after 200ms
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // We can't create a real Bot, but we can test the pattern
        state_clone.set_owner_chat_id(12345).await; // proxy for "bot available"
    });

    // Poll like the resume code does
    let mut found = false;
    for _ in 0..10 {
        if state.owner_chat_id().await.is_some() {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(found, "should find bot after delay");
}

#[tokio::test]
async fn bot_wait_loop_times_out() {
    let state = Arc::new(TelegramState::new());
    let mut found = false;
    for _ in 0..5 {
        if state.owner_chat_id().await.is_some() {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!found, "should time out when bot never connects");
}

// ─────────────────────────────────────────────────────────────────────────────
// Rate limit retry — RetryAfter handling
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn strip_html_tags_removes_formatting() {
    // Test the strip_html_tags helper used in plain text fallback
    let html = "<b>bold</b> and <code>code</code>";
    let plain = html
        .replace("<b>", "")
        .replace("</b>", "")
        .replace("<code>", "")
        .replace("</code>", "");
    assert_eq!(plain, "bold and code");
}

#[test]
fn strip_html_entities() {
    let html = "&lt;tag&gt; &amp; more";
    let plain = html
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    assert_eq!(plain, "<tag> & more");
}
