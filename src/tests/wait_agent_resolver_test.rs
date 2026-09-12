//! Tests for `WaitAgentTool::resolve_agent_id` and
//! `WaitAgentTool::unknown_agent_message`.
//!
//! Pins the resolver behaviour added in 953a895 — previously wait_agent
//! returned a terminal "No sub-agent found" error on any non-exact id,
//! which caused 6/6 failures (100% rate per RSI) on the 2026-04-17
//! logs where the model passed truncated UUIDs, role labels like
//! "clippy", and stale ids.

use crate::brain::tools::subagent::{SubAgent, SubAgentManager, WaitAgentTool};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

/// The session these tests act as (#191). Fixed rather than random: the
/// resolver is now scoped to the caller's session, so a fixture child must be
/// parented to the SAME session the resolver is asked about.
fn parent_session() -> Uuid {
    Uuid::from_u128(0x191)
}

fn mk_agent(id: &str, label: &str) -> SubAgent {
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    SubAgent {
        input_tx: Some(tx),
        ..SubAgent::new(
            id.to_string(),
            label.to_string(),
            Uuid::new_v4(),
            parent_session(),
        )
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
        tool.resolve_agent_id("3b874509abcd1234", parent_session()),
        Some("3b874509abcd1234".into())
    );
}

#[test]
fn unique_prefix_resolves_to_full_id() {
    let tool = tool_with(&[("3b874509abcd1234", "browser"), ("292f89490000ffff", "rsi")]);
    // 4-char prefix that's unique among active agents.
    assert_eq!(
        tool.resolve_agent_id("3b87", parent_session()),
        Some("3b874509abcd1234".into())
    );
    assert_eq!(
        tool.resolve_agent_id("292f", parent_session()),
        Some("292f89490000ffff".into())
    );
}

#[test]
fn too_short_prefix_is_rejected() {
    // Prefix of 3 chars is below the safety threshold — accept only
    // exact id, refuse to guess.
    let tool = tool_with(&[("3b874509abcd1234", "browser")]);
    assert_eq!(tool.resolve_agent_id("3b8", parent_session()), None);
}

#[test]
fn ambiguous_prefix_returns_none() {
    // Both ids start with "3b87" — the resolver must refuse instead of
    // picking one at random.
    let tool = tool_with(&[("3b874509aaaa", "browser"), ("3b87fffffffff", "shell")]);
    assert_eq!(tool.resolve_agent_id("3b87", parent_session()), None);
}

#[test]
fn label_match_works_when_id_does_not() {
    // The 2026-04-17 "No sub-agent found with id: clippy" case: the
    // model passed the role label instead of the UUID.
    let tool = tool_with(&[("3b874509abcd1234", "clippy")]);
    assert_eq!(
        tool.resolve_agent_id("clippy", parent_session()),
        Some("3b874509abcd1234".into())
    );
}

#[test]
fn label_match_is_case_insensitive() {
    let tool = tool_with(&[("3b874509abcd1234", "Clippy")]);
    assert_eq!(
        tool.resolve_agent_id("CLIPPY", parent_session()),
        Some("3b874509abcd1234".into())
    );
    assert_eq!(
        tool.resolve_agent_id("clippy", parent_session()),
        Some("3b874509abcd1234".into())
    );
}

#[test]
fn ambiguous_label_returns_none() {
    // Two agents with the same label: refuse to pick. If this ever
    // trips, upstream should be forced to use a uuid-prefix or full id.
    let tool = tool_with(&[("111aaa", "helper"), ("222bbb", "helper")]);
    assert_eq!(tool.resolve_agent_id("helper", parent_session()), None);
}

#[test]
fn exact_id_wins_over_label_conflict() {
    // Corner case: one agent's id is literally the same string as
    // another agent's label. Exact-id match runs first and returns
    // the id owner.
    let tool = tool_with(&[("helper", "other"), ("222bbb", "helper")]);
    assert_eq!(
        tool.resolve_agent_id("helper", parent_session()),
        Some("helper".into())
    );
}

