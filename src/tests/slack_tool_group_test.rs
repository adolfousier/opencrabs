//! Tests for the collapsible Slack tool group (#373-style expand toggle)
//! and the flow-status summary (#1797): always-on tool-call counter, live
//! clock, settled terminal line, and the background-waiting override.

use crate::channels::slack::SlackState;
use crate::channels::slack::tool_group::{GroupEntry, GroupState, TurnOutcome, render};
use slack_morphism::prelude::{SlackChannelId, SlackTs};

fn entries(n: usize, done: bool) -> Vec<GroupEntry> {
    (0..n)
        .map(|i| GroupEntry::Tool {
            name: format!("tool{i}"),
            context: format!(" (arg{i})"),
            status: if done { Some(true) } else { None },
        })
        .collect()
}

fn group(n: usize, done: bool, expanded: bool) -> GroupState {
    let mut g = GroupState::new(SlackChannelId::new("C1".into()), entries(n, done));
    g.expanded = expanded;
    g
}

fn text_of(content: &slack_morphism::prelude::SlackMessageContent) -> String {
    content.text.clone().unwrap_or_default()
}

#[test]
fn collapsed_group_shows_summary_only() {
    let content = render(&group(3, false, false), &SlackTs::new("1.0".into()));
    let text = text_of(&content);
    assert!(text.contains("3 tool calls"), "text: {text}");
    assert!(text.contains("running"));
    assert!(text.contains("🕒"), "live line carries the clock: {text}");
    assert!(!text.contains("tool0"), "collapsed must hide tool rows");
    // Toggle button present: the counter is visible, the rows stay one tap away.
    assert!(content.blocks.as_ref().is_some_and(|b| b.len() == 2));
}

#[test]
fn expanded_group_lists_every_tool() {
    let content = render(&group(3, true, true), &SlackTs::new("1.0".into()));
    let text = text_of(&content);
    assert!(text.contains("tool0") && text.contains("tool2"), "{text}");
    assert!(text.contains("✅"));
    assert!(
        text.contains("3 tool calls"),
        "summary leads the expanded list too: {text}"
    );
}

#[test]
fn single_tool_turn_still_shows_counter() {
    // #1797: a one-tool turn used to render as a bare tool line with no
    // counter at all. Telegram parity: the count is always on.
    let content = render(&group(1, false, false), &SlackTs::new("1.0".into()));
    let text = text_of(&content);
    assert!(text.contains("1 tool call"), "text: {text}");
    assert!(!text.contains("tool0"), "collapsed hides the row: {text}");
    assert!(
        content.blocks.as_ref().is_some_and(|b| b.len() == 2),
        "expand button present"
    );
}

#[test]
fn settled_line_carries_outcome_ctx_and_clock() {
    let mut g = group(2, true, false);
    g.settle(
        TurnOutcome::Finished,
        0,
        Some("ctx: 78K/200K 39% | 854 tok/s".into()),
    );
    let text = text_of(&render(&g, &SlackTs::new("1.0".into())));
    assert!(text.contains("✅ Finished"), "{text}");
    assert!(text.contains("2 tool calls"), "{text}");
    assert!(text.contains("ctx: 78K/200K 39% | 854 tok/s"), "{text}");
    assert!(text.contains("⏱️"), "terminal clock glyph: {text}");
    assert!(!text.contains("🕒"), "waiting glyph must be gone: {text}");
    assert!(
        !text.contains("running"),
        "no live tail on a settled line: {text}"
    );
}

#[test]
fn background_tasks_override_to_waiting() {
    let mut g = group(2, true, false);
    g.settle(TurnOutcome::Finished, 2, Some("ctx: 78K/200K 39%".into()));
    let text = text_of(&render(&g, &SlackTs::new("1.0".into())));
    assert!(text.contains("⏳ Waiting for 2 background tasks"), "{text}");
    assert!(
        text.contains("🕒"),
        "waiting keeps the rolling clock: {text}"
    );
    assert!(!text.contains("Finished"), "{text}");
}

