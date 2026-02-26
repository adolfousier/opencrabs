//! Generic retry utilities with exponential backoff
//!
//! Provides a unified retry mechanism for both database and API operations.

use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

/// Trait for errors that can be classified as retryable
pub trait RetryableError: std::fmt::Display {
    /// Check if this error should be retried
    fn is_retryable(&self) -> bool;

    /// Optional: Extract Retry-After duration if available
    fn retry_after(&self) -> Option<Duration> {
        None
    }
}

/// Universal retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 means no retries)
    pub max_attempts: u32,
    /// Initial delay before first retry
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Backoff multiplier (typically 2.0 for exponential)
    pub backoff_multiplier: f64,
    /// Add random jitter to delays (0.0 = none, 0.1+ = recommended for distributed systems)
    pub jitter: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter: 0.1,
        }
    }
}

impl RetryConfig {
    /// Create database-optimized config (high frequency, low latency, deterministic)
    pub fn database() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 2.0,
            jitter: 0.0, // Deterministic for database locks
        }
    }

    /// Create aggressive database retry config
    pub fn database_aggressive() -> Self {
        Self {
            max_attempts: 10,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 1.5,
            jitter: 0.0,
        }
    }

    /// Create API-optimized config (distributed backoff with jitter)
    pub fn api() -> Self {
        Self::default()
    }

    /// Create aggressive API retry config
    pub fn api_aggressive() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter: 0.2,
        }
    }

    /// Create no-retry config
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 0,
            ..Default::default()
        }
    }

    /// Calculate delay for a given attempt with optional jitter
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let base_delay_ms = self.initial_delay.as_millis() as f64;
        let exponential = base_delay_ms * self.backoff_multiplier.powi(attempt as i32);
        let capped = exponential.min(self.max_delay.as_millis() as f64);

        let final_delay = if self.jitter > 0.0 {
            use rand::Rng;
            let mut rng = rand::rng();
            let jitter_factor = 1.0 + rng.random_range(-self.jitter..self.jitter);
            (capped * jitter_factor).max(0.0)
        } else {
            capped
        };

        Duration::from_millis(final_delay as u64)
    }
}

