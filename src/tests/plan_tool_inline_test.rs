//! Plan tool.
//!
//! Moved out of `src/brain/tools/plan_tool.rs`: tests live under `src/tests/`,
//! never inline beside the logic they exercise (#1076).

use crate::brain::tools::epistemic::{Belief, Confidence, Source};
use crate::brain::tools::plan_tool::*;
use chrono::Utc;
use uuid::Uuid;

/// Belief carrying only what the flag renderer reads.
fn flag(key: &str, value: &str, confidence: Confidence) -> Belief {
    let now = Utc::now();
    Belief {
        key: key.to_string(),
        value: value.to_string(),
        confidence,
        source: Source {
            origin: "test".to_string(),
            recorded_at: now,
            last_verified: now,
        },
        notes: None,
        hits: 0,
        last_used: None,
    }
}

#[test]
fn outcome_confidence_mapping() {
    assert_eq!(task_outcome_confidence("completed"), Confidence::Verified);
    assert_eq!(task_outcome_confidence("failed"), Confidence::Contradicted);
    assert_eq!(task_outcome_confidence("skipped"), Confidence::Uncertain);
}

#[test]
fn outcome_key_stable_across_retries() {
    let plan = Uuid::new_v4();
    let k1 = task_outcome_key(plan, 3, "Fix the parser");
    let k2 = task_outcome_key(plan, 3, "Fix the parser");
    assert_eq!(k1, k2);
}

#[test]
fn outcome_key_differs_by_task() {
    let plan = Uuid::new_v4();
    assert_ne!(
        task_outcome_key(plan, 3, "Fix the parser"),
        task_outcome_key(plan, 4, "Fix the parser")
    );
    assert_ne!(
        task_outcome_key(plan, 3, "Fix the parser"),
        task_outcome_key(plan, 3, "Write the docs")
    );
}

/// #1083: two plans (e.g. created and completed sequentially in ONE session)
/// with an identically titled task at the same order must NOT share a key —
/// sharing made one plan's outcome contradict the other's.
#[test]
fn outcome_key_differs_by_plan() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    assert_ne!(
        task_outcome_key(a, 1, "Run tests"),
        task_outcome_key(b, 1, "Run tests")
    );
}

/// #1083: every key of a plan sits under that plan's prefix, and no other
/// plan's prefix matches it.
#[test]
fn outcome_key_lives_under_its_plan_prefix() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let key = task_outcome_key(a, 2, "Wire the gate");
    assert!(key.starts_with(&plan_belief_prefix(a)));
    assert!(!key.starts_with(&plan_belief_prefix(b)));
}

#[test]
fn flags_empty_when_nothing_actionable() {
    let plan = Uuid::new_v4();
    let beliefs = vec![
        flag(
            &task_outcome_key(plan, 1, "a"),
            "completed",
            Confidence::Verified,
        ),
        flag(
            &task_outcome_key(plan, 2, "b"),
            "likely",
            Confidence::Inferred,
        ),
    ];
    assert!(render_epistemic_flags(&beliefs, plan, 1).is_empty());
}

#[test]
fn flags_keep_contradicted_and_uncertain_only() {
    let plan = Uuid::new_v4();
    let beliefs = vec![
        flag(
            &task_outcome_key(plan, 1, "a"),
            "failed",
            Confidence::Contradicted,
        ),
        flag(
            &task_outcome_key(plan, 2, "b"),
            "skipped",
            Confidence::Uncertain,
        ),
        flag(
            &task_outcome_key(plan, 3, "c"),
            "completed",
            Confidence::Verified,
        ),
    ];
    let block = render_epistemic_flags(&beliefs, plan, 1);
    assert!(block.contains("Epistemic flags (2)"), "block: {block}");
    assert!(block.contains("failed"));
    assert!(block.contains("skipped"));
    assert!(!block.contains("completed"));
}

/// #1083 landmine: an uncapped list dumps every accumulated flag into every
/// brief. The cap holds and reports what it suppressed.
#[test]
fn flags_are_capped_and_report_the_remainder() {
    let plan = Uuid::new_v4();
    let beliefs: Vec<Belief> = (1..=MAX_EPISTEMIC_FLAGS + 3)
        .map(|i| {
            flag(
                &task_outcome_key(plan, i, &format!("task {i}")),
                &format!("failed {i}"),
                Confidence::Contradicted,
            )
        })
        .collect();
    let block = render_epistemic_flags(&beliefs, plan, 1);
    assert!(
        block.contains(&format!("Epistemic flags ({})", MAX_EPISTEMIC_FLAGS + 3)),
        "total count is reported honestly: {block}"
    );
    assert_eq!(
        block.matches("  ⚠ [").count(),
        MAX_EPISTEMIC_FLAGS,
        "only the cap is rendered: {block}"
    );
    assert!(block.contains("3 more suppressed"), "block: {block}");
}

/// The task about to start is the only flag guaranteed relevant, so it must
/// survive the cap even when it was recorded last.
#[test]
fn flags_put_the_current_task_first() {
    let plan = Uuid::new_v4();
    let mut beliefs: Vec<Belief> = (1..=MAX_EPISTEMIC_FLAGS)
        .map(|i| {
            flag(
                &task_outcome_key(plan, i, &format!("task {i}")),
                "failed elsewhere",
                Confidence::Contradicted,
            )
        })
        .collect();
    let mine = task_outcome_key(plan, 99, "my task");
    beliefs.push(flag(&mine, "failed here", Confidence::Contradicted));

    let block = render_epistemic_flags(&beliefs, plan, 99);
    assert!(
        block.contains(&mine),
        "current task's flag survives: {block}"
    );
    let first_line = block
        .lines()
        .find(|l| l.starts_with("  ⚠ ["))
        .unwrap_or_default();
    assert!(first_line.contains(&mine), "first line: {first_line}");
}
