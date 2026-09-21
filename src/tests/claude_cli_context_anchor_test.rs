//! Claude CLI's `context_input()` never reaches the ctx meter or the budget (#1677).
//!
//! The field reports Claude Code's own internal cache, not the prompt
//! OpenCrabs sent. `tool_loop` already refused it on the calibration branch
//! ("represents Claude's internal cache, not our sent context") while feeding
//! the same value into `call_context_tokens` sixty lines above, which is what
//! the footer displays and what `record_provider_reported_tokens` anchors the
//! compaction budget on. One day of logs carried both failure directions: a
//! 236,325 report against a 122,082 local estimate pushed the budget to 118%
//! of a 200k window, and a report of `2` against a 28,688 estimate collapsed
//! it so compaction could never fire. The only guard was
//! `is_implausible_token_report`, a 2x heuristic that lets both through.
//!
//! The gate lives inside `run_tool_loop`, which no unit test can call, so the
//! source is the assertion surface.

const TOOL_LOOP: &str = include_str!("../brain/agent/service/tool_loop.rs");

const DECL: &str =
    r#"let is_claude_cli = self.provider_for_session(session_id).name() == "claude-cli";"#;

#[test]
fn the_provider_check_is_declared_once_and_before_the_context_figure() {
    assert_eq!(
        TOOL_LOOP.matches(DECL).count(),
        1,
        "one declaration only — a second one below the anchor is how the gate got missed"
    );

    let decl = TOOL_LOOP.find(DECL).expect("is_claude_cli declaration");
    let call_ctx = TOOL_LOOP
        .find("let call_context_tokens =")
        .expect("call_context_tokens binding");
    assert!(
        decl < call_ctx,
        "the provider check must be hoisted above the figure it gates"
    );
}

#[test]
fn the_displayed_context_figure_skips_claude_cli() {
    assert!(
        TOOL_LOOP.contains("let call_context_tokens = if reported_usage && !is_claude_cli {"),
        "claude-cli must fall through to the local tiktoken estimate, not context_input()"
    );
}

#[test]
fn the_budget_anchor_skips_claude_cli() {
    let anchor = TOOL_LOOP
        .find("context.record_provider_reported_tokens(")
        .expect("anchor call site");
    let guard = TOOL_LOOP[..anchor]
        .rfind("if reported_usage")
        .expect("anchor guard");
    let guard_block = &TOOL_LOOP[guard..anchor];
    assert!(
        guard_block.contains("!is_claude_cli"),
        "the anchor guard must exclude claude-cli, not lean on the 2x plausibility heuristic"
    );
}

#[test]
fn the_calibration_branch_still_refuses_the_same_field() {
    assert!(
        TOOL_LOOP.contains("represents Claude's internal cache, not our sent context"),
        "the calibration branch is the stated reason for the gate above; keep them together"
    );
}
