//! Tests for the `/skills` dialog — filter behaviour, navigation,
//! and the keystroke contract.
//!
//! Drives the pure `decide` fn against a `SkillsDialogState` directly
//! — no full `App` needed.

use crate::brain::skills::{Skill, SkillSource};
use crate::tui::app::skills_dialog::input::{KeyOutcome, decide};
use crate::tui::app::skills_dialog::{SkillsDialogState, matching};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn skill(name: &str, description: &str, body: &str, source: SkillSource) -> Skill {
    Skill {
        name: name.to_string(),
        slash_name: format!("/{name}"),
        description: description.to_string(),
        body: body.to_string(),
        globs: Vec::new(),
        review_gate: false,
        source,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn ctrl_key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

// ── Filter behaviour ────────────────────────────────────────────────────────

#[test]
fn empty_filter_returns_every_skill() {
    let skills = vec![
        skill("a", "first", "body a", SkillSource::Builtin),
        skill("b", "second", "body b", SkillSource::User),
    ];
    let visible = matching(&skills, "");
    assert_eq!(visible.len(), 2);
}

#[test]
fn filter_matches_substring_of_name() {
    let skills = vec![
        skill(
            "security-audit",
            "comprehensive audit",
            "body",
            SkillSource::Builtin,
        ),
        skill("cost-estimate", "valuation", "body", SkillSource::Builtin),
    ];
    let visible = matching(&skills, "secur");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].name, "security-audit");
}

#[test]
fn filter_is_case_insensitive() {
    let skills = vec![skill(
        "Security-Audit",
        "comprehensive audit",
        "body",
        SkillSource::Builtin,
    )];
    let visible = matching(&skills, "security");
    assert_eq!(visible.len(), 1);
}

#[test]
fn filter_matches_description_substring() {
    let skills = vec![
        skill(
            "foo",
            "estimate cost-to-build for the codebase",
            "body",
            SkillSource::Builtin,
        ),
        skill("bar", "completely unrelated", "body", SkillSource::Builtin),
    ];
    let visible = matching(&skills, "cost-to-build");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].name, "foo");
}

// ── Type-to-filter keystroke flow ──────────────────────────────────────────

#[test]
fn typing_a_char_appends_to_filter_and_resets_selection() {
    let mut s = SkillsDialogState {
        selected_index: 4,
        ..Default::default()
    };
    let out = decide(&mut s, &[], key(KeyCode::Char('a')));
    assert_eq!(out, KeyOutcome::Consumed);
    assert_eq!(s.filter, "a");
    assert_eq!(
        s.selected_index, 0,
        "selection must reset to top when filter changes"
    );
}

#[test]
fn backspace_pops_last_char_and_resets_selection() {
    let mut s = SkillsDialogState {
        filter: "abc".to_string(),
        selected_index: 3,
        ..Default::default()
    };
    decide(&mut s, &[], key(KeyCode::Backspace));
    assert_eq!(s.filter, "ab");
    assert_eq!(s.selected_index, 0);
}

#[test]
fn ctrl_chars_are_not_consumed_as_filter_input() {
    let mut s = SkillsDialogState::default();
    let out = decide(&mut s, &[], ctrl_key('c'));
    assert_eq!(out, KeyOutcome::NotConsumed);
    assert!(
        s.filter.is_empty(),
        "Ctrl-C should never end up in the filter buffer"
    );
}

// ── Navigation ──────────────────────────────────────────────────────────────

