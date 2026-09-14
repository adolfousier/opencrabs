//! Tests for forum-topic capture + listing — issue #130 follow-up.
//!
//! The bot needs to translate user-typed topic names like
//! "#announcements" to numeric thread_ids it can pass to
//! `telegram_send`. Telegram's Bot API has no `listForumTopics`
//! endpoint, so we capture topic names passively from messages
//! we receive and expose them via `ChannelMessageRepository::topics_for_chat`.

use crate::db::models::ChannelMessage;
use crate::db::{ChannelMessageRepository, Database};

fn msg(
    chat_id: &str,
    msg_id: &str,
    thread_id: Option<&str>,
    topic_name: Option<&str>,
) -> ChannelMessage {
    let mut row = ChannelMessage::new(
        "telegram".into(),
        chat_id.into(),
        Some("Test Group".into()),
        "u1".into(),
        "Alice".into(),
        "hello".into(),
        "text".into(),
        Some(msg_id.into()),
    )
    .with_thread(
        thread_id.map(str::to_string),
        topic_name.map(str::to_string),
    );
    // e* ids are rename rows — the handler persists message_type
    // "topic_edited" (forum_topic_edited), so the pin mirrors production.
    if msg_id.starts_with('e') {
        row.message_type = "topic_edited".into();
    }
    row
}

#[tokio::test]
async fn topics_for_chat_returns_distinct_threads_with_names() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    // Three messages: two in topic 17 ("announcements"), one in
    // topic 22 ("dev"). topics_for_chat should return two distinct
    // entries, each with the topic name captured from at least one
    // of the messages.
    repo.insert(&msg("test-chat", "m1", Some("17"), Some("announcements")))
        .await
        .unwrap();
    repo.insert(&msg("test-chat", "m2", Some("17"), Some("announcements")))
        .await
        .unwrap();
    repo.insert(&msg("test-chat", "m3", Some("22"), Some("dev")))
        .await
        .unwrap();

    let topics = repo.topics_for_chat("telegram", "test-chat").await.unwrap();
    assert_eq!(topics.len(), 2);

    let names: Vec<_> = topics
        .iter()
        .map(|t| (t.thread_id.as_str(), t.topic_name.as_deref()))
        .collect();
    assert!(
        names.contains(&("17", Some("announcements"))),
        "expected #17 announcements; got: {names:?}"
    );
    assert!(
        names.contains(&("22", Some("dev"))),
        "expected #22 dev; got: {names:?}"
    );
}

#[tokio::test]
async fn topics_for_chat_handles_topics_with_no_captured_name() {
    // The bot saw messages in a topic but never the topic-creation
    // service message (e.g. it was added to the group after the
    // topic was created and nobody has replied to the creation
    // message since). thread_id is still useful — it can be used
    // to send — but the agent won't be able to map a user-typed
    // name. The row must still appear in the listing.
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    repo.insert(&msg("test-chat", "m1", Some("33"), None))
        .await
        .unwrap();

    let topics = repo.topics_for_chat("telegram", "test-chat").await.unwrap();
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0].thread_id, "33");
    assert_eq!(topics[0].topic_name, None);
}

#[tokio::test]
async fn topics_for_chat_ignores_non_topic_messages() {
    // Regular (non-forum) chats have thread_id = NULL on every
    // message. The query filters them out so the topic listing
    // doesn't return a phantom "no topic" row that would confuse
    // the agent.
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    repo.insert(&msg("test-chat", "m1", None, None))
        .await
        .unwrap();
    repo.insert(&msg("test-chat", "m2", None, None))
        .await
        .unwrap();

    let topics = repo.topics_for_chat("telegram", "test-chat").await.unwrap();
    assert!(topics.is_empty(), "non-topic messages must not surface");
}

#[tokio::test]
async fn topics_for_chat_isolates_per_channel_and_chat() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    // Same thread_id "17" in two different chats — must NOT collapse.
    repo.insert(&msg("chat-A", "m1", Some("17"), Some("A-announcements")))
        .await
        .unwrap();
    repo.insert(&msg("chat-B", "m1", Some("17"), Some("B-announcements")))
        .await
        .unwrap();

    let a = repo.topics_for_chat("telegram", "chat-A").await.unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].topic_name.as_deref(), Some("A-announcements"));

    let b = repo.topics_for_chat("telegram", "chat-B").await.unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].topic_name.as_deref(), Some("B-announcements"));
}

