/// Tests for the global shared rate limiter.
///
/// Verifies that OpenRouter :free pacing is process-wide, not per-instance —
/// so orchestrator + subagents + team members collectively stay under the
/// provider's rate limit.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;

use crate::brain::provider::rate_limiter::{OPENROUTER_FREE_LIMITER, RateLimiter};

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

    assert_eq!(slept, Duration::ZERO, "first call on fresh limiter should return immediately");
    assert!(elapsed < Duration::from_millis(10), "wall-clock should also be near-zero");
}

// ── Second request paces ─────────────────────────────────────────────

#[tokio::test]
async fn second_request_paces() {
    let gap = Duration::from_millis(100);
    let limiter = RateLimiter::new(gap);

    limiter.wait().await; // first — instant

    let start = Instant::now();
    limiter.wait().await; // second — must sleep ~100 ms
    let elapsed = start.elapsed();

    assert!(
        elapsed >= Duration::from_millis(90),
        "should have slept at least 90 ms, got {:?}",
        elapsed
    );
    assert!(
        elapsed < Duration::from_millis(200),
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
    assert!(start.elapsed() < Duration::from_millis(5));
}

// ── Global OPENROUTER_FREE_LIMITER is usable ────────────────────────

#[tokio::test]
async fn openrouter_free_static_exists() {
    let limiter = OPENROUTER_FREE_LIMITER.as_ref();
    assert!(limiter.min_interval >= Duration::from_secs(2),
            "openrouter_free should be ~3 s, got {:?}", limiter.min_interval);
}
