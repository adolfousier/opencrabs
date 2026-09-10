//! #148 Step 1: the ambient `origin_target` stamp on `ToolExecutionContext`.
//!
//! Pins the CONTRACT, not the channel wiring (the tool-loop derivation is
//! integration-tested through the channel suites): the field exists, defaults
//! to `None` — the headless contract cron/CLI/sub-agent surfaces rely on —
//! and the `OriginTarget::deliver_to` bake rules hold, including the #1319
//! General-topic rule (`:1` never becomes a wire address).

use crate::brain::tools::{OriginTarget, ToolExecutionContext};

#[test]
fn new_context_has_no_origin_target() {
    // Headless flows (cron execute, CLI one-shot, sub-agents, tests) build
    // contexts via `new`; the ambient origin MUST default to None so `here`
    // resolution is refused rather than guessed.
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    assert!(
        ctx.origin_target.is_none(),
        "a fresh ToolExecutionContext must carry no ambient origin"
    );
}

#[test]
fn deliver_to_bakes_chat_only_for_general_topic() {
    // General topic (1) is a SESSION-SCOPING key (#1220), never a wire
    // address (#1319): the bake must NOT append `:1`.
    let general = OriginTarget {
        channel: "telegram",
        chat_id: "-1001234567890".to_string(),
        thread: Some(crate::channels::telegram::session_resolve::GENERAL_TOPIC_ID),
    };
    assert_eq!(general.deliver_to(), "telegram:-1001234567890");
}

#[test]
fn deliver_to_bakes_real_thread_and_bare_chat() {
    let forum = OriginTarget {
        channel: "telegram",
        chat_id: "-1001234567890".to_string(),
        thread: Some(42),
    };
    assert_eq!(forum.deliver_to(), "telegram:-1001234567890:42");

    let dm = OriginTarget {
        channel: "telegram",
        chat_id: "133526395".to_string(),
        thread: None,
    };
    assert_eq!(dm.deliver_to(), "telegram:133526395");

    let discord = OriginTarget {
        channel: "discord",
        chat_id: "123456789012345678".to_string(),
        thread: None,
    };
    assert_eq!(discord.deliver_to(), "discord:123456789012345678");
}