#[test]
fn unknown_returns_none() {
    let tool = tool_with(&[("3b874509abcd1234", "browser")]);
    assert_eq!(
        tool.resolve_agent_id("not-a-real-id", parent_session()),
        None
    );
}

// ─── unknown_agent_message ─────────────────────────────────────────────────

#[test]
fn empty_list_message_nudges_spawn_agent() {
    let tool = tool_with(&[]);
    let msg = tool.unknown_agent_message("anything", parent_session());
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
    let msg = tool.unknown_agent_message("clippy", parent_session());
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
    let msg = tool.unknown_agent_message("clippy-typo", parent_session());
    assert!(
        msg.contains("clippy-typo"),
        "message should echo the bad input: {msg}"
    );
}

// ─── #191 parent scoping ───────────────────────────────────────────────────

/// A session that is NOT the caller — another lane's chat. The manager is
/// process-global, so its children sit in the same map as ours.
fn other_session() -> Uuid {
    Uuid::from_u128(0x1910)
}

fn mk_agent_for(id: &str, label: &str, parent: Uuid) -> SubAgent {
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    SubAgent {
        input_tx: Some(tx),
        ..SubAgent::new(id.to_string(), label.to_string(), Uuid::new_v4(), parent)
    }
}

/// A tool holding the caller's children (`own`) plus another session's
/// (`foreign`) — the foreign ones are what the pre-fix resolver handed back.
fn tool_with_foreign(own: &[(&str, &str)], foreign: &[(&str, &str)]) -> WaitAgentTool {
    let mgr = Arc::new(SubAgentManager::new());
    for (id, label) in own {
        mgr.insert(mk_agent(id, label));
    }
    for (id, label) in foreign {
        mgr.insert(mk_agent_for(id, label, other_session()));
    }
    WaitAgentTool::new(mgr)
}

/// #191: an exact id belonging to ANOTHER session must not resolve. Pre-fix
/// this took the `manager.exists()` fast path, which is process-global — so a
/// session could name, and then block on, another session's child.
#[test]
fn foreign_exact_id_is_not_resolved() {
    let tool = tool_with_foreign(&[("aaaabbbb", "mine")], &[("ccccdddd", "theirs")]);
    assert_eq!(tool.resolve_agent_id("ccccdddd", parent_session()), None);
}

#[test]
fn foreign_prefix_is_not_resolved() {
    let tool = tool_with_foreign(&[], &[("ccccdddd", "theirs")]);
    assert_eq!(tool.resolve_agent_id("cccc", parent_session()), None);
}

#[test]
fn foreign_label_is_not_resolved() {
    let tool = tool_with_foreign(&[], &[("ccccdddd", "theirs")]);
    assert_eq!(tool.resolve_agent_id("theirs", parent_session()), None);
}

/// Two-sided control: the caller's own child still resolves while a foreign
/// child carrying the SAME label is present — so the fix scopes the query
/// rather than refusing everything (and label ambiguity is still ambiguity).
#[test]
fn own_child_resolves_while_a_foreign_child_is_present() {
    let tool = tool_with_foreign(&[("aaaabbbb", "helper")], &[("ccccdddd", "helper")]);
    assert_eq!(
        tool.resolve_agent_id("aaaabbbb", parent_session()),
        Some("aaaabbbb".into())
    );
    assert_eq!(
        tool.resolve_agent_id("helper", parent_session()),
        Some("aaaabbbb".into())
    );
}

/// The not-found listing must not leak another session's ids either: that
/// message is how a foreign id reaches the model, whose next call resolves it.
#[test]
fn unknown_message_does_not_leak_foreign_agents() {
    let tool = tool_with_foreign(&[("aaaabbbb", "mine")], &[("ccccdddd", "theirs")]);
    let msg = tool.unknown_agent_message("nope", parent_session());
    assert!(
        msg.contains("aaaabbbb"),
        "own child should be listed: {msg}"
    );
    assert!(!msg.contains("ccccdddd"), "foreign child leaked: {msg}");
}