/// Generic retry function that works with any retryable error type
///
/// # Example
/// ```ignore
/// let config = RetryConfig::api();
/// let result = retry(|| async { make_api_call().await }, &config).await;
/// ```
pub async fn retry<F, Fut, T, E>(
    mut operation: F,
    config: &RetryConfig,
) -> std::result::Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: RetryableError,
{
    let mut attempt = 0;
    let mut last_error: Option<E> = None;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!("Operation succeeded after {} retries", attempt);
                }
                return Ok(result);
            }
            Err(err) => {
                if config.max_attempts == 0 || !err.is_retryable() {
                    tracing::debug!("Error is not retryable: {}", err);
                    return Err(err);
                }

                if attempt >= config.max_attempts {
                    tracing::warn!("Max retry attempts ({}) exceeded", config.max_attempts);
                    return Err(last_error.unwrap_or(err));
                }

                // Check for Retry-After hint from the error
                let delay = if let Some(retry_after) = err.retry_after() {
                    tracing::info!(
                        "Error provided retry_after hint: {}ms",
                        retry_after.as_millis()
                    );
                    retry_after
                } else {
                    config.calculate_delay(attempt)
                };

                tracing::info!(
                    "Retry attempt {}/{} after {}ms for error: {}",
                    attempt + 1,
                    config.max_attempts,
                    delay.as_millis(),
                    err
                );

                last_error = Some(err);
                sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

/// Retry with a simple error display (for errors that don't implement RetryableError)
///
/// Uses a custom retryable check function.
pub async fn retry_with_check<F, Fut, T, E, C>(
    mut operation: F,
    config: &RetryConfig,
    is_retryable: C,
) -> std::result::Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
    C: Fn(&E) -> bool,
{
    let mut attempt = 0;
    let mut last_error: Option<E> = None;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!("Operation succeeded after {} retries", attempt);
                }
                return Ok(result);
            }
            Err(err) => {
                if config.max_attempts == 0 || !is_retryable(&err) {
                    tracing::debug!("Error is not retryable: {}", err);
                    return Err(err);
                }

                if attempt >= config.max_attempts {
                    tracing::warn!("Max retry attempts ({}) exceeded", config.max_attempts);
                    return Err(last_error.unwrap_or(err));
                }

                let delay = config.calculate_delay(attempt);

                tracing::info!(
                    "Retry attempt {}/{} after {}ms for error: {}",
                    attempt + 1,
                    config.max_attempts,
                    delay.as_millis(),
                    err
                );

                last_error = Some(err);
                sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Debug)]
    struct TestError {
        retryable: bool,
        message: String,
    }

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl RetryableError for TestError {
        fn is_retryable(&self) -> bool {
            self.retryable
        }
    }

    #[tokio::test]
    async fn test_successful_operation_no_retry() {
        let config = RetryConfig::default();
        let result: Result<i32, TestError> = retry(|| async { Ok(42) }, &config).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_non_retryable_error_fails_immediately() {
        let config = RetryConfig::default();
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result: Result<i32, TestError> = retry(
            || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err(TestError {
                        retryable: false,
                        message: "permanent error".into(),
                    })
                }
            },
            &config,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // Only called once
    }

    #[tokio::test]
    async fn test_retryable_error_retries() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            jitter: 0.0,
        };

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result: Result<i32, TestError> = retry(
            || {
                let count = call_count_clone.clone();
                async move {
                    let current = count.fetch_add(1, Ordering::SeqCst);
                    if current < 2 {
                        Err(TestError {
                            retryable: true,
                            message: "temporary error".into(),
                        })
                    } else {
                        Ok(42)
                    }
                }
            },
            &config,
        )
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // Initial + 2 retries
    }

    #[tokio::test]
    async fn test_max_attempts_exceeded() {
        let config = RetryConfig {
            max_attempts: 2,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            jitter: 0.0,
        };

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result: Result<i32, TestError> = retry(
            || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err(TestError {
                        retryable: true,
                        message: "always fails".into(),
                    })
                }
            },
            &config,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // Initial + 2 retries
    }

    #[tokio::test]
    async fn test_no_retry_config() {
        let config = RetryConfig::no_retry();
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result: Result<i32, TestError> = retry(
            || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err(TestError {
                        retryable: true,
                        message: "error".into(),
                    })
                }
            },
            &config,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // No retries
    }

    #[test]
    fn test_calculate_delay_exponential() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            jitter: 0.0,
            ..Default::default()
        };

        assert_eq!(config.calculate_delay(0), Duration::from_millis(100));
        assert_eq!(config.calculate_delay(1), Duration::from_millis(200));
        assert_eq!(config.calculate_delay(2), Duration::from_millis(400));
        assert_eq!(config.calculate_delay(3), Duration::from_millis(800));
    }

    #[test]
    fn test_calculate_delay_capped() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
            backoff_multiplier: 2.0,
            jitter: 0.0,
            ..Default::default()
        };

        assert_eq!(config.calculate_delay(0), Duration::from_millis(100));
        assert_eq!(config.calculate_delay(1), Duration::from_millis(200));
        assert_eq!(config.calculate_delay(2), Duration::from_millis(400));
        assert_eq!(config.calculate_delay(3), Duration::from_millis(500)); // Capped
        assert_eq!(config.calculate_delay(10), Duration::from_millis(500)); // Capped
    }

    #[test]
    fn test_preset_configs() {
        let db = RetryConfig::database();
        assert_eq!(db.max_attempts, 5);
        assert_eq!(db.initial_delay, Duration::from_millis(50));
        assert_eq!(db.jitter, 0.0);

        let api = RetryConfig::api();
        assert_eq!(api.max_attempts, 3);
        assert_eq!(api.jitter, 0.1);

        let no_retry = RetryConfig::no_retry();
        assert_eq!(no_retry.max_attempts, 0);
    }
}
