//! Tests for the goal driver line (#480): the status surfaces must not claim a
//! driver that does not exist.

use crate::brain::goal::driver::driver_line;
use crate::db::models::GoalState;
use chrono::{Duration, Utc};

/// A goal row in the shape the defect was reported on: `active`, never
/// evaluated, so `updated_at` still equals `created_at`.
fn armed_never_evaluated() -> GoalState {
    let mut goal = GoalState::new(uuid::Uuid::new_v4(), "test goal".into(), None, None, Some(20));
    goal.state = "active".into();
    goal.turns_used = 0;
    goal
}

#[test]
fn active_goal_never_evaluated_does_not_claim_a_driver() {
    let line = driver_line(&armed_never_evaluated());
    assert!(
        line.contains("never evaluated"),
        "expected an explicit never-evaluated marker, got: {line}"
    );
    assert!(
        line.contains("not driving"),
        "expected the driverless qualifier on an active goal, got: {line}"
    );
    // The raw segment the defect was about must not survive on its own.
    assert_ne!(
        line, "Turns: 0/20",
        "the bare counter must not be the whole line"
    );
}

#[test]
fn evaluated_goal_reports_last_activity_age() {
    let mut goal = armed_never_evaluated();
    goal.turns_used = 3;
    goal.updated_at = (Utc::now() - Duration::minutes(5)).to_rfc3339();
    let line = driver_line(&goal);
    assert!(line.contains("Turns: 3/20"), "counter preserved: {line}");
    assert!(line.contains("last activity"), "age reported: {line}");
    assert!(line.contains("ago"), "age is relative: {line}");
    assert!(
        !line.contains("never evaluated"),
        "an evaluated goal is not never-evaluated: {line}"
    );
}

#[test]
fn stale_active_goal_warns_that_nothing_will_re_evaluate_it() {
    let mut goal = armed_never_evaluated();
    goal.turns_used = 1;
    goal.updated_at = (Utc::now() - Duration::hours(2)).to_rfc3339();
    let line = driver_line(&goal);
    assert!(line.contains("⚠️"), "stale active goal warns: {line}");
    assert!(
        line.contains("nothing re-evaluates"),
        "warning names the mechanism: {line}"
    );
}

#[test]
fn stale_paused_goal_is_not_warned_about_a_driver() {
    // A paused goal is deliberately not driven, so the warning would be noise.
    let mut goal = armed_never_evaluated();
    goal.state = "paused".into();
    goal.turns_used = 1;
    goal.updated_at = (Utc::now() - Duration::hours(2)).to_rfc3339();
    let line = driver_line(&goal);
    assert!(
        !line.contains("⚠️"),
        "paused goals carry no driver warning: {line}"
    );
}

#[test]
fn unparseable_timestamp_omits_the_age_rather_than_guessing() {
    let mut goal = armed_never_evaluated();
    goal.turns_used = 2;
    goal.updated_at = "not-a-timestamp".into();
    let line = driver_line(&goal);
    assert!(line.contains("Turns: 2/20"), "counter preserved: {line}");
    assert!(!line.contains("ago"), "no confident age from a bad stamp: {line}");
}

#[test]
fn humanize_age_boundaries_are_stable() {
    use crate::utils::string::humanize_age;
    assert_eq!(humanize_age(0), "<1m");
    assert_eq!(humanize_age(59), "<1m");
    assert_eq!(humanize_age(60), "1m");
    assert_eq!(humanize_age(3_600), "1h 0m");
    assert_eq!(humanize_age(86_400), "1d 0h");
    // Negative input must not render a future age.
    assert_eq!(humanize_age(-5), "<1m");
}
