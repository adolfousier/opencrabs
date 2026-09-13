//! Regression test for Telegram long rate-limit guard (#1110).
//!
//! When Telegram returns `Retry-After: N` where N > 1 hour, the chat is
//! flood-banned for hours (28442s = 7.9 hours observed in Adi's audit).
//! Retrying the send ladder burns 90 seconds (3 × 30s clamped wait) for
//! no gain. This test pins the fix: long rate-limits bail immediately.
//!
//! The two retrying tests run on a paused clock (#1532). They used to sleep
//! those 90 seconds for real — between them roughly 87% of the suite's wall
//! clock, and both tripped the harness 60-second warning. Under
//! `start_paused` tokio auto-advances whenever nothing is runnable, so the
//! waits resolve instantly and `Instant::elapsed` still reports the virtual
//! 90s. That turns the wait from a cost into an assertion: measuring it is
//! what pins the #1064 cap, since attempt counts alone cannot tell a capped
//! 30s wait from the full uncapped window.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use teloxide::RequestError;
use teloxide::types::Seconds;

/// Clamped wait per retry (`MAX_INLINE_RATE_LIMIT_WAIT`) times `MAX_RETRIES`.
/// Named so the two expectations below read as the same rule, not as two
/// coincidentally equal numbers.
const LADDER_WAIT: Duration = Duration::from_secs(90);

/// Long rate-limit (>1 hour) bails immediately without retrying.
#[tokio::test]
async fn long_rate_limit_bails_immediately() {
    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_clone = attempts.clone();

    // Mock a send that always returns a 7.9-hour rate-limit
    let result = crate::channels::telegram::intermediates::send_retrying_rate_limit(
        "test send",
        move || {
            let attempts = attempts_clone.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                // 28442 seconds = 7.9 hours (observed in Adi's audit)
                Err::<(), _>(RequestError::RetryAfter(Seconds::from_seconds(28442)))
            }
        },
    )
    .await;

    // Should fail immediately without retrying
    assert!(result.is_err(), "Long rate-limit should return error");
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "Long rate-limit should bail after 1 attempt, not retry"
    );
}

/// Short rate-limit (<1 hour) retries normally up to 3 attempts.
#[tokio::test(start_paused = true)]
async fn short_rate_limit_retries_normally() {
    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_clone = attempts.clone();
    let start = tokio::time::Instant::now();

    // Mock a send that returns a 30-second rate-limit 3 times, then succeeds
    let result = crate::channels::telegram::intermediates::send_retrying_rate_limit(
        "test send",
        move || {
            let attempts = attempts_clone.clone();
            async move {
                let count = attempts.fetch_add(1, Ordering::SeqCst);
                if count < 3 {
                    // 30 seconds (typical flood window)
                    Err::<(), _>(RequestError::RetryAfter(Seconds::from_seconds(30)))
                } else {
                    Ok(())
                }
            }
        },
    )
    .await;

    // Should succeed after retries
    assert!(
        result.is_ok(),
        "Short rate-limit should succeed after retries"
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        4,
        "Short rate-limit should make 4 attempts (1 initial + 3 retries)"
    );
    // A 30s window sits exactly ON the inline cap, so it is waited in full
    // rather than clamped. Same 90s total as the capped case below, reached
    // the other way round.
    assert_eq!(
        start.elapsed(),
        LADDER_WAIT,
        "three 30s windows waited in full"
    );
}

/// Rate-limit at exactly 1 hour (3600s) is NOT long yet (boundary check).
#[tokio::test(start_paused = true)]
async fn rate_limit_at_threshold_retries() {
    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_clone = attempts.clone();
    let start = tokio::time::Instant::now();

    // Mock a send that returns exactly 3600s (1 hour) rate-limit
    let result = crate::channels::telegram::intermediates::send_retrying_rate_limit(
        "test send",
        move || {
            let attempts = attempts_clone.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                // Exactly 1 hour (at threshold, not over)
                Err::<(), _>(RequestError::RetryAfter(Seconds::from_seconds(3600)))
            }
        },
    )
    .await;

    // Should retry (not bail immediately) because it's AT threshold, not OVER
    assert!(result.is_err(), "Should fail after exhausting retries");
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        4,
        "Rate-limit at threshold should make 4 attempts (1 initial + 3 retries)"
    );
    // The assertion the attempt count cannot make: each 3600s window is
    // clamped to the 30s cap, so the ladder costs 90s and not 3x3600s. Drop
    // the clamp and the count above still passes while this fails (#1064).
    assert_eq!(
        start.elapsed(),
        LADDER_WAIT,
        "three 3600s windows clamped to the inline cap, not slept in full"
    );
}
