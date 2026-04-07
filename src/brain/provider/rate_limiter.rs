//! Rate Limiter for proactive request pacing
//!
//! Used to stay under provider rate limits (e.g. OpenRouter :free at 20 req/min)
//! by enforcing a minimum interval between API calls. This is robustness:
//! preventing rate-limit hits rather than reacting to them.
//!
//! ## Global Shared Limiter
//!
//! All OpenRouter :free provider instances share a single global limiter
//! (`OPENROUTER_FREE_LIMITER`). This ensures that the main orchestrator,
//! subagents, and team members all pace against ONE budget — not per-instance
//! budgets that would collectively exceed the provider's actual rate limit.

use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Fixed reference point for nanosecond timestamps.
/// All `last_granted` values are stored as nanoseconds since this Instant.
static PROCESS_START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Global rate limiter shared by ALL OpenRouter :free provider instances.
/// 20 req/min cap → 3.5s between requests (with 0.5s headroom over the
/// strict 3s minimum) to absorb clock drift and parallel subagent bursts.
pub static OPENROUTER_FREE_LIMITER: LazyLock<Arc<RateLimiter>> =
    LazyLock::new(|| Arc::new(RateLimiter::openrouter_free()));

/// Enforces a minimum interval between consecutive calls to `wait()`.
///
/// Thread-safe and clone-friendly — multiple provider clones share the same
/// limiter, so pacing is process-wide, not per-instance.
#[derive(Debug)]
pub struct RateLimiter {
    /// Minimum gap between allowed requests.
    pub(crate) min_interval: Duration,
    /// Nanoseconds since PROCESS_START when the last request was granted.
    /// 0 = no request yet.
    last_granted: AtomicU64,
}

impl RateLimiter {
    /// Create a new rate limiter with the given minimum interval.
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            // 0 = no slot claimed yet. First call will always win because
            // PROCESS_START.elapsed() is always >> 0.
            last_granted: AtomicU64::new(0),
        }
    }

    /// OpenRouter :free tier rate — 20 req/min cap. Pace at 3.5s between
    /// requests (0.5s headroom over the strict 3s minimum) so transient
    /// bursts from parallel subagents don't tip us over the moving window.
    pub fn openrouter_free() -> Self {
        Self::new(Duration::from_millis(3500))
    }

    fn now_ns() -> u64 {
        PROCESS_START.elapsed().as_nanos() as u64
    }

    /// Wait if necessary so that at least `min_interval` has elapsed since the
    /// previous successful call. Returns the duration we actually slept
    /// (zero if we were already within budget).
    pub async fn wait(&self) -> Duration {
        let now_ns = Self::now_ns();

        loop {
            let last = self.last_granted.load(Ordering::Acquire);

            if last == 0 {
                // No previous grant — first call always wins immediately.
                // Treat 0 as a sentinel regardless of `now_ns` value.
                if self
                    .last_granted
                    .compare_exchange(0, now_ns, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Duration::ZERO;
                }
                continue;
            }

            let elapsed_ns = now_ns.saturating_sub(last);
            let elapsed = Duration::from_nanos(elapsed_ns);

            if elapsed >= self.min_interval {
                if self
                    .last_granted
                    .compare_exchange(last, now_ns, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Duration::ZERO;
                }
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
        }
    }
}
