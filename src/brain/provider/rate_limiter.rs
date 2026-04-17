//! Rate Limiter for proactive request pacing
//!
//! Used to stay under provider rate limits (e.g. OpenRouter :free at 20 req/min)
//! by enforcing a minimum interval between API calls. This is robustness:
//! preventing rate-limit hits rather than reacting to them.
//!
//! ## Per-Model Global Limiters
//!
//! Each `:free` model gets its own independent rate limiter bucket, shared
//! across all provider instances and sessions using that exact model.
//! This prevents 429s from concurrent sessions hammering the same endpoint.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

/// Fixed reference point for nanosecond timestamps.
/// All `last_granted` values are stored as nanoseconds since this Instant.
static PROCESS_START: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

/// Interval between requests for OpenRouter :free models.
/// 4.0s = 15 req/min, safely under the 20 req/min window with 25% headroom.
const OPENROUTER_FREE_INTERVAL: Duration = Duration::from_millis(4000);

/// Global registry of per-model rate limiters for OpenRouter :free tier.
/// Keyed by exact model string (e.g. "qwen/qwen3.6-plus:free").
/// Lazily creates a new limiter on first use for each model.
pub static OPENROUTER_FREE_LIMITERS: std::sync::LazyLock<GlobalRateLimiter> =
    std::sync::LazyLock::new(GlobalRateLimiter::new);

/// Interval between requests for Qwen OAuth free tier.
/// Portal reports 60 req/min (1/s sustained). 1500ms = 40 req/min, 33% headroom.
const QWEN_OAUTH_INTERVAL: Duration = Duration::from_millis(1500);

/// Global singleton limiter for the Qwen OAuth endpoint. Must outlive any
/// individual provider instance, because `try_create_qwen` is called on every
/// sticky-fallback resolve — a per-provider limiter would reset each request
/// and let the second call bypass pacing entirely.
pub static QWEN_OAUTH_LIMITER: std::sync::LazyLock<Arc<RateLimiter>> =
    std::sync::LazyLock::new(|| Arc::new(RateLimiter::new(QWEN_OAUTH_INTERVAL)));

/// Enforces a minimum interval between consecutive calls to `wait()` for a
/// single `:free` model. Thread-safe and clone-friendly.
#[derive(Debug)]
pub struct RateLimiter {
    /// Minimum gap between allowed requests.
    pub(crate) min_interval: Duration,
    /// Nanoseconds since PROCESS_START when the last request was granted.
    /// 0 = no request yet.
    last_granted: AtomicU64,
}

impl RateLimiter {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_granted: AtomicU64::new(0),
        }
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

/// Global registry that hands out per-model rate limiters.
/// Each unique model string gets its own independent RateLimiter.
/// Thread-safe — multiple threads can call `get(model)` concurrently.
pub struct GlobalRateLimiter {
    limiters: Arc<Mutex<HashMap<String, Arc<RateLimiter>>>>,
}

impl GlobalRateLimiter {
    pub(crate) fn new() -> Self {
        Self {
            limiters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get (or create) the per-model rate limiter for a given model string.
    /// Sync — safe to call from anywhere without async context.
    pub fn get(&self, model: &str) -> Arc<RateLimiter> {
        {
            let map = self.limiters.lock().unwrap();
            if let Some(limiter) = map.get(model) {
                return Arc::clone(limiter);
            }
        }
        // Double-checked locking: another thread may have created it while we
        // were waiting for the lock after the first lookup.
        let mut map = self.limiters.lock().unwrap();
        // Check again under the write lock
        if let Some(limiter) = map.get(model) {
            return Arc::clone(limiter);
        }
        let limiter = Arc::new(RateLimiter::new(OPENROUTER_FREE_INTERVAL));
        map.insert(model.to_string(), limiter.clone());
        limiter
    }
}

