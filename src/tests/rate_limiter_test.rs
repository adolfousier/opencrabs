//! Tests for the global shared rate limiter.
//!
//! Verifies that OpenRouter :free pacing is process-wide, not per-instance —
//! so orchestrator + subagents + team members collectively stay under the
//! provider's rate limit.
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;

use crate::brain::provider::rate_limiter::{
    GlobalRateLimiter, OPENROUTER_FREE_LIMITERS, RateLimiter,
};

// ── First-call-free ──────────────────────────────────────────────────
// A newly created limiter grants the first slot immediately.
// We use a 50 ms interval — the process has been running far longer by the
// time tests execute, so `last_granted = 0` is firmly in the past.

#[tokio::test]
async fn first_request_instant() {
    let limiter = RateLimiter::new(Duration::from_millis(50));
    let start = Instant::now();
    let slept = limiter.wait().await;
    let elapsed = start.elapsed();

    assert_eq!(
        slept,
        Duration::ZERO,
        "first call on fresh limiter should return immediately"
    );
    assert!(
        elapsed < Duration::from_millis(50),
        "wall-clock should also be near-zero (was <10ms, loosened for CI)"
    );
}

// ── Second request paces ─────────────────────────────────────────────

#[tokio::test]
async fn second_request_paces() {
    // Under heavy parallel-test CPU contention the tokio scheduler can
    // delay >90 ms between the first `wait()` returning and the second
    // `Instant::now()` — making the second call see `elapsed >= interval`
    // and skip its sleep, tripping the `>= 90 ms` floor. Widen the gap
    // to 500 ms and also assert on the *returned* sleep duration (what
    // the limiter computed before actually sleeping), which is
    // scheduler-independent.
    let gap = Duration::from_millis(500);
    let limiter = RateLimiter::new(gap);

    limiter.wait().await; // first — instant

    let start = Instant::now();
    let returned = limiter.wait().await; // second — must sleep
    let elapsed = start.elapsed();

    assert!(
        returned >= Duration::from_millis(300),
        "limiter should have computed ≥300 ms sleep, got {:?}",
        returned
    );
    assert!(
        elapsed >= Duration::from_millis(200),
        "wall-clock should be ≥200 ms, got {:?}",
        elapsed
    );
    assert!(
        elapsed < Duration::from_millis(1500),
        "should not have overslept, got {:?}",
        elapsed
    );
}

// ── Multiple Arc<> clones share one budget ───────────────────────────

#[tokio::test]
async fn multiple_arcs_share_state() {
    let limiter = Arc::new(RateLimiter::new(Duration::from_millis(80)));

    let a = Arc::clone(&limiter);
    let b = Arc::clone(&limiter);

    a.wait().await; // slot claimed

    // b must wait because it shares the same AtomicU64
    let start = Instant::now();
    b.wait().await;
    let elapsed = start.elapsed();

    assert!(
        elapsed >= Duration::from_millis(70),
        "shared Arc should enforce the gap, got {:?}",
        elapsed
    );
}

// ── Concurrent callers serialise ─────────────────────────────────────
// 3 concurrent callers share the same limiter. Because tokio::test uses a
// single-threaded runtime for most tests, they run sequentially — but the
// limiter's CAS still guarantees that across all three, at least one full
// gap worth of sleep is accrued (one wins free, the second pays the gap).

#[tokio::test]
async fn concurrent_callers_serialise() {
    let limiter = Arc::new(RateLimiter::new(Duration::from_millis(100)));
    let mut handles = Vec::new();

    let start = Instant::now();
    for _ in 0..3 {
        let lim = limiter.clone();
        handles.push(tokio::spawn(async move { lim.wait().await }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|h| h.unwrap())
        .collect();

    // At least one caller must have slept — the one that lost the first CAS.
    let total_sleep: Duration = results.iter().sum();
    assert!(
        total_sleep >= Duration::from_millis(90),
        "at least one gap's worth of sleep across all callers, got {:?}",
        total_sleep
    );

    // Wall-clock must also reflect at least one gap.
    assert!(
        start.elapsed() >= Duration::from_millis(90),
        "wall-clock >= 90 ms, got {:?}",
        start.elapsed()
    );
}

// ── After sufficient idle time, request is instant ───────────────────

#[tokio::test]
async fn instant_after_idle_gap() {
    let limiter = RateLimiter::new(Duration::from_millis(20));

    limiter.wait().await;
    sleep(Duration::from_millis(40)).await; // wait 2× the interval

    let start = Instant::now();
    let slept = limiter.wait().await;
    assert_eq!(slept, Duration::ZERO, "after 2× idle gap, should be free");
    assert!(
        start.elapsed() < Duration::from_millis(50),
        "after 2× idle gap, should be near-instant"
    );
}

// ── Global OPENROUTER_FREE_LIMITERS is usable ────────────────────────

#[tokio::test]
async fn openrouter_free_static_exists() {
    let limiter = OPENROUTER_FREE_LIMITERS.get("qwen/qwen3.6-plus:free");
    assert!(
        limiter.min_interval >= Duration::from_secs(2),
        "openrouter_free should be ~4 s, got {:?}",
        limiter.min_interval
    );
}

// ── GlobalRateLimiter identity ───────────────────────────────────────
// Same model string → shared Arc. Different model strings → independent
// limiters. These were previously inline `#[cfg(test)]` tests in
// rate_limiter.rs; moved here to keep all tests under src/tests/.

#[tokio::test]
async fn global_limiter_returns_same_limiter_for_same_model() {
    let global = GlobalRateLimiter::new();
    let a = global.get("qwen/qwen3.6-plus:free");
    let b = global.get("qwen/qwen3.6-plus:free");
    assert!(Arc::ptr_eq(&a, &b));
}

#[tokio::test]
async fn global_limiter_returns_different_limiter_for_different_model() {
    let global = GlobalRateLimiter::new();
    let a = global.get("qwen/qwen3.6-plus:free");
    let b = global.get("google/gemma-3-27b-it:free");
    assert!(!Arc::ptr_eq(&a, &b));
}
