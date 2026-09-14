use crate::channels::telegram::intermediates::{
    is_deliverable_rich_report, is_deliverable_status_report,
};

#[test]
fn status_update_heading_is_deliverable() {
    let text = "### Status Update: Issue #213\nCarrier build dispatched.\nWaiting on CI gate.";
    assert!(is_deliverable_status_report(text));
    assert!(is_deliverable_rich_report(text));
}

#[test]
fn pipeline_state_heading_is_deliverable() {
    let text = "### Current Pipeline State\n- Step 1: Worktree created\n- Step 2: Gating code";
    assert!(is_deliverable_status_report(text));
    assert!(is_deliverable_rich_report(text));
}

#[test]
fn substantial_general_atx_section_is_deliverable() {
    let text = "## Audit Findings\n\
        The audit of the telemetry layer identified three distinct latency bottlenecks.\n\
        First, log serialization runs synchronously on the main event handling thread.\n\
        Second, unbuffered socket writes stall under heavy load.\n\
        Third, timeout handling lacks backoff retries.";
    assert!(text.chars().count() >= 150);
    assert!(is_deliverable_status_report(text));
    assert!(is_deliverable_rich_report(text));
}

#[test]
fn structured_callout_is_deliverable() {
    let text = "> **Status:** Phase 2 intermediate verification in progress.\n\
        > All 12 worker processes are active and responding to pings.";
    assert!(text.chars().count() >= 60);
    assert!(is_deliverable_status_report(text));
    assert!(is_deliverable_rich_report(text));
}

#[test]
fn case_insensitive_keyword_heading_is_deliverable() {
    let text = "### STATUS REPORT: DEPLOYMENT\nTarget host: prod-vds\nResult: pending verification";
    assert!(is_deliverable_status_report(text));
    assert!(is_deliverable_rich_report(text));
}

#[test]
fn thin_narration_is_rejected() {
    assert!(!is_deliverable_status_report("Let me check the logs."));
    assert!(!is_deliverable_rich_report("Let me check the logs."));

    assert!(!is_deliverable_status_report("Running git status"));
    assert!(!is_deliverable_rich_report("Running git status"));
}

#[test]
fn single_line_heading_is_rejected() {
    // Single line heading does not meet the line_count >= 2 requirement
    assert!(!is_deliverable_status_report("# Status Update"));
    assert!(!is_deliverable_rich_report("# Status Update"));
}

#[test]
fn hash_without_space_is_rejected() {
    // #note without space is not an ATX heading
    let text = "#note: status update\nSecond line with some details";
    assert!(!is_deliverable_status_report(text));
    assert!(!is_deliverable_rich_report(text));
}

#[test]
fn short_prose_under_threshold_is_rejected() {
    // Two-line prose containing keyword 'plan' but under 50 total chars
    let text = "Continuing with the plan,\nchecking target dir";
    assert!(text.chars().count() < 50);
    assert!(!is_deliverable_status_report(text));
    assert!(!is_deliverable_rich_report(text));
}

#[test]
fn code_dump_with_hash_lines_inside_fence_is_rejected() {
    // Code fence should prevent # config from being treated as an ATX heading
    let text = "Output:\n```\n# config line with status\n# another line\nvalue = true\n```\n";
    assert!(!is_deliverable_status_report(text));
    assert!(!is_deliverable_rich_report(text));
}

#[test]
fn regressions_mermaid_and_tables_remain_deliverable() {
    let mermaid = "```mermaid\nflowchart TD\n  A --> B\n```";
    assert!(is_deliverable_rich_report(mermaid));

    let table = format!(
        "| Column A | Column B |\n|---|---|\n| Data 1 | Data 2 |\n| {} | {} |",
        "A".repeat(100),
        "B".repeat(100)
    );
    assert!(is_deliverable_rich_report(&table));
}

#[test]
fn regressions_folded_shapes_remain_folded() {
    // Small table under 200 chars remains folded
    assert!(!is_deliverable_rich_report("| a | b |\n|-|-|\n| 1 | 2 |"));

    // Lone-pipe prose remains folded
    assert!(!is_deliverable_rich_report(
        "Here is some text with | pipes | that does not form a real table."
    ));

    // SQL code block remains folded
    let sql = "```sql\nSELECT * FROM users WHERE active = 1;\n```";
    assert!(!is_deliverable_rich_report(sql));

    // Unclosed mermaid fence remains folded
    let unclosed = "```mermaid\nflowchart TD\n  A --> B";
    assert!(!is_deliverable_rich_report(unclosed));
}
