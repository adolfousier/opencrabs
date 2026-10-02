//! Tests for `send_retrying_rate_limit` (#297): command replies must wait
//! out Telegram's RetryAfter (429) and retry instead of dropping. A per-chat
//! rate limit (typically a streaming turn editing its placeholder in the
//! same chat) may DELAY a programmatic reply, never lose it.
//!
//! #1064: the inline wait is capped at
//! [`crate::channels::telegram::rate_limit::MAX_INLINE_RATE_LIMIT_WAIT`] so a
//! multi-hour flood-ban window (8288s observed) cannot park the agent turn
//! for hours. Small windows keep #297 semantics unchanged.

use crate::channels::telegram::governor::test_support;
use crate::channels::telegram::handler::send_retrying_rate_limit;
use std::cell::Cell;
use std::time::Duration;
use teloxide::types::Seconds;

fn rate_limited<T>() -> Result<T, teloxide::RequestError> {
    // 0 seconds so tests do not sleep for real.
    Err(teloxide::RequestError::RetryAfter(Seconds::from_seconds(0)))
}

#[tokio::test]
async fn waits_out_rate_limits_then_delivers() {
    // #1832: `wait_out` arms the process-wide GLOBAL_COOLDOWN and advances
    // the shared virtual clock, so this must not run beside guarded tests.
    let _guard = test_support::registry_guard().await;
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let calls = Cell::new(0u32);
    let out = send_retrying_rate_limit("test send", || {
        let n = calls.get() + 1;
        calls.set(n);
        async move { if n <= 2 { rate_limited() } else { Ok(42u32) } }
    })
    .await;
    assert_eq!(out.unwrap(), 42);
    assert_eq!(calls.get(), 3, "two rate-limited attempts, then success");
}

#[tokio::test]
async fn exhausted_retries_propagate_the_error() {
    // #1832: arms GLOBAL_COOLDOWN via `wait_out`, see above.
    let _guard = test_support::registry_guard().await;
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let calls = Cell::new(0u32);
    let out: Result<(), _> = send_retrying_rate_limit("test send", || {
        calls.set(calls.get() + 1);
        async { rate_limited() }
    })
    .await;
    assert!(matches!(out, Err(teloxide::RequestError::RetryAfter(_))));
    // 1 initial attempt + 3 retries.
    assert_eq!(calls.get(), 4);
}

#[tokio::test]
async fn non_rate_limit_error_propagates_immediately() {
    let _guard = test_support::registry_guard().await;
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let calls = Cell::new(0u32);
    let out: Result<(), _> = send_retrying_rate_limit("test send", || {
        calls.set(calls.get() + 1);
        async { Err(teloxide::RequestError::Api(teloxide::ApiError::BotBlocked)) }
    })
    .await;
    assert!(matches!(
        out,
        Err(teloxide::RequestError::Api(teloxide::ApiError::BotBlocked))
    ));
    assert_eq!(calls.get(), 1, "no retries for non-429 errors");
}

#[tokio::test]
async fn first_try_success_sends_once() {
    let _guard = test_support::registry_guard().await;
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let calls = Cell::new(0u32);
    let out = send_retrying_rate_limit("test send", || {
        calls.set(calls.get() + 1);
        async { Ok::<_, teloxide::RequestError>("ok") }
    })
    .await;
    assert_eq!(out.unwrap(), "ok");
    assert_eq!(calls.get(), 1);
}

// --- #1064: oversized windows are capped inline, never slept in full ---

/// The observed field case: every attempt 429s with an 8288s window. With
/// the #1110 fix, windows > 1 hour bail immediately without retrying (1 attempt
/// total), since retrying burns 90s for no gain when the window is hours long.
#[tokio::test(start_paused = true)]
async fn oversized_windows_are_capped_not_slept_in_full() {
    let _guard = test_support::registry_guard().await;
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let calls = Cell::new(0u32);
    let start = tokio::time::Instant::now();
    let out: Result<(), _> = send_retrying_rate_limit("test send", || {
        calls.set(calls.get() + 1);
        async {
            Err(teloxide::RequestError::RetryAfter(Seconds::from_seconds(
                8288,
            )))
        }
    })
    .await;
    let elapsed = start.elapsed();
    assert!(
        matches!(out, Err(teloxide::RequestError::RetryAfter(_))),
        "long rate-limit (>1 hour) bails immediately with error"
    );
    assert_eq!(
        calls.get(),
        1,
        "long rate-limit bails immediately, no retries (#1110)"
    );
    assert_eq!(
        elapsed,
        Duration::from_secs(0),
        "3 capped waits of 30s, not the 3x8288s inline hostage of #1064"
    );
}

/// Windows under the cap keep #297 semantics: waited in full, then retried.
#[tokio::test(start_paused = true)]
async fn small_windows_are_waited_in_full() {
    // #1832: a 5s window arms GLOBAL_COOLDOWN for 7s via `wait_out`, see above.
    let _guard = test_support::registry_guard().await;
    let _cooldown = crate::tests::telegram_cooldown_lock::guard().await;
    let calls = Cell::new(0u32);
    let start = tokio::time::Instant::now();
    let out = send_retrying_rate_limit("test send", || {
        let n = calls.get() + 1;
        calls.set(n);
        async move {
            if n == 1 {
                Err(teloxide::RequestError::RetryAfter(Seconds::from_seconds(5)))
            } else {
                Ok(7u32)
            }
        }
    })
    .await;
    assert_eq!(out.unwrap(), 7);
    assert_eq!(start.elapsed(), Duration::from_secs(5));
}

#[test]
fn clamp_inline_wait_respects_the_cap_boundary() {
    use crate::channels::telegram::rate_limit::clamp_inline_wait;
    // Under the cap: unchanged, not flagged.
    assert_eq!(
        clamp_inline_wait(Duration::from_secs(0)),
        (Duration::from_secs(0), false)
    );
    assert_eq!(
        clamp_inline_wait(Duration::from_secs(5)),
        (Duration::from_secs(5), false)
    );
    assert_eq!(
        clamp_inline_wait(Duration::from_secs(30)),
        (Duration::from_secs(30), false),
        "exactly the cap is not a capped wait"
    );
    // Over the cap: clamped and flagged for the forensics log line.
    assert_eq!(
        clamp_inline_wait(Duration::from_secs(31)),
        (Duration::from_secs(30), true)
    );
    assert_eq!(
        clamp_inline_wait(Duration::from_secs(8288)),
        (Duration::from_secs(30), true),
        "the observed flood-ban window clamps to the cap"
    );
}
