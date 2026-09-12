//! Bounded memory of recently seen messages (#1484 forward).

use waproto::whatsapp::Message;

use crate::channels::whatsapp::recent::{DEFAULT_CAPACITY, RecentMessages};

fn text(body: &str) -> Message {
    Message {
        conversation: Some(body.to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_remembered_message_comes_back() {
    let recent = RecentMessages::default();
    recent.remember("3EB0AAA", text("hello")).await;

    let got = recent.get("3EB0AAA").await.expect("remembered");
    assert_eq!(got.conversation.as_deref(), Some("hello"));
}

#[tokio::test]
async fn an_unknown_id_yields_nothing() {
    let recent = RecentMessages::default();
    assert!(recent.get("3EB0MISSING").await.is_none());
}

#[tokio::test]
async fn an_empty_id_is_not_stored() {
    // An id-less message cannot be addressed for a forward, and storing it
    // under "" would let one such message shadow the next.
    let recent = RecentMessages::default();
    recent.remember("", text("ghost")).await;
    assert_eq!(recent.len().await, 0);
}

#[tokio::test]
async fn the_oldest_message_is_evicted_once_the_window_is_full() {
    let recent = RecentMessages::with_capacity(3);
    for i in 0..4 {
        recent
            .remember(format!("id-{i}"), text(&format!("body {i}")))
            .await;
    }
    assert_eq!(recent.len().await, 3);
    assert!(recent.get("id-0").await.is_none(), "oldest evicted");
    assert!(recent.get("id-3").await.is_some(), "newest kept");
}

#[tokio::test]
async fn re_remembering_refreshes_the_proto_and_the_position() {
    // A message still being referenced must not age out mid-conversation.
    let recent = RecentMessages::with_capacity(2);
    recent.remember("a", text("first")).await;
    recent.remember("b", text("second")).await;
    recent.remember("a", text("first, edited")).await;
    recent.remember("c", text("third")).await;

    assert_eq!(
        recent
            .get("a")
            .await
            .and_then(|m| m.conversation)
            .as_deref(),
        Some("first, edited"),
        "the refreshed proto wins"
    );
    assert!(recent.get("b").await.is_none(), "b became the oldest");
    assert!(recent.get("c").await.is_some());
    assert_eq!(recent.len().await, 2);
}

#[tokio::test]
async fn a_zero_capacity_still_holds_one_message() {
    // Otherwise every insert evicts itself and forward looks broken rather
    // than disabled.
    let recent = RecentMessages::with_capacity(0);
    recent.remember("only", text("kept")).await;
    assert!(recent.get("only").await.is_some());
}

#[tokio::test]
async fn the_default_window_spans_a_normal_conversation() {
    let recent = RecentMessages::default();
    for i in 0..DEFAULT_CAPACITY {
        recent.remember(format!("id-{i}"), text("x")).await;
    }
    assert_eq!(recent.len().await, DEFAULT_CAPACITY);
    assert!(recent.get("id-0").await.is_some(), "nothing evicted yet");
}
