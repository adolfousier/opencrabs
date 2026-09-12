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