#[tokio::test]
async fn topics_for_chat_orders_by_most_recent_activity() {
    use chrono::{Duration, Utc};

    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    // Older message in topic 100; newer message in topic 200. The
    // listing should put 200 first because the agent usually cares
    // about whatever the user touched most recently.
    let mut old = msg("chat", "m1", Some("100"), Some("old-topic"));
    old.created_at = Utc::now() - Duration::hours(2);
    let new = msg("chat", "m2", Some("200"), Some("new-topic"));

    repo.insert(&old).await.unwrap();
    repo.insert(&new).await.unwrap();

    let topics = repo.topics_for_chat("telegram", "chat").await.unwrap();
    assert_eq!(topics.len(), 2);
    assert_eq!(
        topics[0].thread_id, "200",
        "most-recent activity must come first"
    );
    assert_eq!(topics[1].thread_id, "100");
}

#[tokio::test]
async fn message_count_per_topic_is_correct() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    // 3 messages in topic 17, 1 in topic 22.
    for i in 1..=3 {
        repo.insert(&msg(
            "chat",
            &format!("a{i}"),
            Some("17"),
            Some("announcements"),
        ))
        .await
        .unwrap();
    }
    repo.insert(&msg("chat", "d1", Some("22"), Some("dev")))
        .await
        .unwrap();

    let topics = repo.topics_for_chat("telegram", "chat").await.unwrap();
    let by_id: std::collections::HashMap<_, _> = topics
        .iter()
        .map(|t| (t.thread_id.as_str(), t.message_count))
        .collect();
    assert_eq!(by_id.get("17"), Some(&3));
    assert_eq!(by_id.get("22"), Some(&1));
}

#[tokio::test]
async fn topics_for_chat_serves_renamed_topic_not_creation_name() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    // Topic 17 created as "announcements"; later renamed to "general".
    // Regular messages keep re-teaching the CREATION name via their
    // reply-target, so the newest NON-EDIT row still says the old name.
    // topics_for_chat must serve "general" — the rename is the freshest
    // user intent, not MAX() (alphabetical) nor the newest learned name.
    repo.insert(&msg("rc", "m1", Some("17"), Some("announcements")))
        .await
        .unwrap();
    repo.insert(&msg("rc", "m2", Some("17"), Some("announcements")))
        .await
        .unwrap();
    repo.insert(&msg("rc", "e1", Some("17"), Some("general")))
        .await
        .unwrap();
    // Post-rename regular message: reply-target re-teaches the old name.
    repo.insert(&msg("rc", "m3", Some("17"), Some("announcements")))
        .await
        .unwrap();

    let topics = repo.topics_for_chat("telegram", "rc").await.unwrap();
    assert_eq!(topics.len(), 1);
    assert_eq!(
        topics[0].topic_name.as_deref(),
        Some("general"),
        "rename must outrank the re-taught creation name"
    );
}

#[tokio::test]
async fn topics_for_chat_message_count_includes_all_rows_after_rename() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = ChannelMessageRepository::new(db.pool().clone());

    for i in 1..=2 {
        repo.insert(&msg("rc2", &format!("m{i}"), Some("9"), Some("old")))
            .await
            .unwrap();
    }
    repo.insert(&msg("rc2", "e1", Some("9"), Some("new")))
        .await
        .unwrap();

    let topics = repo.topics_for_chat("telegram", "rc2").await.unwrap();
    assert_eq!(topics.len(), 1);
    assert_eq!(
        topics[0].message_count, 3,
        "JOIN must not drop rows from the count"
    );
}

#[test]
fn telegram_send_tool_schema_describes_bot_observed_scope_for_list_topics() {
    use crate::brain::tools::telegram_send::TelegramSendTool;
    use crate::brain::tools::Tool;
    use std::sync::Arc;

    let tool = TelegramSendTool::new(Arc::default());
    let schema = tool.input_schema();
    let action_desc = schema["properties"]["action"]["description"]
        .as_str()
        .expect("action description must exist");

    assert!(
        action_desc.contains("ONLY the bot-observed"),
        "action description should make bot-observed constraint explicit"
    );
    assert!(
        action_desc.contains("does NOT enumerate the full forum surface"),
        "action description should make clear it does not enumerate full forum surface"
    );
    assert!(
        action_desc.contains("fast-mcp-telegram tg_get_chat_info"),
        "action description should guide users/agents to MTProto tools for exhaustive enumeration"
    );
}
