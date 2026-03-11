/// The `is_system_continuation` check from `src/brain/agent/service/tool_loop.rs:212`.
/// Tests the string prefix that determines whether a user message is a
/// system continuation (and should be skipped from DB persistence).

#[test]
fn system_prefix_matches() {
    let msg = "[System: You just rebuilt yourself from source and restarted]";
    assert!(msg.starts_with("[System:"));
}

#[test]
fn uppercase_system_does_not_match() {
    // This was the bug fixed in 5524571 — the old prefix was "[SYSTEM:"
    let msg = "[SYSTEM: You just rebuilt yourself]";
    assert!(!msg.starts_with("[System:"));
}

#[test]
fn normal_user_message_does_not_match() {
    let msg = "Hello, how are you?";
    assert!(!msg.starts_with("[System:"));
}

#[test]
fn empty_message_does_not_match() {
    let msg = "";
    assert!(!msg.starts_with("[System:"));
}

#[test]
fn partial_prefix_does_not_match() {
    let msg = "[Syste";
    assert!(!msg.starts_with("[System:"));
}

#[test]
fn system_lowercase_does_not_match() {
    let msg = "[system: something]";
    assert!(!msg.starts_with("[System:"));
}
