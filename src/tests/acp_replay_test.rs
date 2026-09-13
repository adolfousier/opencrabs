//! ACP transcript-replay mapping: stored message rows → session/update
//! chunk shapes, split out of protocol.rs to keep the module under the
//! 500-line ceiling.

use crate::acp::protocol::replay_updates;
use crate::db::models::Message;
use serde_json::{Value, json};

fn msg(role: &str, content: &str, thinking: Option<&str>) -> Message {
    Message {
        id: uuid::Uuid::new_v4(),
        session_id: uuid::Uuid::new_v4(),
        role: role.to_string(),
        content: content.to_string(),
        sequence: 0,
        created_at: chrono::Utc::now(),
        token_count: None,
        cost: None,
        input_tokens: None,
        cache_creation_tokens: None,
        cache_read_tokens: None,
        thinking: thinking.map(str::to_string),
        duration_secs: None,
    }
}

fn kind_of(update: &Value) -> &str {
    update["sessionUpdate"].as_str().unwrap()
}

#[test]
fn replays_user_and_assistant_in_order() {
    let messages = vec![
        msg("user", "hello", None),
        msg("assistant", "hi there", None),
        msg("user", "next", None),
    ];
    let updates = replay_updates(&messages);
    let kinds: Vec<&str> = updates.iter().map(kind_of).collect();
    assert_eq!(
        kinds,
        [
            "user_message_chunk",
            "agent_message_chunk",
            "user_message_chunk"
        ]
    );
    assert_eq!(updates[1]["content"]["text"], json!("hi there"));
}

#[test]
fn replays_thinking_and_inline_reasoning_as_thought_chunks() {
    let messages = vec![msg(
        "assistant",
        "<!-- reasoning -->pondering<!-- /reasoning -->visible answer",
        Some("persisted thought"),
    )];
    let updates = replay_updates(&messages);
    let kinds: Vec<&str> = updates.iter().map(kind_of).collect();
    assert_eq!(
        kinds,
        [
            "agent_thought_chunk",
            "agent_thought_chunk",
            "agent_message_chunk"
        ]
    );
    assert_eq!(updates[0]["content"]["text"], json!("persisted thought"));
    assert_eq!(updates[1]["content"]["text"], json!("pondering"));
    assert_eq!(updates[2]["content"]["text"], json!("visible answer"));
}

#[test]
fn skips_empty_and_non_transcript_roles() {
    let messages = vec![
        msg("user", "   ", None),
        msg("system", "[SYSTEM: Compact context now.]", None),
        msg("tool", "tool output", None),
        msg("user", "real", None),
    ];
    let updates = replay_updates(&messages);
    assert_eq!(updates.len(), 1);
    assert_eq!(kind_of(&updates[0]), "user_message_chunk");
}
