//! Rate Limiter for proactive request pacing
//!
//! Used to stay under provider rate limits (e.g. OpenRouter :free at 20 req/min)
//! by enforcing a minimum interval between API calls. This is robustness:
//! preventing rate-limit hits rather than reacting to them.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Enforces a minimum interval between consecutive calls to `wait()`.
///
/// Thread-safe and clone-friendly — multiple provider clones share the same
/// limiter, so pacing is process-wide, not per-instance.
#[derive(Debug)]
pub struct RateLimiter {
    /// Minimum gap between allowed requests.
    min_interval: Duration,
    /// Nanosecond timestamp of the last granted request.
    last_granted: AtomicU64,
}

impl RateLimiter {
    /// Create a new rate limiter with the given minimum interval.
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            // Start in the past so the very first request never waits.
            last_granted: AtomicU64::new(0),
        }
    }

    /// OpenRouter :free tier rate — 20 req/min → 3s between requests.
    pub fn openrouter_free() -> Self {
        Self::new(Duration::from_secs(3))
    }

    /// Wait if necessary so that at least `min_interval` has elapsed since the
    /// previous successful call. Returns the duration we actually slept
    /// (zero if we were already within budget).
    pub async fn wait(&self) -> Duration {
        let now_ns = Instant::now().elapsed().as_nanos() as u64;

        loop {
            let last = self.last_granted.load(Ordering::Acquire);
            let elapsed_ns = now_ns.saturating_sub(last);
            let elapsed = Duration::from_nanos(elapsed_ns);

            if elapsed >= self.min_interval {
                // CAS to claim this slot. If another thread beat us, retry.
                if self
                    .last_granted
                    .compare_exchange(last, now_ns, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Duration::ZERO;
                }
                // Another thread won — re-check with updated `last`.
                continue;
            }

            let sleep_for = self.min_interval - elapsed;
            let grant_at = now_ns + sleep_for.as_nanos() as u64;

            if self
                .last_granted
                .compare_exchange(last, grant_at, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                sleep(sleep_for).await;
                return sleep_for;
            }
            // CAS failed — recalculate with fresh last_granted.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_first_call_is_free() {
        let limiter = RateLimiter::new(Duration::from_millis(50));
        let waited = limiter.wait().await;
        assert_eq!(waited, Duration::ZERO);
    }

    #[tokio::test]
    async fn test_enforces_minimum_gap() {
        let limiter = RateLimiter::new(Duration::from_millis(50));
        limiter.wait().await; // first call, instant

        let start = Instant::now();
        limiter.wait().await;
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(40),
            "expected ≥40ms gap, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_no_wait_after_gap() {
        let limiter = RateLimiter::new(Duration::from_millis(20));
        limiter.wait().await;
        sleep(Duration::from_millis(30)).await;

        let start = Instant::now();
        let waited = limiter.wait().await;
        assert_eq!(waited, Duration::ZERO);
        assert!(start.elapsed() < Duration::from_millis(5));
    }
}
