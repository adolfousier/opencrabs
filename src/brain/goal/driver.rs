//! The goal driver line (#480): one shared derivation of what the turn
//! counter means for the driver, so the two operator status surfaces
//! (`goal_manage status` and `/goal`) cannot drift apart.
//!
//! A goal is a CONTINUATION mechanism, not a driver. The only thing that
//! advances it — `GoalManager::evaluate_after_turn` — runs at turn END, and
//! nothing in the codebase starts a turn for a goal. An idle session holding
//! an `active` goal is therefore inert while its state reads as driven, and
//! the raw `Turns: 0/20` segment is what makes it read that way.

use crate::db::models::GoalState;
use crate::utils::string::humanize_age;

/// An `active` goal with no goal activity for longer than this is reported as
/// parked rather than merely busy. Set above the longest observed turn (a
/// full CI gate runs ~25 min), so the warning fires on a lane that has
/// genuinely stopped instead of one that is mid-turn.
pub const DRIVER_STALE_AFTER_SECS: i64 = 3600;

/// The `Turns: n/max` segment of the goal status line, qualified with what
/// the counter means for the driver.
///
/// `turns_used` is the discriminator: its only writer is the evaluation
/// increment, so `0` means no evaluation has ever run — the goal is armed but
/// nothing is driving it.
///
/// The age comes from `updated_at`, which is the last goal WRITE and not a
/// last-evaluation stamp: `set_state` moves it on pause/resume with no
/// evaluation behind it. It is reported as "last activity" for that reason —
/// calling it "last evaluated" would be the same class of lie this module
/// exists to remove, pointed the other way.
pub fn driver_line(goal: &GoalState) -> String {
    let turns = format!("Turns: {}/{}", goal.turns_used, goal.max_turns);
    if goal.turns_used == 0 {
        return match goal.state.as_str() {
            "active" => format!(
                "{turns} — never evaluated; armed but not driving \
                 (nothing starts a turn for a goal)"
            ),
            _ => format!("{turns} — never evaluated"),
        };
    }
    match age_secs(&goal.updated_at) {
        Some(age) if goal.state == "active" && age > DRIVER_STALE_AFTER_SECS => format!(
            "{turns} — ⚠️ no goal activity for {}; nothing re-evaluates this \
             goal until a turn runs here",
            humanize_age(age)
        ),
        Some(age) => format!("{turns} — last activity {} ago", humanize_age(age)),
        None => turns,
    }
}

/// Seconds since an RFC3339 stamp, or `None` when it does not parse: an
/// unreadable stamp is omitted rather than rendered as a confident age.
fn age_secs(stamp: &str) -> Option<i64> {
    let then = chrono::DateTime::parse_from_rfc3339(stamp).ok()?;
    Some(
        chrono::Utc::now()
            .signed_duration_since(then)
            .num_seconds()
            .max(0),
    )
}
