//! The `usage` meter frame must match the official ACP v1 schema (#1815 F1).
//!
//! OpenCrabs emitted `{"sessionUpdate": "usage", "usage": {"used", "size"}}`
//! at two call sites. That variant exists in no schema version: a client
//! generated from `schema/v1/schema.json` sees an unknown tagged variant and
//! drops the notification, so the context meter never moved outside the
//! MonoCode pairing, which accepts the shape by coincidence rather than by
//! contract.
//!
//! The expectations below were read off the fetched schema (247168 bytes,
//! 2026-10-02), not from this file's own constants: `SessionUpdate.oneOf` has
//! exactly eleven variants, `usage_update` is one of them, `usage` is not, and
//! `UsageUpdate` folds in via `allOf` with `used` and `size` required as flat
//! `uint64` siblings of `sessionUpdate` with `minimum: 0`.

use crate::acp::protocol::{SESSION_UPDATE, session_update, usage_update};
use serde_json::Value;

/// The tagged-variant names the v1 schema admits for `session/update`.
/// Copy of `SessionUpdate.oneOf[*].properties.sessionUpdate.const`.
const V1_SESSION_UPDATE_TAGS: &[&str] = &[
    "user_message_chunk",
    "agent_message_chunk",
    "agent_thought_chunk",
    "tool_call",
    "tool_call_update",
    "plan",
    "available_commands_update",
    "current_mode_update",
    "config_option_update",
    "session_info_update",
    "usage_update",
];

#[test]
fn the_usage_frame_carries_the_schema_tag() {
    let frame = usage_update(123, 200_000);
    assert_eq!(
        frame["sessionUpdate"].as_str(),
        Some("usage_update"),
        "a tag outside the oneOf list is a frame clients are allowed to drop"
    );
}

#[test]
fn used_and_size_are_flat_siblings_not_a_nested_object() {
    let frame = usage_update(123, 200_000);
    assert_eq!(frame["used"], Value::from(123));
    assert_eq!(frame["size"], Value::from(200_000));
    assert!(
        frame.get("usage").is_none(),
        "the nested `usage` wrapper is the deviation this replaced: {frame}"
    );
    let keys: Vec<&str> = frame
        .as_object()
        .expect("an object frame")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys.len(), 3, "exactly tag, used, size: {keys:?}");
}

#[test]
fn the_emitted_tag_is_a_member_of_the_v1_variant_list() {
    let tag = usage_update(0, 0)["sessionUpdate"]
        .as_str()
        .expect("a string tag")
        .to_string();
    assert!(
        V1_SESSION_UPDATE_TAGS.contains(&tag.as_str()),
        "`{tag}` is not one of the {n} variants v1 admits",
        n = V1_SESSION_UPDATE_TAGS.len()
    );
    // And the name we used to send is provably absent from the list, so this
    // test cannot pass by both spellings being accepted.
    assert!(!V1_SESSION_UPDATE_TAGS.contains(&"usage"));
    assert_eq!(V1_SESSION_UPDATE_TAGS.len(), 11);
}

#[test]
fn cost_stays_absent_because_we_do_not_have_one() {
    // `cost` is optional upstream; sending nothing is conformant, sending a
    // guessed number is not.
    let frame = usage_update(10, 100);
    assert!(frame.get("cost").is_none());
}

#[test]
fn zero_is_a_legal_meter_reading() {
    // `minimum: 0`, so an empty context must still be renderable rather than
    // filtered out by the caller.
    let frame = usage_update(0, 0);
    assert_eq!(frame["used"], Value::from(0));
    assert_eq!(frame["size"], Value::from(0));
}

#[test]
fn the_full_wire_frame_is_a_session_update_notification() {
    let wire = session_update("acp-sess-1", usage_update(4_096, 200_000));
    assert_eq!(wire["jsonrpc"], Value::from("2.0"));
    assert_eq!(wire["method"], Value::from(SESSION_UPDATE));
    assert_eq!(wire["params"]["sessionId"], Value::from("acp-sess-1"));
    let update = &wire["params"]["update"];
    assert_eq!(update["sessionUpdate"], Value::from("usage_update"));
    assert_eq!(update["used"], Value::from(4_096));
    assert_eq!(update["size"], Value::from(200_000));
    // A notification must not carry an id.
    assert!(wire.get("id").is_none());
}

#[test]
fn no_call_site_still_speaks_the_bogus_usage_tag() {
    // Both sites were `json!` literals, so a source-level pin is the honest
    // regression guard: the day someone re-inlines the frame, this fails.
    for (name, src) in [
        ("turn.rs", include_str!("../acp/turn.rs")),
        ("server.rs", include_str!("../acp/server.rs")),
    ] {
        assert!(
            !src.contains("\"sessionUpdate\": \"usage\"")
                && !src.contains("\"sessionUpdate\":\"usage\""),
            "{name} still emits the variant that is not in the v1 schema"
        );
    }
}

#[test]
fn both_meter_sites_route_through_the_one_constructor() {
    let turn = include_str!("../acp/turn.rs");
    let server = include_str!("../acp/server.rs");
    assert!(
        turn.contains("protocol::usage_update("),
        "the live-turn meter must use the shared shape, not a local literal"
    );
    assert!(
        server.contains("protocol::usage_update("),
        "the session/load replay meter must use the shared shape"
    );
    // Exactly one place in the crate builds this frame.
    let protocol = include_str!("../acp/protocol.rs");
    assert_eq!(
        protocol
            .matches("\"sessionUpdate\": \"usage_update\"")
            .count(),
        1,
        "two constructors drift; one does not"
    );
}
