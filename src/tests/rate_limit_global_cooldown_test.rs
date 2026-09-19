//! Unit tests for the process-wide Telegram 429 cooldown lock and the
//! `Retry after N` parser (#262).
//!
//! These live here rather than inline in `channels/telegram/rate_limit.rs` so
//! that all tests stay under `src/tests/` per the test-isolation rule.
use std::time::Duration;

use crate::channels::telegram::governor::test_support;
use crate::channels::telegram::rate_limit::{
    clamp_inline_wait, is_global_cooldown_active, parse_retry_after, record_global_429,
    reset_global_cooldown, wait_global_cooldown,
};

#[test]
fn test_parse_retry_after() {
    assert_eq!(
        parse_retry_after("Too Many Requests: retry after 5"),
        Some(Duration::from_secs(5))
    );
    assert_eq!(
        parse_retry_after("Retry after 12 seconds"),
        Some(Duration::from_secs(12))
    );
    assert_eq!(parse_retry_after("Retry after 0"), None);
    assert_eq!(parse_retry_after("Other error"), None);
}

#[test]
fn test_clamp_inline_wait() {
    let (d, capped) = clamp_inline_wait(Duration::from_secs(10));
    assert_eq!(d, Duration::from_secs(10));
    assert!(!capped);

    let (d, capped) = clamp_inline_wait(Duration::from_secs(35));
    assert_eq!(d, Duration::from_secs(30));
    assert!(capped);
}

#[tokio::test]
async fn test_global_429_lock_cooldown() {
    let _guard = test_support::registry_guard().await;
    test_support::reset(0);
    reset_global_cooldown();

    assert!(!is_global_cooldown_active());
    assert_eq!(wait_global_cooldown().await, Duration::ZERO);

    // Record 5s cooldown -> total wait is 5s + 2s margin = 7s
    record_global_429(Duration::from_secs(5));
    assert!(is_global_cooldown_active());

    // Wait should consume remaining and return non-zero (~7s)
    let waited = wait_global_cooldown().await;
    assert!(
        waited >= Duration::from_millis(6900) && waited <= Duration::from_millis(7100),
        "waited {waited:?} expected ~7s"
    );

    // Now virtual clock advanced 7000ms, cooldown should have elapsed
    assert!(!is_global_cooldown_active());
    reset_global_cooldown();
}

#[tokio::test]
async fn test_global_429_lock_extension_monotonic() {
    let _guard = test_support::registry_guard().await;
    test_support::reset(0);
    reset_global_cooldown();

    // 10s cooldown -> 12s total
    record_global_429(Duration::from_secs(10));
    assert!(is_global_cooldown_active());

    // A smaller 3s cooldown shouldn't shorten the 12s deadline
    record_global_429(Duration::from_secs(3));
    let waited = wait_global_cooldown().await;
    assert!(
        waited >= Duration::from_millis(11900) && waited <= Duration::from_millis(12100),
        "waited {waited:?} expected ~12s"
    );

    reset_global_cooldown();
}
