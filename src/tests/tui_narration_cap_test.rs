//! Narration cap: intermediate working-out rows render as bounded excerpts
//! when their turn is expanded, so a long agentic run can never repaint the
//! transcript as a wall of narration (the 2026-09-26 overnight-wall report).
//! Folded frames hide these rows entirely (existing #758 behaviour); the cap
//! exists for the expanded frame.

use crate::tui::app::DisplayMessage;
use crate::tui::render::chat::{cap_narration_lines, turn_narration_rows, turn_ranges};
use ratatui::text::Line;
use uuid::Uuid;

fn msg(role: &str, content: &str) -> DisplayMessage {
    DisplayMessage {
        id: Uuid::new_v4(),
        role: role.to_string(),
        content: content.to_string(),
        timestamp: chrono::Utc::now(),
        token_count: None,
        cost: None,
        approval: None,
        approve_menu: None,
        details: None,
        expanded: false,
        expanded_full: false,
        tool_group: None,
        duration_secs: None,
    }
}

fn lines(n: usize) -> Vec<Line<'static>> {
    (0..n).map(|i| Line::from(format!("line {i}"))).collect()
}

fn text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.to_string()).collect()
}

#[test]
fn cap_at_or_under_ceiling_is_byte_identical() {
    for n in 0..=3 {
        let input = lines(n);
        let out = cap_narration_lines(input.clone());
        assert_eq!(out.len(), n, "no cap applies at or under the ceiling");
        for (a, b) in input.iter().zip(out.iter()) {
            assert_eq!(text(a), text(b));
        }
    }
}

#[test]
fn cap_over_ceiling_truncates_and_counts_hidden_lines() {
    let out = cap_narration_lines(lines(9));
    assert_eq!(out.len(), 4, "3 excerpt lines + 1 explicit marker");
    assert_eq!(text(&out[0]), "line 0");
    assert_eq!(text(&out[2]), "line 2");
    let marker = text(&out[3]);
    assert!(
        marker.contains("+6 more lines hidden"),
        "marker must state the suppressed count, got: {marker}"
    );
    assert!(
        marker.contains("intermediate narration"),
        "marker must name the suppression, not look like eaten content"
    );
}

#[test]
fn narration_rows_catch_prose_but_never_user_final_or_errors() {
    let messages = vec![
        msg("user", "run the release"),
        msg("assistant", "checking the tags now"),
        msg("assistant", "tags look fine, building"),
        msg("error", "tool failed"),
        msg("system", "model switched"),
        msg("assistant", "done, everything is green"),
    ];
    let turns = turn_ranges(&messages);
    assert_eq!(turns.len(), 1);
    // Final answer = last row (index 5).
    let rows = turn_narration_rows(&messages, turns[0], Some(5));
    assert!(rows.contains(&1), "intermediate prose is working-out");
    assert!(rows.contains(&2), "intermediate prose is working-out");
    assert!(!rows.contains(&0), "user rows are never working-out");
    assert!(!rows.contains(&5), "the final answer is never working-out");
    assert!(!rows.contains(&3), "errors must stay fully visible");
    assert!(!rows.contains(&4), "system status must stay fully visible");
}

#[test]
fn narration_rows_include_thinking_only_rows() {
    let messages = vec![
        msg("user", "go"),
        msg("assistant", ""), // thinking-only: reasoning lives in details
        msg("assistant", "result"),
    ];
    let turns = turn_ranges(&messages);
    let rows = turn_narration_rows(&messages, turns[0], Some(2));
    assert!(rows.contains(&1), "thinking-only rows are working-out too");
}

#[test]
fn narration_rows_exclude_deliverable_reports() {
    // >200 chars and a real GFM table row: the #904 deliverable marker.
    let mut table = String::from("Audit results:\n\n| col | val |\n|---|---|\n");
    for i in 0..12 {
        table.push_str(&format!("| row {i} | some longer cell content here |\n"));
    }
    assert!(table.chars().count() > 200);
    let messages = vec![
        msg("user", "audit it"),
        msg("assistant", &table), // deliverable, not narration
        msg("assistant", "closing remark"),
    ];
    let turns = turn_ranges(&messages);
    let rows = turn_narration_rows(&messages, turns[0], Some(2));
    assert!(
        !rows.contains(&1),
        "a deliverable report is content, never working-out"
    );
}
