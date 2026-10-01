//! Regression tests for the Slack follow-up suggestion tap (#1838).
//!
//! A message typed by a human runs the tool-loop display path: the flow group
//! opens immediately with its 🕒 clock, tool steps and narration fold into it,
//! and the answer settles it. The tap used to ride the bare
//! single-completion send — zero tools, zero progress events — so the channel
//! sat silent from the echo until the answer landed minutes later and the
//! user had no way to know the tap worked. These guard the wiring
//! structurally, the same way `cli_headless_tools_test.rs` guards #492.

/// The tap branch must hand the turn to the tool-loop display path and never
/// call the bare single-completion send again.
#[test]
fn followup_tap_branch_delegates_to_the_display_path() {
    let handler = include_str!("../channels/slack/handler.rs");
    let start = handler
        .find("FOLLOWUP_PREFIX)")
        .expect("follow-up tap branch present");
    let rest = &handler[start..];
    let end = rest
        .find("continue;")
        .expect("tap branch terminates with continue");
    let branch = &rest[..end];
    assert!(
        branch.contains("run_followup_turn("),
        "#1838: a tapped suggestion must run through run_followup_turn"
    );
    assert!(
        !branch.contains(".send_message("),
        "the tap branch must not ride the bare single-completion send"
    );
}

/// The tap turn itself must ride the tool-loop display path with both
/// callbacks wired, and the live status must be born BEFORE the turn is
/// dispatched — that ordering is the whole of #1838.
#[test]
fn tap_turn_opens_live_status_before_dispatching() {
    let handler = include_str!("../channels/slack/handler.rs");
    let start = handler
        .find("async fn run_followup_turn(")
        .expect("tap-turn helper present");
    let end = handler[start..]
        .find("pub(crate) fn make_approval_callback(")
        .expect("helper sits directly above the approval-callback builder");
    let body = &handler[start..start + end];

    assert!(
        body.contains("send_message_with_tools_and_display("),
        "the tap turn must call the tool-loop display entry point"
    );
    assert!(
        !body.contains(".send_message("),
        "the tap turn must never regress to the bare send"
    );
    assert!(
        body.contains("Some(progress_cb)"),
        "progress events must reach the channel callback"
    );
    assert!(
        body.contains("Some(approval_cb)"),
        "tool approvals must route to the Slack buttons"
    );

    // #1838 guarantee: the status is visible from second zero, so the group
    // and its ticker spawn before the agent turn starts.
    let dispatch = body
        .find("send_message_with_tools_and_display(")
        .expect("turn dispatch present");
    let ticker = body
        .find("spawn_flow_ticker(")
        .expect("the flow ticker must run for tap turns");
    assert!(
        ticker < dispatch,
        "spawn_flow_ticker must run before the tap turn is dispatched"
    );
    let first_group = body
        .find("sync_step_group(")
        .expect("the step group must open at turn start");
    assert!(
        first_group < dispatch,
        "sync_step_group must create the group before the turn is dispatched"
    );
}

/// The tap path must handle the chained-suggestion event: buttons minted by a
/// tapped turn would otherwise die silently, recreating the bug this fix
/// removes one level up.
#[test]
fn tap_path_renders_chained_suggestions() {
    let handler = include_str!("../channels/slack/handler.rs");
    let start = handler
        .find("async fn run_followup_turn(")
        .expect("tap-turn helper present");
    let rest = &handler[start..];
    let end = rest
        .find("pub(crate) fn make_approval_callback(")
        .expect("end marker present");
    let body = &rest[..end];
    assert!(
        body.contains("SuggestedOptions"),
        "the tap progress callback must observe suggestion events"
    );
    assert!(
        body.contains("render_suggestions("),
        "suggestion events must post clickable buttons again"
    );
}
