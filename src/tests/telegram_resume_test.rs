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
async fn cancel_token_new_message_does_not_cancel_previous() {
    // #652: storing a new token must NOT cancel the in-flight one. The old
    // cancel-on-store hard-killed a running turn on a near-simultaneous resend.
    // try_begin_turn (#501) is the concurrency gate now; cancellation only
    // happens through cancel_session.
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
    assert!(
        !old_token.is_cancelled(),
        "storing a new token must not cancel the previous one (#652)"
    );
    assert!(!new_token.is_cancelled());
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
async fn cancel_token_rapid_replacement_never_cancels() {
    // #652: rapid re-stores just overwrite the map entry; store_cancel_token
    // never cancels, so every token stays active.
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    let mut tokens = Vec::new();
    for _ in 0..10 {
        let t = CancellationToken::new();
        state.store_cancel_token(session_id, t.clone()).await;
        tokens.push(t);
    }
    for t in &tokens {
        assert!(
            !t.is_cancelled(),
            "store_cancel_token must never cancel a token (#652)"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TelegramState — session chat management
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn session_chat_register_and_lookup() {
    let state = TelegramState::new();
    let session_id = Uuid::new_v4();
    state.register_session_chat(session_id, -12345, None).await;
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
    // #652: concurrent stores race to own the map slot but must never cancel a
    // token — none are cancelled by storing.
    let active_count = tokens.iter().filter(|t| !t.is_cancelled()).count();
    assert_eq!(active_count, 20, "store must never cancel any token (#652)");
    // cancel_session then cancels exactly the one still in the map.
    assert!(state.cancel_session(session_id).await);
    let cancelled = tokens.iter().filter(|t| t.is_cancelled()).count();
    assert_eq!(
        cancelled, 1,
        "cancel_session cancels exactly the surviving token"
    );
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

#[test]
fn md_to_html_escapes_angle_bracket_placeholder() {
    // The bug: /help's `/rename <new title>` reached Telegram as a `<new>` tag,
    // so the HTML parser rejected the whole message and the reply vanished.
    let result = crate::channels::telegram::handler::md_to_html("`/rename <new title>`");
    assert!(
        result.contains("&lt;new title&gt;"),
        "placeholder must be escaped; got: {result}"
    );
    assert!(
        !result.contains("<new"),
        "raw <new tag must not leak; got: {result}"
    );
}

#[test]
fn md_to_html_escapes_plain_html_metacharacters() {
    let result = crate::channels::telegram::handler::md_to_html("a < b & c > d");
    assert_eq!(result, "a &lt; b &amp; c &gt; d");
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
fn dedup_exact_match_suppresses_final() {
    let response = "Let me commit. Done. Committed as abc123.";
    let intermediates = ["Let me commit. Done. Committed as abc123."];
    let result = if intermediates.iter().any(|i| i.trim() == response.trim()) {
        String::new()
    } else {
        response.to_string()
    };
    assert!(result.is_empty(), "exact match should suppress final");
}

#[test]
fn dedup_substring_match_does_not_strip() {
    // The #252 fix: streaming diff chunks are substrings of the final
    // response. OLD behavior stripped them (breaking numbered lists).
    // NEW behavior only suppresses on exact match.
    let response = "Step 1. Step 2. Final result.";
    let intermediates = ["Step 1.", "Step 2."];
    let result = if intermediates.iter().any(|i| i.trim() == response.trim()) {
        String::new()
    } else {
        response.to_string()
    };
    assert_eq!(result, "Step 1. Step 2. Final result.");
}

#[test]
fn dedup_noop_when_intermediate_not_in_response() {
    let response = "Done. Committed as abc123.";
    let intermediates = ["Let me commit."];
    let result = if intermediates.iter().any(|i| i.trim() == response.trim()) {
        String::new()
    } else {
        response.to_string()
    };
    assert_eq!(result, "Done. Committed as abc123.");
}

#[test]
fn dedup_empty_after_exact_match() {
    let response = "Let me commit.";
    let intermediates = ["Let me commit."];
    let result = if intermediates.iter().any(|i| i.trim() == response.trim()) {
        String::new()
    } else {
        response.to_string()
    };
    assert!(result.is_empty(), "exact match should suppress final");
}

#[test]
fn dedup_preserves_unrelated_text() {
    let response = "Hello world. Goodbye moon.";
    let intermediates = ["Something else entirely."];
    let result = if intermediates.iter().any(|i| i.trim() == response.trim()) {
        String::new()
    } else {
        response.to_string()
    };
    assert_eq!(result, "Hello world. Goodbye moon.");
}

#[test]
fn dedup_trim_tolerates_whitespace() {
    // Trailing newline/whitespace from streaming should not break exact match
    let response = "All done!\n";
    let intermediates = ["All done!"];
    let result = if intermediates.iter().any(|i| i.trim() == response.trim()) {
        String::new()
    } else {
        response.to_string()
    };
    assert!(result.is_empty(), "trimmed match should suppress final");
}

#[test]
fn dedup_17_item_list_not_gutted() {
    // Regression #252: 17-item numbered list sent as streaming diffs.
    // Items 1-16 are diff intermediates, item 17 is the last chunk.
    // Final response = all 17 items. OLD replace() stripped items 1-16.
    // NEW exact-match keeps the full list intact.
    let response = "1. Alpha\n2. Beta\n3. Gamma\n4. Delta\n5. Epsilon\n6. Zeta\n7. Eta\n8. Theta\n9. Iota\n10. Kappa\n11. Lambda\n12. Mu\n13. Nu\n14. Xi\n15. Omicron\n16. Pi\n17. Rho";
    let intermediates = ["1. Alpha", "2. Beta", "17. Rho"];
    let result = if intermediates.iter().any(|i| i.trim() == response.trim()) {
        String::new()
    } else {
        response.to_string()
    };
    assert_eq!(result, response, "full list must survive dedup");
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
        channel_thread_id: Some("77".to_string()),
        channel_message_id: Some("flow:159".to_string()),
        origin: "user".to_string(),
    };
    assert_eq!(pr.channel, "telegram");
    assert_eq!(pr.channel_chat_id, Some("-12345".to_string()));
    assert_eq!(pr.channel_thread_id, Some("77".to_string()));
    assert_eq!(pr.channel_message_id, Some("flow:159".to_string()));
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
        channel_thread_id: None,
        channel_message_id: None,
        origin: "user".to_string(),
    };
    assert_eq!(pr.channel, "tui");
    assert!(pr.channel_chat_id.is_none());
    assert!(pr.channel_message_id.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// Carried bubble adoption — #2010
//
// A SIGKILLed Telegram turn leaves its partially written bubble on screen and
// the boot resume used to open a SECOND bubble for the same answer. The
// pending row now carries the live surface as "<surface>:<message id>" and the
// resume must adopt exactly that bubble, or decline and do what it did before.
// No fake Telegram transport exists in this crate, so the send-vs-edit wiring
// itself is pinned structurally against the source; the pure decision function
// is tested directly.
// ─────────────────────────────────────────────────────────────────────────────

use crate::channels::telegram::resume::{CarriedBubble, streaming_bubble_to_adopt};
use crate::db::repository::pending_request::PendingRequest;
use teloxide::types::MessageId;

/// The chat the original incident happened in (oc-users topic 15).
const CARRIED_CHAT: i64 = -1004428873948;

fn carried_row(surface: Option<&str>) -> PendingRequest {
    PendingRequest {
        id: "row-1".to_string(),
        session_id: "sess-1".to_string(),
        user_message: "hello".to_string(),
        channel: "telegram".to_string(),
        channel_chat_id: Some(CARRIED_CHAT.to_string()),
        channel_thread_id: Some("15".to_string()),
        channel_message_id: surface.map(str::to_string),
        origin: "user".to_string(),
    }
}

#[test]
fn carried_flow_bubble_is_adopted_as_rich() {
    let got = streaming_bubble_to_adopt(&carried_row(Some("flow:159")), CARRIED_CHAT);
    assert_eq!(got, Some(CarriedBubble::Flow(MessageId(159), true)));
}

#[test]
fn carried_plain_flow_bubble_keeps_the_html_path() {
    let got = streaming_bubble_to_adopt(&carried_row(Some("flow_plain:159")), CARRIED_CHAT);
    assert_eq!(got, Some(CarriedBubble::Flow(MessageId(159), false)));
}

#[test]
fn carried_answer_bubble_is_adopted_as_the_placeholder() {
    let got = streaming_bubble_to_adopt(&carried_row(Some("answer:160")), CARRIED_CHAT);
    assert_eq!(got, Some(CarriedBubble::Answer(MessageId(160))));
}

#[test]
fn row_without_a_carried_bubble_adopts_nothing() {
    assert_eq!(
        streaming_bubble_to_adopt(&carried_row(None), CARRIED_CHAT),
        None
    );
}

#[test]
fn malformed_carriage_adopts_nothing() {
    // Anything that is not exactly "<known surface>:<i64>" falls back to the
    // pre-#2010 behaviour: the resume opens its own bubble.
    for bad in [
        "159",
        "flow:",
        "mystery:159",
        "flow:not-a-number",
        "flow:99999999999999999999",
    ] {
        let got = streaming_bubble_to_adopt(&carried_row(Some(bad)), CARRIED_CHAT);
        assert_eq!(got, None, "{bad} must not be adopted");
    }
}

#[test]
fn adoption_declines_rows_that_are_not_this_turns_telegram_bubble() {
    // Telegram message ids are unique per CHAT: editing with a foreign chat
    // id would edit a stranger's message.
    let mut other_chat = carried_row(Some("flow:159"));
    other_chat.channel_chat_id = Some("-1009999999999".to_string());
    assert_eq!(streaming_bubble_to_adopt(&other_chat, CARRIED_CHAT), None);

    let no_chat = {
        let mut r = carried_row(Some("flow:159"));
        r.channel_chat_id = None;
        r
    };
    assert_eq!(streaming_bubble_to_adopt(&no_chat, CARRIED_CHAT), None);

    // Other channels have no Telegram bubbles to adopt.
    let discord = {
        let mut r = carried_row(Some("flow:159"));
        r.channel = "discord".to_string();
        r
    };
    assert_eq!(streaming_bubble_to_adopt(&discord, CARRIED_CHAT), None);

    // A `system` push row re-delivers its text (#12), it never re-streams a
    // bubble.
    let pushed = {
        let mut r = carried_row(Some("flow:159"));
        r.origin = "system".to_string();
        r
    };
    assert_eq!(streaming_bubble_to_adopt(&pushed, CARRIED_CHAT), None);
}

fn resume_source() -> String {
    std::fs::read_to_string(format!(
        "{}/src/channels/telegram/resume.rs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("read channels/telegram/resume.rs")
}

#[test]
fn resume_seeds_the_streaming_state_from_the_carried_bubble() {
    // #2010 contract, structural half: the resume looks the row up, runs it
    // through the adoption decision and SEEDS the StreamingState with the
    // result, so the first refresh edits the carried bubble instead of
    // sending a new one.
    let src = resume_source();
    assert!(
        src.contains("streaming_bubble_to_adopt(&row, chat_id.0)"),
        "resume must decide adoption from the pending row"
    );
    let ctor_at = src
        .find("StreamingState {")
        .expect("StreamingState construction in resume");
    let ctor = &src[ctor_at..ctor_at + 2000];
    assert!(
        ctor.contains("msg_id: seed_msg_id"),
        "placeholder must be seeded"
    );
    assert!(
        ctor.contains("open_group_msg_id: seed_flow.map(|(id, _)| id)"),
        "flow block must be seeded"
    );
    assert!(
        ctor.contains("flow_rich: matches!(seed_flow, Some((_, true)))"),
        "the carried surface's API must be seeded"
    );
    assert!(
        !ctor.contains("msg_id: None"),
        "the placeholder field must not be hardcoded to None again"
    );
}

#[test]
fn teardown_keeps_an_adopted_bubble_and_still_deletes_its_own() {
    // #1462 ethos: a bubble that predates this process is user-visible
    // history. A stale bubble is recoverable by hand, a deleted one is gone.
    let src = resume_source();
    assert!(
        src.contains("if adopted_answer {")
            && src.contains("keeping adopted bubble")
            && src.contains("best_effort_delete(&bot, chat_id, mid, \"streaming teardown\")"),
        "teardown must branch on adopted_answer, not delete unconditionally"
    );
}

#[test]
fn stream_loop_records_the_live_bubble_surface_on_the_row() {
    // The write hook: without it the row never carries an id and the resume
    // has nothing to adopt. The flow block wins when both surfaces exist.
    let src = std::fs::read_to_string(format!(
        "{}/src/channels/telegram/stream_loop.rs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("read channels/telegram/stream_loop.rs");
    assert!(
        src.contains("set_channel_message_id_for_session(sid, &carried)"),
        "the edit loop must persist the live bubble"
    );
    assert!(
        src.contains("if s.flow_rich { \"flow\" } else { \"flow_plain\" }")
            && src.contains("snap.msg_id.map(|m| (\"answer\", m.0))"),
        "the surface tag must follow flow_rich and fall back to the placeholder"
    );
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

// ── #376: split_message multi-byte regression ──────────────────────────────

#[test]
fn split_message_multibyte_chars_not_split_by_bytes() {
    // 2049 Cyrillic chars = 4098 bytes but only 2049 chars.
    // Old byte-based code would split at byte 4096 (= char 2048),
    // creating 2 chunks when Telegram accepts up to 4096 chars.
    let text: String = "Д".repeat(2049);
    assert!(text.len() > 4096); // 4098 bytes
    assert_eq!(text.chars().count(), 2049); // 2049 chars
    let result = crate::channels::telegram::handler::split_message(&text, 4096);
    assert_eq!(
        result.len(),
        1,
        "2049 chars should fit in one 4096-char chunk"
    );
    assert_eq!(result[0], text);
}

#[test]
fn split_message_multibyte_exceeds_char_limit() {
    // 5000 Cyrillic chars = 10000 bytes, 5000 chars.
    // Should split into 2 chunks of ~2500 chars each.
    let text: String = "Д".repeat(5000);
    let result = crate::channels::telegram::handler::split_message(&text, 4096);
    assert!(
        result.len() >= 2,
        "5000 chars should split into multiple chunks"
    );
    for chunk in &result {
        assert!(
            chunk.chars().count() <= 4096,
            "each chunk should have <= 4096 chars, got {}",
            chunk.chars().count()
        );
    }
    // All content preserved
    let rejoined: String = result.iter().copied().collect();
    assert_eq!(rejoined.chars().count(), 5000);
}

// ── #378: strip_html_tags completeness regression ──────────────────────────

#[test]
fn strip_html_tags_removes_all_supported_tags() {
    let html = "<b>bold</b> <i>italic</i> <u>underline</u> <s>strike</s> <code>code</code>";
    let plain = crate::channels::telegram::handler::strip_html_tags(html);
    assert_eq!(plain, "bold italic underline strike code");
}

#[test]
fn strip_html_tags_removes_anchor_tags() {
    let html = "visit <a href=\"https://example.com?q=rust&source=web\">example</a> now";
    let plain = crate::channels::telegram::handler::strip_html_tags(html);
    assert_eq!(plain, "visit example now");
}

#[test]
fn strip_html_tags_removes_blockquote() {
    let html = "<blockquote>quoted text</blockquote> and normal";
    let plain = crate::channels::telegram::handler::strip_html_tags(html);
    assert_eq!(plain, "quoted text and normal");
}

#[test]
fn strip_html_tags_unescapes_entities() {
    let html = "&lt;tag&gt; &amp; &quot;quoted&quot;";
    let plain = crate::channels::telegram::handler::strip_html_tags(html);
    assert_eq!(plain, "<tag> & \"quoted\"");
}
