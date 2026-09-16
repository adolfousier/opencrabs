//! Tests for the collapsible Discord tool group (#380): render modes,
//! toggle/preservation semantics, and retention pruning — mirroring the
//! Slack port's contracts.

use crate::channels::discord::DiscordState;
use crate::channels::discord::tool_group::{
    GroupEntry, GroupState, render_components, render_content,
};

fn entries(n: usize, done: bool) -> Vec<GroupEntry> {
    (0..n)
        .map(|i| GroupEntry {
            name: format!("tool{i}"),
            context: format!(" (arg{i})"),
            status: if done { Some(true) } else { None },
        })
        .collect()
}

fn group(n: usize, done: bool, expanded: bool) -> GroupState {
    GroupState {
        entries: entries(n, done),
        expanded,
        notes: Vec::new(),
    }
}

#[test]
fn collapsed_shows_summary_expanded_lists_tools() {
    let collapsed = render_content(&group(3, false, false));
    assert!(collapsed.contains("3 tool calls"));
    assert!(collapsed.contains("running"));
    assert!(!collapsed.contains("tool0"));

    let expanded = render_content(&group(3, true, true));
    assert!(expanded.contains("tool0") && expanded.contains("tool2"));

    let single = render_content(&group(1, false, false));
    assert!(single.contains("tool0"));
    assert!(!single.contains("tool call"));
}

#[test]
fn toggle_button_only_for_multi_tool_groups() {
    assert!(render_components(&group(1, false, false), 7).is_empty());
    assert_eq!(render_components(&group(2, false, false), 7).len(), 1);
}

#[tokio::test]
async fn toggle_flips_and_updates_preserve_expansion() {
    let state = DiscordState::new();
    state.upsert_tool_group(111, group(2, false, false)).await;
    let toggled = state.toggle_tool_group(111).await.expect("exists");
    assert!(toggled.expanded);
    // Progress updates (built collapsed) must preserve the user's choice.
    let stored = state.upsert_tool_group(111, group(2, true, false)).await;
    assert!(stored.expanded);
    assert!(state.toggle_tool_group(999).await.is_none());
}

#[tokio::test]
async fn retention_prunes_oldest_groups() {
    let state = DiscordState::new();
    for i in 0..25u64 {
        state.upsert_tool_group(i, group(2, true, false)).await;
    }
    assert!(state.toggle_tool_group(0).await.is_none());
    assert!(state.toggle_tool_group(24).await.is_some());
}