#[test]
fn flip_to_finished_keeps_the_ctx_budget() {
    let mut g = group(2, true, false);
    g.settle(TurnOutcome::Finished, 1, Some("ctx: 10K/200K 5%".into()));
    // The completion arrived and the recount is zero: the flip re-stamps
    // WITHOUT a ctx, and the original budget must survive.
    g.settle(TurnOutcome::Finished, 0, None);
    let text = text_of(&render(&g, &SlackTs::new("1.0".into())));
    assert!(text.contains("✅ Finished"), "{text}");
    assert!(
        text.contains("ctx: 10K/200K 5%"),
        "budget preserved: {text}"
    );
}

#[test]
fn failed_settle_reports_the_outcome() {
    let mut g = group(1, false, false);
    g.settle(TurnOutcome::Failed, 0, None);
    let text = text_of(&render(&g, &SlackTs::new("1.0".into())));
    assert!(text.contains("❌ Failed"), "{text}");
    // A failed settle never overrides to waiting, even with tasks running.
    let mut g = group(1, false, false);
    g.settle(TurnOutcome::Failed, 3, None);
    let text = text_of(&render(&g, &SlackTs::new("1.0".into())));
    assert!(
        text.contains("❌ Failed"),
        "waiting only overrides Finished: {text}"
    );
}

#[tokio::test]
async fn toggle_flips_and_updates_preserve_expansion() {
    let state = SlackState::new();
    state
        .upsert_tool_group("111.0".into(), group(2, false, false))
        .await;
    // User expands.
    let toggled = state.toggle_tool_group("111.0").await.expect("exists");
    assert!(toggled.expanded);
    // A later progress update (built with expanded=false) must NOT snap
    // the group shut: upsert preserves the stored expansion choice.
    let stored = state
        .upsert_tool_group("111.0".into(), group(2, true, false))
        .await;
    assert!(
        stored.expanded,
        "update must preserve user's expanded state"
    );
    assert_eq!(stored.entries.len(), 2);
    // Unknown ts: toggle is a no-op.
    assert!(state.toggle_tool_group("999.9").await.is_none());
}

#[tokio::test]
async fn upsert_preserves_settle_and_turn_start() {
    // #1797: a straggler status update after delivery must never un-settle
    // the group or restart its clock.
    let state = SlackState::new();
    let mut first = group(2, true, false);
    let anchored = first.started_at;
    first.settle(TurnOutcome::Finished, 0, Some("ctx: 1K/200K 1%".into()));
    state.upsert_tool_group("111.0".into(), first).await;
    // A late update built fresh by a progress callback.
    let stored = state
        .upsert_tool_group("111.0".into(), group(2, true, false))
        .await;
    assert!(stored.settled.is_some(), "settle must survive upsert");
    assert_eq!(
        stored.started_at, anchored,
        "clock anchor must survive upsert"
    );
    assert!(
        stored.settled.unwrap().ctx.is_some(),
        "stamped ctx must survive upsert"
    );
}

#[tokio::test]
async fn waiting_group_flip_roundtrip() {
    let state = SlackState::new();
    assert!(state.take_waiting_group_for("C1").await.is_none());
    state
        .note_waiting_group("C1".into(), "111.0".into(), uuid::Uuid::new_v4())
        .await;
    // Another channel never sees it.
    assert!(state.take_waiting_group_for("C2").await.is_none());
    let (ts, _session) = state.take_waiting_group_for("C1").await.expect("waiting");
    assert_eq!(ts, "111.0");
    // Taken: a second flip is a no-op.
    assert!(state.take_waiting_group_for("C1").await.is_none());
}

#[tokio::test]
async fn retention_prunes_oldest_groups() {
    let state = SlackState::new();
    for i in 0..25 {
        state
            .upsert_tool_group(format!("{i}.0"), group(2, true, false))
            .await;
    }
    // Cap is 20: the first five aged out, the newest survive.
    assert!(state.toggle_tool_group("0.0").await.is_none());
    assert!(state.toggle_tool_group("4.0").await.is_none());
    assert!(state.toggle_tool_group("24.0").await.is_some());
}
