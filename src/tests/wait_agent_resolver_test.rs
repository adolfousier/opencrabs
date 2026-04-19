//! Tests for `WaitAgentTool::resolve_agent_id` and
//! `WaitAgentTool::unknown_agent_message`.
//!
//! Pins the resolver behaviour added in 953a895 — previously wait_agent
//! returned a terminal "No sub-agent found" error on any non-exact id,
//! which caused 6/6 failures (100% rate per RSI) on the 2026-04-17
//! logs where the model passed truncated UUIDs, role labels like
//! "clippy", and stale ids.

use crate::brain::tools::subagent::{SubAgent, SubAgentManager, SubAgentState, WaitAgentTool};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn mk_agent(id: &str, label: &str) -> SubAgent {
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    SubAgent {
        id: id.to_string(),
        label: label.to_string(),
        session_id: Uuid::new_v4(),
        state: SubAgentState::Running,
        cancel_token: CancellationToken::new(),
        join_handle: None,
        input_tx: Some(tx),
        output: None,
        spawned_at: chrono::Utc::now(),
    }
}

fn tool_with(agents: &[(&str, &str)]) -> WaitAgentTool {
    let mgr = Arc::new(SubAgentManager::new());
    for (id, label) in agents {
        mgr.insert(mk_agent(id, label));
    }
    WaitAgentTool::new(mgr)
}

#[test]
fn exact_id_match_is_the_fast_path() {
    let tool = tool_with(&[("3b874509abcd1234", "browser"), ("292f89490000ffff", "rsi")]);
    assert_eq!(
        tool.resolve_agent_id("3b874509abcd1234"),
        Some("3b874509abcd1234".into())
    );
}

#[test]
fn unique_prefix_resolves_to_full_id() {
    let tool = tool_with(&[("3b874509abcd1234", "browser"), ("292f89490000ffff", "rsi")]);
    // 4-char prefix that's unique among active agents.
    assert_eq!(
        tool.resolve_agent_id("3b87"),
        Some("3b874509abcd1234".into())
    );
    assert_eq!(
        tool.resolve_agent_id("292f"),
        Some("292f89490000ffff".into())
    );
}

#[test]
fn too_short_prefix_is_rejected() {
    // Prefix of 3 chars is below the safety threshold — accept only
    // exact id, refuse to guess.
    let tool = tool_with(&[("3b874509abcd1234", "browser")]);
    assert_eq!(tool.resolve_agent_id("3b8"), None);
}

#[test]
fn ambiguous_prefix_returns_none() {
    // Both ids start with "3b87" — the resolver must refuse instead of
    // picking one at random.
    let tool = tool_with(&[("3b874509aaaa", "browser"), ("3b87fffffffff", "shell")]);
    assert_eq!(tool.resolve_agent_id("3b87"), None);
}

#[test]
fn label_match_works_when_id_does_not() {
    // The 2026-04-17 "No sub-agent found with id: clippy" case: the
    // model passed the role label instead of the UUID.
    let tool = tool_with(&[("3b874509abcd1234", "clippy")]);
    assert_eq!(
        tool.resolve_agent_id("clippy"),
        Some("3b874509abcd1234".into())
    );
}

#[test]
fn label_match_is_case_insensitive() {
    let tool = tool_with(&[("3b874509abcd1234", "Clippy")]);
    assert_eq!(
        tool.resolve_agent_id("CLIPPY"),
        Some("3b874509abcd1234".into())
    );
    assert_eq!(
        tool.resolve_agent_id("clippy"),
        Some("3b874509abcd1234".into())
    );
}

#[test]
fn ambiguous_label_returns_none() {
    // Two agents with the same label: refuse to pick. If this ever
    // trips, upstream should be forced to use a uuid-prefix or full id.
    let tool = tool_with(&[("111aaa", "helper"), ("222bbb", "helper")]);
    assert_eq!(tool.resolve_agent_id("helper"), None);
}

#[test]
fn exact_id_wins_over_label_conflict() {
    // Corner case: one agent's id is literally the same string as
    // another agent's label. Exact-id match runs first and returns
    // the id owner.
    let tool = tool_with(&[("helper", "other"), ("222bbb", "helper")]);
    assert_eq!(tool.resolve_agent_id("helper"), Some("helper".into()));
}

#[test]
fn unknown_returns_none() {
    let tool = tool_with(&[("3b874509abcd1234", "browser")]);
    assert_eq!(tool.resolve_agent_id("not-a-real-id"), None);
}

// ─── unknown_agent_message ─────────────────────────────────────────────────

#[test]
fn empty_list_message_nudges_spawn_agent() {
    let tool = tool_with(&[]);
    let msg = tool.unknown_agent_message("anything");
    assert!(
        msg.contains("no active sub-agents"),
        "empty-list message should say no actives: {msg}"
    );
    assert!(
        msg.contains("spawn_agent"),
        "empty-list message should hint at spawn_agent: {msg}"
    );
}

#[test]
fn populated_message_lists_every_active_with_id_label_state() {
    let tool = tool_with(&[("3b874509abcd1234", "browser"), ("292f89490000ffff", "rsi")]);
    let msg = tool.unknown_agent_message("clippy");
    assert!(msg.contains("3b874509abcd1234"), "lists id 1: {msg}");
    assert!(msg.contains("browser"), "lists label 1: {msg}");
    assert!(msg.contains("292f89490000ffff"), "lists id 2: {msg}");
    assert!(msg.contains("rsi"), "lists label 2: {msg}");
    assert!(msg.contains("Running"), "lists state: {msg}");
    assert!(
        msg.contains("wait_agent"),
        "message tells caller how to use the listing: {msg}"
    );
}

#[test]
fn populated_message_includes_the_bad_input() {
    // The caller's bad string should appear in the message so the
    // model can see exactly what it sent vs what was available.
    let tool = tool_with(&[("111aaa222bbb", "helper")]);
    let msg = tool.unknown_agent_message("clippy-typo");
    assert!(
        msg.contains("clippy-typo"),
        "message should echo the bad input: {msg}"
    );
}
