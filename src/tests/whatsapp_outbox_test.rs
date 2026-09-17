//! Outbox bookkeeping (#1408): the message-id capture that edit-in-place
//! streaming, self-reactions and pin/forward all depend on.

use std::time::{Duration, SystemTime};
use uuid::Uuid;

use crate::channels::whatsapp::outbox::{EDIT_WINDOW, Outbox, OutboxEntry};

fn entry(body: &str) -> OutboxEntry {
    OutboxEntry::new("3EB0ABCDEF", body)
}

#[tokio::test]
async fn record_then_get_returns_the_entry() {
    let outbox = Outbox::default();
    let session = Uuid::new_v4();
    outbox.record(session, entry("first chunk")).await;

    let got = outbox.get(session).await.expect("entry recorded");
    assert_eq!(got.message_id, "3EB0ABCDEF");
    assert_eq!(got.body, "first chunk");
}

#[tokio::test]
async fn get_on_unknown_session_is_none() {
    let outbox = Outbox::default();
    assert!(outbox.get(Uuid::new_v4()).await.is_none());
}

#[tokio::test]
async fn record_replaces_the_previous_entry_for_a_session() {
    let outbox = Outbox::default();
    let session = Uuid::new_v4();
    outbox.record(session, entry("old")).await;
    outbox
        .record(session, OutboxEntry::new("SECOND", "new"))
        .await;

    let got = outbox.get(session).await.expect("entry recorded");
    assert_eq!(got.message_id, "SECOND");
    assert_eq!(outbox.len().await, 1, "replacement must not grow the map");
}

#[tokio::test]
async fn sessions_do_not_share_entries() {
    let outbox = Outbox::default();
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    outbox.record(a, OutboxEntry::new("A", "body a")).await;
    outbox.record(b, OutboxEntry::new("B", "body b")).await;

    assert_eq!(outbox.get(a).await.unwrap().message_id, "A");
    assert_eq!(outbox.get(b).await.unwrap().message_id, "B");
    assert_eq!(outbox.len().await, 2);
}

#[tokio::test]
async fn set_body_updates_in_place_and_keeps_the_id() {
    let outbox = Outbox::default();
    let session = Uuid::new_v4();
    outbox.record(session, entry("chunk one")).await;

    assert!(outbox.set_body(session, "chunk one\nchunk two").await);
    let got = outbox.get(session).await.unwrap();
    assert_eq!(got.body, "chunk one\nchunk two");
    assert_eq!(
        got.message_id, "3EB0ABCDEF",
        "an edit must target the same message"
    );
}

#[tokio::test]
async fn set_body_on_a_cleared_session_reports_failure() {
    let outbox = Outbox::default();
    let session = Uuid::new_v4();
    assert!(
        !outbox.set_body(session, "anything").await,
        "caller must be told to fall back to a fresh send"
    );
}

#[tokio::test]
async fn clear_removes_the_entry_and_returns_it() {
    let outbox = Outbox::default();
    let session = Uuid::new_v4();
    outbox.record(session, entry("body")).await;

    let taken = outbox.clear(session).await.expect("entry returned");
    assert_eq!(taken.message_id, "3EB0ABCDEF");
    assert!(outbox.get(session).await.is_none());
    assert_eq!(outbox.len().await, 0);
}

#[test]
fn a_fresh_entry_is_editable() {
    assert!(entry("body").editable());
}

#[test]
fn an_entry_past_the_window_is_not_editable() {
    let mut stale = entry("body");
    stale.sent_at = SystemTime::now() - EDIT_WINDOW - Duration::from_secs(1);
    assert!(
        !stale.editable(),
        "WhatsApp rejects edits older than 15 minutes; do not spend the round trip"
    );
}

#[test]
fn an_entry_just_inside_the_window_is_still_editable() {
    let mut nearly = entry("body");
    nearly.sent_at = SystemTime::now() - EDIT_WINDOW + Duration::from_secs(30);
    assert!(nearly.editable());
}

