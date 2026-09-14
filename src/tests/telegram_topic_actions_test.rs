//! Unit tests for create_topic and rename_topic actions in telegram_send (#161).

use crate::brain::tools::Tool;
use crate::brain::tools::ToolExecutionContext;
use crate::brain::tools::telegram_send::TelegramSendTool;
use crate::channels::telegram::TelegramState;
use crate::db::ChannelMessageRepository;
use crate::db::Database;
use serde_json::json;
use std::sync::Arc;
use teloxide::Bot;
use uuid::Uuid;

fn make_tool() -> TelegramSendTool {
    let state = Arc::new(TelegramState::new());
    TelegramSendTool::new(state)
}

/// `execute()` refuses a botless state before any action dispatch ("Telegram
/// is not connected"), so tests that drive the full tool must install a dummy
/// bot first. The name-bounds validations return before any request is built —
/// the bot is never used on the wire.
async fn make_connected_tool() -> TelegramSendTool {
    let state = Arc::new(TelegramState::new());
    state.set_bot(Bot::new("TESTTOKEN")).await;
    TelegramSendTool::new(state)
}

#[test]
fn telegram_send_schema_declares_topic_actions_and_params() {
    let tool = make_tool();
    let schema = tool.input_schema();
    let props = schema.get("properties").expect("schema has properties");

    let action = props.get("action").expect("schema has action");
    let enums = action
        .get("enum")
        .and_then(|v| v.as_array())
        .expect("action has enum");
    let enum_strs: Vec<&str> = enums.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        enum_strs.contains(&"create_topic"),
        "action enum missing create_topic"
    );
    assert!(
        enum_strs.contains(&"rename_topic"),
        "action enum missing rename_topic"
    );
    assert_eq!(
        enum_strs.len(),
        22,
        "action enum should contain exactly 22 actions"
    );

    let name = props.get("name").expect("schema has name property");
    assert_eq!(name.get("type").and_then(|v| v.as_str()), Some("string"));
}

#[tokio::test]
async fn telegram_send_rejects_empty_or_too_long_topic_name() {
    let tool = make_connected_tool().await;
    let ctx = ToolExecutionContext::new(Uuid::new_v4());

    // Empty name
    let res = tool
        .execute(
            json!({
                "action": "create_topic",
                "chat_id": -1001234567890i64,
                "name": "   "
            }),
            &ctx,
        )
        .await
        .expect("tool execution returns Ok(ToolResult)");
    assert!(!res.success, "empty topic name should fail validation");
    assert!(
        res.error
            .unwrap_or_default()
            .contains("between 1 and 128 characters")
    );

    // Name exceeding 128 chars
    let long_name = "a".repeat(129);
    let res2 = tool
        .execute(
            json!({
                "action": "create_topic",
                "chat_id": -1001234567890i64,
                "name": long_name
            }),
            &ctx,
        )
        .await
        .expect("tool execution returns Ok(ToolResult)");
    assert!(!res2.success, "overlong topic name should fail validation");
    assert!(
        res2.error
            .unwrap_or_default()
            .contains("between 1 and 128 characters")
    );

    // Rename without thread_id
    let res3 = tool
        .execute(
            json!({
                "action": "rename_topic",
                "chat_id": -1001234567890i64,
                "name": "Valid Name"
            }),
            &ctx,
        )
        .await
        .expect("tool execution returns Ok(ToolResult)");
    assert!(!res3.success, "rename without thread_id should fail");
    assert!(res3.error.unwrap_or_default().contains("thread_id"));
}

#[tokio::test]
async fn record_topic_created_and_edited_persists_to_repository() {
    let db = Database::connect_in_memory().await.expect("in-memory db");
    db.run_migrations().await.expect("migrations");
    let pool = db.pool().clone();

    // Simulate record_topic_created for a new topic
    crate::channels::telegram::record_topic_created(
        Some(pool.clone()),
        -1001234567890,
        42,
        "General Discussion",
        false,
    )
    .await;

    let repo = ChannelMessageRepository::new(pool.clone());
    let topics = repo
        .topics_for_chat("telegram", "-1001234567890")
        .await
        .expect("topics query");
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0].thread_id, "42");
    assert_eq!(topics[0].topic_name.as_deref(), Some("General Discussion"));

    // Simulate record_topic_created for a renamed topic
    crate::channels::telegram::record_topic_created(
        Some(pool.clone()),
        -1001234567890,
        42,
        "Announcements",
        true,
    )
    .await;

    let topics_after = repo
        .topics_for_chat("telegram", "-1001234567890")
        .await
        .expect("topics query");
    assert_eq!(topics_after.len(), 1);
    assert_eq!(topics_after[0].thread_id, "42");
    assert_eq!(topics_after[0].topic_name.as_deref(), Some("Announcements"));
}
