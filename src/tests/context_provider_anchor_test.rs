//! The context budget follows the provider's count, not our estimate.
//!
//! `token_count` is a tiktoken estimate of the system prompt plus messages.
//! It omits the tool schemas the provider also receives and disagrees with
//! their tokenizer on code and JSON. A 2.4MB Claude CLI request we estimated
//! at ~660k was counted by the provider as ~1.03M, against a 1M limit we
//! believed we were at 66% of, so compaction fired eight times and never got
//! under a ceiling that was never where we thought it was.

use uuid::Uuid;

use crate::brain::agent::context::AgentContext;
use crate::brain::provider::Message;

fn ctx(max_tokens: usize) -> AgentContext {
    AgentContext::new(Uuid::new_v4(), max_tokens)
}

#[test]
fn without_a_report_the_estimate_still_governs() {
    let mut c = ctx(1_000);
    c.token_count = 400;
    assert_eq!(c.effective_token_count(), 400, "first turn is unchanged");
    assert!((c.usage_percentage() - 40.0).abs() < f64::EPSILON);
}

#[test]
fn a_reported_count_replaces_the_estimate() {
    let mut c = ctx(1_000_000);
    c.token_count = 660_000;
    assert!(
        c.usage_percentage() < 66.5,
        "the estimate alone reads as two thirds full"
    );

    c.record_provider_reported_tokens(1_026_909);

    assert_eq!(c.effective_token_count(), 1_026_909);
    assert!(
        c.usage_percentage() > 100.0,
        "the request the provider actually received is over the limit, and the \
         budget must say so instead of reporting 66%"
    );
}

#[test]
fn appending_after_the_anchor_raises_the_budget() {
    let mut c = ctx(1_000_000);
    c.token_count = 600_000;
    c.record_provider_reported_tokens(900_000);

    let before = c.effective_token_count();
    c.add_message(Message::user("some more conversation".to_string()));
    let grew_by = c.effective_token_count() - before;

    assert!(grew_by > 0, "growth after the anchor must be counted");
    assert_eq!(
        grew_by,
        c.token_count - 600_000,
        "it grows by our estimate of what was added, the only measure we have \
         until the provider reports again"
    );
}

#[test]
fn trimming_after_the_anchor_lowers_the_budget() {
    // The failure this guards: an anchor that only ever grew would keep
    // reporting a full context after compaction freed it, and every following
    // turn would compact again against a number that could not come down.
    let mut c = ctx(1_000_000);
    c.token_count = 900_000;
    c.record_provider_reported_tokens(1_000_000);

    c.token_count = 300_000; // as compaction or truncation would leave it

    assert_eq!(
        c.effective_token_count(),
        400_000,
        "the provider's count moves down by everything we removed"
    );
    assert!(c.usage_percentage() < 65.0, "and the turn stops compacting");
}

#[test]
fn a_later_report_re_anchors_on_fresh_truth() {
    let mut c = ctx(1_000_000);
    c.token_count = 500_000;
    c.record_provider_reported_tokens(800_000);
    c.token_count = 520_000;

    c.record_provider_reported_tokens(850_000);

    assert_eq!(
        c.effective_token_count(),
        850_000,
        "the newest count stands on its own, with no drift carried over"
    );
}

#[test]
fn the_delta_cannot_underflow_past_zero() {
    let mut c = ctx(1_000_000);
    c.token_count = 100;
    c.record_provider_reported_tokens(50);
    c.token_count = 0;
    assert_eq!(c.effective_token_count(), 0);
}

#[test]
fn compaction_clears_provider_anchor() {
    let mut c = ctx(200_000);
    c.token_count = 125_000;
    // Provider reported 138k against 125k local estimate (+13k delta).
    c.record_provider_reported_tokens(138_000);
    assert_eq!(c.effective_token_count(), 138_000);

    // Context compacts with a summary down to ~67k tokens.
    c.compact_with_summary("Summary of conversation".to_string(), 0);

    // Stale provider anchor must be dropped so post-compaction budget
    // matches local token count exactly (#211).
    assert_eq!(c.provider_anchor, None);
    assert_eq!(c.effective_token_count(), c.token_count);
    assert!(c.token_count < 100); // 0 budget + short summary
    assert_eq!(c.effective_token_count(), c.token_count);
}