#[test]
fn a_backwards_clock_leaves_the_entry_editable() {
    // `SystemTime::elapsed` errors when the clock jumped back. Guessing
    // "expired" there would silently disable editing for the rest of the
    // turn; let the server decide instead.
    let mut future = entry("body");
    future.sent_at = SystemTime::now() + Duration::from_secs(60);
    assert!(future.editable());
}

// ── Turn boundaries (#1614) ─────────────────────────────────────────────────
//
// The entry outlives the turn that created it on purpose: the final edit and
// the completion reaction both run after the agent call returns. What was
// missing is the release at the START of the next turn, which is why a
// follow-up sent inside the 15-minute window edited the answer already on
// screen instead of posting a new message.

use crate::channels::whatsapp::WhatsAppState;
use crate::channels::whatsapp::stream::{CHUNK_LIMIT, Delivery, plan};

#[tokio::test]
async fn a_new_turn_releases_the_previous_turns_message() {
    let state = WhatsAppState::new();
    let session = Uuid::new_v4();
    state
        .record_outbound(session, OutboxEntry::new("TURN1", "turn one's answer"))
        .await;
    assert!(
        state.editable_outbound(session).await.is_some(),
        "the finished turn still owns its message until the next turn starts"
    );

    state.begin_turn(session).await;

    assert!(
        state.editable_outbound(session).await.is_none(),
        "turn two must not inherit turn one's message as an edit target"
    );
}

#[tokio::test]
async fn begin_turn_on_an_untracked_session_is_a_noop() {
    let state = WhatsAppState::new();
    let session = Uuid::new_v4();
    state.begin_turn(session).await;
    assert!(state.editable_outbound(session).await.is_none());
}

#[tokio::test]
async fn begin_turn_only_releases_its_own_session() {
    let state = WhatsAppState::new();
    let (mine, other) = (Uuid::new_v4(), Uuid::new_v4());
    state
        .record_outbound(mine, OutboxEntry::new("MINE", "mine"))
        .await;
    state
        .record_outbound(other, OutboxEntry::new("OTHER", "other"))
        .await;

    state.begin_turn(mine).await;

    assert!(state.editable_outbound(mine).await.is_none());
    assert_eq!(
        state
            .editable_outbound(other)
            .await
            .expect("another chat's turn is untouched")
            .message_id,
        "OTHER"
    );
}

#[tokio::test]
async fn first_chunk_of_a_follow_up_turn_sends_instead_of_editing() {
    // The reported #1614 failure in full: turn one answers, turn two arrives
    // well inside the edit window, and its first streamed chunk must open a
    // new bubble rather than rewrite what the user already read.
    let state = WhatsAppState::new();
    let session = Uuid::new_v4();
    state
        .record_outbound(session, OutboxEntry::new("TURN1", "turn one's answer"))
        .await;

    state.begin_turn(session).await;

    let tracked = state.editable_outbound(session).await;
    assert_eq!(
        plan(
            tracked.as_ref().map(|e| e.body.as_str()),
            "thinking about turn two",
            CHUNK_LIMIT
        ),
        Delivery::Send("thinking about turn two".to_string()),
        "without the release this was Edit(\"turn one's answer\\n\\nthinking about turn two\")"
    );
}

#[test]
fn the_handler_releases_the_outbox_before_the_stream_task_starts() {
    // `handle_message` needs a live client, so the ordering that fixes #1614
    // cannot be exercised in a unit test. Pin it in the source instead: the
    // release must happen before the turn's first chunk can be delivered.
    let handler = std::fs::read_to_string("src/channels/whatsapp/handler.rs")
        .expect("handler.rs is readable from the crate root");
    let release = handler
        .find("begin_turn(session_id)")
        .expect("handle_message must release the previous turn's outbox entry (#1614)");
    let spawn = handler
        .find("stream::spawn")
        .expect("the streaming consumer is still spawned in handler.rs");
    assert!(
        release < spawn,
        "the outbox release must run before the stream task, or the first chunk \
         still edits the previous turn's message"
    );
}
