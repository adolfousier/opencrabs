//! Tests for the per-session primary-failure-streak counter that
//! gates fallback stickiness.
//!
//! Regression context (2026-05-30): a transient stream error from
//! `dialagram/qwen-3.7-max-thinking` (provider that closes the
//! socket without `[DONE]`) was triggering immediate permanent
//! fallback to the configured fallback provider. After the
//! `text_looks_complete` fix (commit 97683fb0) most of those
//! errors no longer fire, but for the cases where the fallback
//! DOES engage, the session was getting demoted to the fallback
//! provider on the very first incident — even though the primary
//! recovered on the next request. User intent: "if fallback rescues
//! 3 times consecutively successfully, the 4th it sticks".
//!
//! Superseded in part by #1667: the count is now "rescues inside
//! `STICKY_FALLBACK_WINDOW`", not "rescues in a row". Resetting on any
//! primary success made the threshold unreachable for a flapping
//! provider, which is the case the gate is for.
//!
//! These tests cover the bare counter mechanics. Integration with
//! the actual fallback flow lives in `tool_loop.rs` and is harder
//! to unit-test (requires a real provider + DB); the counter
//! helpers it consumes ARE testable here in isolation.

use crate::brain::agent::service::failure_window::STICKY_FALLBACK_WINDOW;
use crate::tests::agent_service_mocks::create_test_service;

#[tokio::test]
async fn fresh_session_starts_with_zero_streak() {
    let (svc, sid) = create_test_service().await;
    assert_eq!(svc.peek_primary_failure_streak(sid), 0);
}

#[tokio::test]
async fn bump_increments_and_returns_new_count() {
    let (svc, sid) = create_test_service().await;
    assert_eq!(svc.bump_primary_failure_streak(sid), 1);
    assert_eq!(svc.bump_primary_failure_streak(sid), 2);
    assert_eq!(svc.bump_primary_failure_streak(sid), 3);
    assert_eq!(svc.peek_primary_failure_streak(sid), 3);
}

#[tokio::test]
async fn rescues_age_out_of_the_window() {
    // What used to be the job of reset-on-success: history must not
    // accumulate forever. Time does it now, so an outage long past
    // cannot stick the fallback on today's first hiccup.
    let (svc, sid) = create_test_service().await;
    let long_ago = std::time::Instant::now();
    let now = long_ago + STICKY_FALLBACK_WINDOW + std::time::Duration::from_secs(1);

    svc.record_primary_failure_at(sid, long_ago);
    svc.record_primary_failure_at(sid, long_ago);
    svc.record_primary_failure_at(sid, long_ago);
    assert_eq!(svc.peek_primary_failure_streak_at(sid, long_ago), 3);

    assert_eq!(
        svc.peek_primary_failure_streak_at(sid, now),
        0,
        "rescues older than the window must stop counting"
    );
    assert_eq!(
        svc.record_primary_failure_at(sid, now),
        1,
        "a fresh rescue after the window starts over at 1"
    );
}

#[tokio::test]
async fn a_flapping_primary_reaches_the_threshold() {
    // #1667: the counter used to be reset by ANY primary first-try
    // success, so a fail/ok/fail/ok provider oscillated 1, 0, 1, 0 and
    // the threshold was unreachable for exactly the intermittent
    // pattern sticky fallback exists to absorb. The interleaved
    // successes are represented by their absence here: nothing clears
    // the history any more, so four rescues inside the window count.
    let (svc, sid) = create_test_service().await;
    let start = std::time::Instant::now();
    let minute = std::time::Duration::from_secs(60);

    let mut reached = 0;
    for turn in 0..4 {
        // Every other turn the primary succeeded; only the failures
        // are recorded, spaced a minute apart but well inside the window.
        reached = svc.record_primary_failure_at(sid, start + minute * (turn * 2));
    }

    assert!(
        reached >= 4,
        "a primary failing every other turn must still reach the sticky threshold, got {reached}"
    );
}

#[tokio::test]
async fn streak_is_per_session_isolated() {
    let (svc, sid_a) = create_test_service().await;
    let (_svc_b, sid_b) = create_test_service().await;
    // Bump session A only.
    svc.bump_primary_failure_streak(sid_a);
    svc.bump_primary_failure_streak(sid_a);
    assert_eq!(svc.peek_primary_failure_streak(sid_a), 2);
    // Session B (different service instance) — distinct counter.
    // Also confirm even on the SAME service that an unrelated
    // session_id reads as 0.
    let other_sid = uuid::Uuid::new_v4();
    assert_eq!(svc.peek_primary_failure_streak(other_sid), 0);
    assert_eq!(svc.peek_primary_failure_streak(sid_b), 0);
}

#[tokio::test]
async fn remove_session_provider_also_clears_streak() {
    // When a session is deleted (e.g. user cleared history) the
    // streak counter must clear with it; otherwise a future
    // session that happened to reuse the same UUID would inherit
    // a phantom count.
    let (svc, sid) = create_test_service().await;
    svc.bump_primary_failure_streak(sid);
    svc.bump_primary_failure_streak(sid);
    svc.bump_primary_failure_streak(sid);
    assert_eq!(svc.peek_primary_failure_streak(sid), 3);
    svc.remove_session_provider(sid);
    assert_eq!(svc.peek_primary_failure_streak(sid), 0);
}

#[tokio::test]
async fn threshold_value_is_four() {
    // Sentinel: the user-stated intent was "3 consecutive
    // rescues, the 4th sticks". Encode that as a numeric assertion
    // so a future refactor that bumps the constant has to update
    // this test deliberately rather than silently changing UX.
    let (svc, sid) = create_test_service().await;
    let mut count = 0;
    while count < 4 {
        count = svc.bump_primary_failure_streak(sid);
    }
    assert_eq!(
        count, 4,
        "stickiness must engage on the 4th consecutive rescue"
    );
}
