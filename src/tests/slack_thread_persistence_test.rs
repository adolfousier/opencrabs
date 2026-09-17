//! Slack thread persistence (#1620).
//!
//! `ChannelMessage::new()` hardcodes `thread_id: None`; only `.with_thread()`
//! sets it. Slack never called it, so every stored row landed with a NULL
//! thread_id. That is invisible until someone scopes the history fetch to a
//! thread: SQL NULL never equals a value, so a scoped lookup over NULL rows
//! returns nothing at all and the channel silently loses its history.
//!
//! These tests pin the storage contract at the data layer, where it can be
//! exercised for real, plus the three handler call sites at source level.

use crate::db::Database;
use crate::db::models::ChannelMessage;
use crate::db::repository::channel_message::ChannelMessageRepository;

fn row(sender: &str, content: &str, thread: Option<&str>) -> ChannelMessage {
    ChannelMessage::new(
        "slack".into(),
        "C123".into(),
        Some("general".into()),
        "U1".into(),
        sender.into(),
        content.into(),
        "text".into(),
        None,
    )
    .with_thread(thread.map(|t| t.to_string()), None)
}

#[tokio::test]
async fn thread_scoped_fetch_sees_only_its_own_thread() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    repo.insert(&row("Adi", "top level", None)).await.unwrap();
    repo.insert(&row("Adi", "in thread A", Some("111.1")))
        .await
        .unwrap();
    repo.insert(&row("Carlos", "in thread B", Some("222.2")))
        .await
        .unwrap();

    let thread_a = repo
        .recent(Some("slack"), "C123", 30, Some("111.1"), None)
        .await
        .unwrap();
    assert_eq!(thread_a.len(), 1, "thread A must not see B or top level");
    assert_eq!(thread_a[0].content, "in thread A");

    let channel_wide = repo
        .recent(Some("slack"), "C123", 30, None, None)
        .await
        .unwrap();
    assert_eq!(
        channel_wide.len(),
        3,
        "a top-level turn passes None and keeps the channel-wide view"
    );
}

#[tokio::test]
async fn rows_stored_without_a_thread_id_are_invisible_to_a_scoped_fetch() {
    // This is why persistence had to land before the fetch was scoped, and it
    // is the transitional cost: rows written before this fix keep NULL, so a
    // thread only regains history once new messages accumulate in it.
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    repo.insert(&row("Adi", "legacy row, no thread_id", None))
        .await
        .unwrap();

    let scoped = repo
        .recent(Some("slack"), "C123", 30, Some("111.1"), None)
        .await
        .unwrap();
    assert!(
        scoped.is_empty(),
        "NULL thread_id never matches a scoped lookup — scoping before \
         persisting would have emptied every threaded conversation"
    );
}

#[test]
fn every_slack_write_site_records_the_thread() {
    const HANDLER: &str = include_str!("../channels/slack/handler.rs");
    let writes = HANDLER.matches("DbChannelMessage::new(").count();
    let threaded = HANDLER.matches(".with_thread(").count();
    assert_eq!(
        writes, threaded,
        "every Slack channel_messages write must record its thread: \
         {writes} writes but {threaded} carry .with_thread()"
    );
}
