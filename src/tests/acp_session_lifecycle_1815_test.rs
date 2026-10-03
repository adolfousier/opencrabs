//! #1815 F5: the v1 session lifecycle methods.
//!
//! Before this, `dispatch_request` had no arm for `session/list`,
//! `session/resume`, `session/set_config_option`, `session/close` or
//! `session/delete`, and every one of them fell into the catch-all answering
//! `METHOD_NOT_FOUND` -- while `initialize` advertised `loadSession: true`, so
//! a client that believed us had a working session model and asked for a
//! sibling of the one method we did implement.
//!
//! These tests pin the wire vocabulary the handlers answer with: the exact
//! method strings, the capability advertisement that makes the methods
//! discoverable, and the required/optional field rules taken from the official
//! `schema/v1/schema.json` (`ListSessionsResponse` requires `sessions`;
//! `SessionInfo` requires `sessionId` and `cwd`; a null optional is OMITTED,
//! not sent as null).

use crate::acp::protocol;
use crate::acp::server::config_option_value;
use serde_json::{Value, json};

/// The five method names, spelled exactly as v1 spells them. A typo here is a
/// client-facing `METHOD_NOT_FOUND` that no compiler will catch.
#[test]
fn f5_method_names_match_the_spec() {
    assert_eq!(protocol::SESSION_LIST, "session/list");
    assert_eq!(protocol::SESSION_RESUME, "session/resume");
    assert_eq!(
        protocol::SESSION_SET_CONFIG_OPTION,
        "session/set_config_option"
    );
    assert_eq!(protocol::SESSION_CLOSE, "session/close");
    assert_eq!(protocol::SESSION_DELETE, "session/delete");
}

/// `initialize` must advertise what we now answer, or the methods stay
/// undiscoverable: `sessionCapabilities` is the gate the schema names for
/// list / resume / close / delete.
#[test]
fn initialize_advertises_the_lifecycle_capabilities() {
    let result = protocol::initialize_result();
    let caps = &result["agentCapabilities"]["sessionCapabilities"];
    assert_eq!(caps["list"], json!({}), "session/list must be advertised");
    assert_eq!(
        caps["resume"],
        json!({}),
        "session/resume must be advertised"
    );
    assert_eq!(caps["close"], json!({}), "session/close must be advertised");
    assert_eq!(
        caps["delete"],
        json!({}),
        "session/delete must be advertised"
    );
}

/// The advertisement is honest in the other direction too:
/// `additionalDirectories` has a capability bit and we do NOT set it, because
/// the field is accepted and ignored on our session requests. Claiming it would
/// promise workspace roots we never activate.
#[test]
fn additional_directories_stays_unadvertised() {
    let init = protocol::initialize_result();
    let caps = init["agentCapabilities"]["sessionCapabilities"]
        .as_object()
        .expect("sessionCapabilities is an object");
    assert!(
        !caps.contains_key("additionalDirectories"),
        "advertised a workspace-roots capability we do not implement: {caps:?}"
    );
}

/// `ListSessionsResponse` requires `sessions`. An empty store answers with an
/// empty array, never an absent field -- and `nextCursor` appears only when a
/// next page exists, per the schema ("If absent, there are no more results").
#[test]
fn list_response_always_carries_sessions_array() {
    let last = protocol::list_sessions_payload(vec![], None);
    assert!(
        last.get("sessions").and_then(Value::as_array).is_some(),
        "sessions is required: {last}"
    );
    assert_eq!(last["sessions"], json!([]));
    assert!(
        last.get("nextCursor").is_none(),
        "a last page must not carry a cursor: {last}"
    );

    let page = protocol::list_sessions_payload(
        vec![protocol::session_info("abc", "/tmp", None, None)],
        Some("50".to_string()),
    );
    assert_eq!(page["nextCursor"], json!("50"));
    assert_eq!(page["sessions"].as_array().map(Vec::len), Some(1));
}

/// `SessionInfo` requires `sessionId` and `cwd`; `title` and `updatedAt` are
/// optional and omitted when unknown rather than emitted as null (a strict
/// client reading `title: null` against `"type": ["string","null"]` is fine,
/// but omitting is what the reference serializer does).
#[test]
fn session_info_required_fields_and_omissions() {
    let full = protocol::session_info(
        "sid-1",
        "/Users/dev/project",
        Some("Telegram: Ops [chat:9]"),
        Some("2026-10-03T02:12:00+00:00"),
    );
    assert_eq!(full["sessionId"], json!("sid-1"));
    assert_eq!(full["cwd"], json!("/Users/dev/project"));
    assert_eq!(full["title"], json!("Telegram: Ops [chat:9]"));
    assert_eq!(full["updatedAt"], json!("2026-10-03T02:12:00+00:00"));

    let bare = protocol::session_info("sid-2", "/srv/app", None, None);
    assert!(bare.get("title").is_none(), "title must be omitted: {bare}");
    assert!(
        bare.get("updatedAt").is_none(),
        "updatedAt must be omitted: {bare}"
    );
}

/// `SetSessionConfigOptionResponse` requires the FULL option set with current
/// values, which is what lets a client resync its picker after a write without
/// re-running `session/new`.
#[test]
fn config_option_response_requires_the_full_set() {
    let options = json!([
        { "id": "model", "name": "Model", "currentValue": "anthropic/claude-opus-5" }
    ]);
    let response = protocol::config_options_response(options.clone());
    assert_eq!(
        response.get("configOptions"),
        Some(&options),
        "configOptions must carry the whole set: {response}"
    );
}

/// `session/resume` answers with `ResumeSessionResponse`, which has no
/// `sessionId` field at all -- the same rule `session/load` already follows
/// (#1815 F4). One builder, both spellings, so neither can start echoing.
#[test]
fn resume_response_does_not_echo_session_id() {
    let response = protocol::session_response(
        "6f0f6b4a-0000-4000-8000-000000000001",
        json!({ "availableModels": [], "currentModelId": "" }),
        json!({ "currentModeId": "auto", "availableModes": [] }),
        json!([]),
        false,
    );
    assert!(
        response.get("sessionId").is_none(),
        "resume/load responses must not carry sessionId: {response}"
    );
    assert!(
        response.get("configOptions").is_some() && response.get("modes").is_some(),
        "the fields ResumeSessionResponse does define: {response}"
    );
}

/// v1 carries the requested value as a tagged union. The string form is the
/// default when `type` is absent; the boolean form is `{ "type": "boolean",
/// "value": true }`. The only option published is a model select, so a boolean
/// is refused instead of becoming a model named "true".
#[test]
fn config_value_union_accepts_string_and_refuses_boolean() {
    let string_form = json!({ "sessionId": "s", "configId": "model", "value": "openai/gpt-5" });
    assert_eq!(
        config_option_value(&string_form),
        Some("openai/gpt-5".to_string())
    );

    let boolean_form = json!({
        "sessionId": "s", "configId": "model",
        "value": { "type": "boolean", "value": true }
    });
    assert_eq!(
        config_option_value(&boolean_form),
        None,
        "an object value is the boolean variant; it cannot name a model"
    );

    for absent in [
        json!({ "sessionId": "s", "configId": "model" }),
        json!({ "sessionId": "s", "configId": "model", "value": 42 }),
    ] {
        assert_eq!(
            config_option_value(&absent),
            None,
            "only a string is a usable value: {absent}"
        );
    }
}