#[test]
fn tab_advances_selection_within_filtered_count() {
    let skills = vec![
        skill("a", "x", "b", SkillSource::Builtin),
        skill("b", "x", "b", SkillSource::Builtin),
        skill("c", "x", "b", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState::default();
    decide(&mut s, &skills, key(KeyCode::Tab));
    assert_eq!(s.selected_index, 1);
    decide(&mut s, &skills, key(KeyCode::Down));
    assert_eq!(s.selected_index, 2);
}

#[test]
fn down_at_last_wraps_to_first() {
    let skills = vec![
        skill("a", "x", "b", SkillSource::Builtin),
        skill("b", "x", "b", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState {
        selected_index: 1,
        ..Default::default()
    };
    decide(&mut s, &skills, key(KeyCode::Down));
    assert_eq!(s.selected_index, 0, "Down at last should wrap to first");
}

#[test]
fn tab_at_last_wraps_to_first() {
    let skills = vec![
        skill("a", "x", "b", SkillSource::Builtin),
        skill("b", "x", "b", SkillSource::Builtin),
        skill("c", "x", "b", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState {
        selected_index: 2,
        ..Default::default()
    };
    decide(&mut s, &skills, key(KeyCode::Tab));
    assert_eq!(s.selected_index, 0);
}

#[test]
fn up_at_first_wraps_to_last() {
    let skills = vec![
        skill("a", "x", "b", SkillSource::Builtin),
        skill("b", "x", "b", SkillSource::Builtin),
        skill("c", "x", "b", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState::default();
    decide(&mut s, &skills, key(KeyCode::Up));
    assert_eq!(s.selected_index, 2, "Up at 0 should wrap to last");
}

#[test]
fn back_tab_at_first_wraps_to_last() {
    let skills = vec![
        skill("a", "x", "b", SkillSource::Builtin),
        skill("b", "x", "b", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState::default();
    decide(&mut s, &skills, key(KeyCode::BackTab));
    assert_eq!(s.selected_index, 1);
}

#[test]
fn back_tab_goes_backward() {
    let skills = vec![
        skill("a", "x", "b", SkillSource::Builtin),
        skill("b", "x", "b", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState {
        selected_index: 1,
        ..Default::default()
    };
    decide(&mut s, &skills, key(KeyCode::BackTab));
    assert_eq!(s.selected_index, 0);
}

#[test]
fn navigation_uses_filtered_count_not_total() {
    // Total of 3 skills, filter narrows to 1 match. Down should clamp
    // at index 0, not advance into the filtered-out entries.
    let skills = vec![
        skill("alpha", "x", "b", SkillSource::Builtin),
        skill("beta", "x", "b", SkillSource::Builtin),
        skill("gamma", "x", "b", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState {
        filter: "beta".to_string(),
        ..Default::default()
    };
    decide(&mut s, &skills, key(KeyCode::Down));
    assert_eq!(s.selected_index, 0, "single match — selection stays pinned");
}

// ── Enter & Esc ─────────────────────────────────────────────────────────────

#[test]
fn enter_executes_selected_skill_with_its_body() {
    let skills = vec![
        skill("audit", "x", "BODY-A", SkillSource::Builtin),
        skill("estimate", "x", "BODY-B", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState::default();
    decide(&mut s, &skills, key(KeyCode::Tab));
    let out = decide(&mut s, &skills, key(KeyCode::Enter));
    assert_eq!(out, KeyOutcome::Execute("BODY-B".to_string()));
}

#[test]
fn enter_with_empty_filtered_list_is_consumed_silently() {
    let skills: Vec<Skill> = Vec::new();
    let mut s = SkillsDialogState::default();
    let out = decide(&mut s, &skills, key(KeyCode::Enter));
    assert_eq!(out, KeyOutcome::Consumed);
}

#[test]
fn esc_returns_close() {
    let mut s = SkillsDialogState::default();
    let out = decide(&mut s, &[], key(KeyCode::Esc));
    assert_eq!(out, KeyOutcome::Close);
}

// ── Auto-focus on unique match ──────────────────────────────────────────────

#[test]
fn typing_until_one_match_leaves_first_index_focused_and_executable() {
    // The "auto-focus on unique match" guarantee: as the filter
    // narrows to a single skill, that skill is at index 0 of the
    // filtered list and Enter fires it.
    let skills = vec![
        skill("security-audit", "x", "AUDIT-BODY", SkillSource::Builtin),
        skill("cost-estimate", "x", "ESTIMATE-BODY", SkillSource::Builtin),
    ];
    let mut s = SkillsDialogState::default();
    for c in "secur".chars() {
        decide(&mut s, &skills, key(KeyCode::Char(c)));
    }
    let visible = matching(&skills, &s.filter);
    assert_eq!(visible.len(), 1);
    let out = decide(&mut s, &skills, key(KeyCode::Enter));
    assert_eq!(out, KeyOutcome::Execute("AUDIT-BODY".to_string()));
}
