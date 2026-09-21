//! Protocol framing tests for the ACP server mode (#1540, #1539).
//!
//! Landed under `src/tests/` per the contribution rule that test modules
//! never live inline: the items exercised here (`parse_line`, `prompt_text`,
//! `tool_kind`, `permission_outcome`, `ClientMessage`) are all `pub` in
//! `crate::acp::protocol`, so no test-only surface has to leak into the
//! production module.

use crate::acp::protocol::{
    AcpMode, ClientMessage, SESSION_SET_MODE, SESSION_SET_MODEL, initialize_result, modes_payload,
    parse_line, permission_outcome, prompt_text, replay_updates, tool_kind,
};
use crate::db::models::Message;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

#[test]
fn parses_request() {
    let msg =
        parse_line(r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/tmp"}}"#)
            .unwrap();
    match msg {
        ClientMessage::Request { id, method, params } => {
            assert_eq!(id, json!(1));
            assert_eq!(method, "session/new");
            assert_eq!(params["cwd"], json!("/tmp"));
        }
        other => panic!("expected request, got {other:?}"),
    }
}

#[test]
fn parses_notification() {
    let msg = parse_line(r#"{"jsonrpc":"2.0","method":"session/cancel","params":{}}"#).unwrap();
    assert!(matches!(msg, ClientMessage::Notification { .. }));
}

#[test]
fn parses_response_result_and_error() {
    let ok = parse_line(r#"{"jsonrpc":"2.0","id":7,"result":{"x":1}}"#).unwrap();
    match ok {
        ClientMessage::Response { id, result } => {
            assert_eq!(id, 7);
            assert_eq!(result.unwrap()["x"], json!(1));
        }
        other => panic!("expected response, got {other:?}"),
    }
    let err =
        parse_line(r#"{"jsonrpc":"2.0","id":8,"error":{"code":-32601,"message":"no"}}"#).unwrap();
    match err {
        ClientMessage::Response { id, result } => {
            assert_eq!(id, 8);
            let e = result.unwrap_err();
            assert_eq!(e.code, -32601);
        }
        other => panic!("expected response, got {other:?}"),
    }
}

#[test]
fn rejects_garbage() {
    assert!(parse_line("not json").is_err());
    assert!(parse_line(r#"{"jsonrpc":"2.0"}"#).is_err());
}

#[test]
fn extracts_prompt_text_blocks() {
    let params = json!({
        "prompt": [
            { "type": "text", "text": "hello" },
            { "type": "resource_link", "uri": "file:///tmp/a.png" },
            { "type": "image", "data": "..." }
        ]
    });
    let text = prompt_text(&params).unwrap();
    assert!(text.starts_with("hello"));
    assert!(text.contains("file:///tmp/a.png"));
}

#[test]
fn permission_outcome_maps_option_ids() {
    assert_eq!(
        permission_outcome(&json!({"outcome":{"outcome":"selected","optionId":"allow-once"}})),
        (true, false)
    );
    assert_eq!(
        permission_outcome(&json!({"outcome":{"outcome":"selected","optionId":"allow-always"}})),
        (true, true)
    );
    assert_eq!(
        permission_outcome(&json!({"outcome":{"outcome":"selected","optionId":"reject-once"}})),
        (false, false)
    );
    assert_eq!(
        permission_outcome(&json!({"outcome":{"outcome":"cancelled"}})),
        (false, false)
    );
}

#[test]
fn tool_kind_covers_loop_tools() {
    assert_eq!(tool_kind("bash"), "execute");
    assert_eq!(tool_kind("read_file"), "read");
    assert_eq!(tool_kind("edit_file"), "edit");
    assert_eq!(tool_kind("grep"), "search");
    assert_eq!(tool_kind("http_request"), "fetch");
    assert_eq!(tool_kind("spawn_agent"), "other");
}

#[test]
fn initialize_result_is_honest_about_load_replay() {
    let caps = initialize_result()["agentCapabilities"].clone();
    // session/load binds an existing session AND replays the stored
    // transcript before answering; the advertised capability must not claim
    // otherwise.
    assert_eq!(caps["loadSession"], json!(true));
    assert_eq!(caps["promptCapabilities"]["text"], json!(true));
}

#[test]
fn set_mode_is_accepted_alongside_set_model() {
    // The contract Moe's PR body promised is spelled `session/set_mode`;
    // both spellings dispatch to the same handler.
    assert_eq!(SESSION_SET_MODE, "session/set_mode");
    assert_eq!(SESSION_SET_MODEL, "session/set_model");
}

#[test]
fn mode_parse_round_trips_advertised_ids() {
    for id in [
        "supervised",
        "auto-accept-edits",
        "auto",
        "full-access",
        "plan",
    ] {
        let mode = AcpMode::parse(id).expect("advertised id parses");
        assert_eq!(mode.id(), id);
    }
    assert!(AcpMode::parse("yolo").is_none());
}

#[test]
fn modes_payload_names_current() {
    let payload = modes_payload(AcpMode::Plan);
    assert_eq!(payload["currentModeId"], json!("plan"));
    let ids: Vec<&str> = payload["availableModes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            "supervised",
            "auto-accept-edits",
            "auto",
            "full-access",
            "plan"
        ]
    );
}

#[test]
fn replay_updates_maps_reasoning_blocked_and_text_segments() {
    let msg = Message {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        role: "assistant".into(),
        content: "<!-- reasoning -->\nthinking hard\n<!-- /reasoning -->\n\
                  <!-- phantom_blocked=1 -->\nphantom narration\n<!-- /phantom_blocked=1 -->\n\
                  the visible answer"
            .into(),
        sequence: 1,
        created_at: Utc::now(),
        token_count: None,
        cost: None,
        input_tokens: None,
        cache_creation_tokens: None,
        cache_read_tokens: None,
        thinking: None,
        duration_secs: None,
    };
    let updates = replay_updates(&[msg]);
    let (kinds, texts): (Vec<&str>, Vec<&str>) = updates
        .iter()
        .map(|u| {
            (
                u["sessionUpdate"].as_str().unwrap(),
                u["content"]["text"].as_str().unwrap(),
            )
        })
        .unzip();
    assert_eq!(
        kinds,
        vec![
            "agent_thought_chunk",
            "agent_thought_chunk",
            "agent_message_chunk",
        ]
    );
    assert_eq!(texts[0], "thinking hard");
    assert!(texts[1].contains("Blocked narration"));
    assert!(texts[1].contains("phantom narration"));
    assert_eq!(texts[2], "the visible answer");
}
