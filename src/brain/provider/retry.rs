//! Retry Logic with Exponential Backoff
//!
//! Provides automatic retry with exponential backoff for failed API requests.
//!
//! ## Features
//! - Exponential backoff with jitter
//! - Configurable max attempts and delays
//! - Rate limit handling with Retry-After support
//! - Selective retry based on error type

use super::error::{ProviderError, Result};
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

/// Retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries)
    pub max_attempts: u32,
    /// Initial delay before first retry
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Backoff multiplier (typically 2.0 for exponential)
    pub backoff_multiplier: f64,
    /// Add random jitter to delays (0.0-1.0)
    pub jitter: f64,
    /// If true, 429 / rate-limit errors are retried in-place with backoff
    /// (honoring Retry-After) instead of immediately bailing to the
    /// FallbackProvider. Matches qwen-cli's DEFAULT_RETRY_OPTIONS behavior.
    pub retry_on_rate_limit: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter: 0.1,
            retry_on_rate_limit: false,
        }
    }
}

impl RetryConfig {
    /// Create a new retry config with custom settings
    pub fn new(max_attempts: u32, initial_delay: Duration) -> Self {
        Self {
            max_attempts,
            initial_delay,
            ..Default::default()
        }
    }

    /// Create config with no retries
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 0,
            ..Default::default()
        }
    }

    /// Create config for aggressive retry (for rate limits)
    pub fn aggressive() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter: 0.2,
            retry_on_rate_limit: true,
        }
    }

    /// Qwen / OpenRouter-style tight-window retry: retry in-place with a 3s
    /// initial delay so we don't burn quota while the rate-limit window is
    /// closed. Backoff 3s → 6s → 12s → 24s.
    pub fn qwen_cli_match() -> Self {
        Self {
            max_attempts: 4,
            initial_delay: Duration::from_secs(3),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter: 0.2,
            retry_on_rate_limit: true,
        }
    }

    /// Calculate delay for a given attempt with exponential backoff and jitter
    fn calculate_delay(&self, attempt: u32) -> Duration {
        let base_delay = self.initial_delay.as_millis() as f64;
        let exponential = base_delay * self.backoff_multiplier.powi(attempt as i32);
        let max_delay_ms = self.max_delay.as_millis() as f64;

        // Apply max delay cap
        let delay = exponential.min(max_delay_ms);

        // Apply jitter: random value between (1 - jitter) and (1 + jitter)
        let jitter_factor = if self.jitter > 0.0 {
            use rand::Rng;
            let mut rng = rand::rng();
            1.0 + rng.random_range(-self.jitter..self.jitter)
        } else {
            1.0
        };

        let final_delay = (delay * jitter_factor).max(0.0) as u64;
        Duration::from_millis(final_delay)
    }
}

/// Retry a provider operation with exponential backoff
///
/// # Example
/// ```no_run
/// use opencrabs::brain::provider::retry::{retry_with_backoff, RetryConfig};
/// use opencrabs::brain::provider::ProviderError;
///
/// async fn example() {
///     async fn make_api_call() -> Result<String, ProviderError> {
///         // ... API call logic
///         Ok("response".to_string())
///     }
///
///     let config = RetryConfig::default();
///     let result = retry_with_backoff(|| make_api_call(), &config).await;
/// }
/// ```
pub async fn retry_with_backoff<F, Fut, T>(mut operation: F, config: &RetryConfig) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 0;
    let mut last_error: Option<ProviderError> = None;

    loop {
        // Try the operation
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!("Operation succeeded after {} retries", attempt);
                }
                return Ok(result);
            }
            Err(err) => {
                // Check if we should retry
                if !err.is_retryable() {
                    tracing::debug!("Error is not retryable: {}", err);
                    return Err(err);
                }

                // Rate-limit errors: by default we bail immediately so the
                // FallbackProvider can swap to a healthy chain in milliseconds
                // instead of hammering a dead route whose shared upstream
                // window is closed. Providers that want qwen-cli-style
                // in-place retry (e.g. qwen OAuth, whose window reopens
                // within seconds) opt in via `retry_on_rate_limit`.
                let is_rate_limit = matches!(&err, ProviderError::RateLimitExceeded(_))
                    || matches!(
                        &err,
                        ProviderError::ApiError { status, .. } if *status == 429
                    );
                if is_rate_limit && !config.retry_on_rate_limit {
                    tracing::warn!(
                        "Rate limit hit — skipping retries, bailing to fallback: {}",
                        err
                    );
                    return Err(err);
                }

                // Check if we've exhausted attempts
                if attempt >= config.max_attempts {
                    tracing::warn!(
                        "Max retry attempts ({}) exceeded for error: {}",
                        config.max_attempts,
                        err
                    );
                    return Err(last_error.unwrap_or(err));
                }

                // When retrying a rate limit, prefer the server's Retry-After
                // hint (clamped to max_delay) over the naïve exponential
                // schedule so we don't retry before the window reopens.
                let delay = if is_rate_limit {
                    let base = config.calculate_delay(attempt);
                    match extract_retry_after(&err) {
                        Some(hint) => hint.min(config.max_delay).max(base),
                        None => base,
                    }
                } else {
                    config.calculate_delay(attempt)
                };

                tracing::info!(
                    "Retry attempt {} after {}ms for error: {}",
                    attempt + 1,
                    delay.as_millis(),
                    err
                );

                // Store error for final return if needed
                last_error = Some(err);

                // Wait before retrying
                sleep(delay).await;

                attempt += 1;
            }
        }
    }
}

/// Retry a provider operation with rate limit aware backoff
///
/// This variant respects Retry-After headers from rate limit responses
pub async fn retry_with_rate_limit<F, Fut, T>(
    operation: F,
    config: &RetryConfig,
    retry_after: Option<Duration>,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    // If we have a Retry-After header, wait for it first
    if let Some(wait_duration) = retry_after {
        tracing::info!(
            "Rate limit detected, waiting {}s as requested by server",
            wait_duration.as_secs()
        );
        sleep(wait_duration).await;
    }

    // Then use normal retry logic
    retry_with_backoff(operation, config).await
}

/// Extract Retry-After duration from rate limit error
///
/// Parses rate limit error messages to extract retry duration
pub fn extract_retry_after(error: &ProviderError) -> Option<Duration> {
    match error {
        ProviderError::RateLimitExceeded(msg) => {
            // Try to parse "retry in X seconds" or similar
            if let Some(secs) = parse_retry_seconds(msg) {
                return Some(Duration::from_secs(secs));
            }
            // No parseable hint — let exponential backoff decide
            None
        }
        ProviderError::ApiError {
            status, message, ..
        } if *status == 429 => {
            if let Some(secs) = parse_retry_seconds(message) {
                return Some(Duration::from_secs(secs));
            }
            // No parseable hint — let exponential backoff decide
            None
        }
        _ => None,
    }
}

/// Parse retry seconds from error message
fn parse_retry_seconds(msg: &str) -> Option<u64> {
    // Try to extract numbers followed by "second" or "s"
    use regex::Regex;

    // Patterns: "60 seconds", "60s", "retry in 60", etc.
    let patterns = [
        r"(\d+)\s*seconds?",
        r"(\d+)\s*s\b",
        r"retry in (\d+)",
        r"wait (\d+)",
    ];

    for pattern in patterns {
        if let Ok(re) = Regex::new(pattern)
            && let Some(captures) = re.captures(msg)
            && let Some(num_str) = captures.get(1)
            && let Ok(secs) = num_str.as_str().parse::<u64>()
        {
            return Some(secs);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.initial_delay, Duration::from_millis(100));
        assert_eq!(config.max_delay, Duration::from_secs(30));
    }

    #[test]
    fn test_retry_config_no_retry() {
        let config = RetryConfig::no_retry();
        assert_eq!(config.max_attempts, 0);
    }

    #[test]
    fn test_calculate_delay() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            jitter: 0.0, // Disable jitter for predictable testing
            max_attempts: 5,
            retry_on_rate_limit: false,
        };

        let delay0 = config.calculate_delay(0);
        assert_eq!(delay0, Duration::from_millis(100));

        let delay1 = config.calculate_delay(1);
        assert_eq!(delay1, Duration::from_millis(200));

        let delay2 = config.calculate_delay(2);
        assert_eq!(delay2, Duration::from_millis(400));

        let delay3 = config.calculate_delay(3);
        assert_eq!(delay3, Duration::from_millis(800));

        // Should cap at max_delay (10s = 10000ms)
        let delay10 = config.calculate_delay(10);
        assert_eq!(delay10, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn test_retry_success_immediate() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let config = RetryConfig::default();
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_backoff(
            move || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, ProviderError>(42)
                }
            },
            &config,
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_retries() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let config = RetryConfig::new(3, Duration::from_millis(10));
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_backoff(
            move || {
                let count = call_count_clone.clone();
                async move {
                    let current = count.fetch_add(1, Ordering::SeqCst) + 1;
                    if current < 3 {
                        Err(ProviderError::Timeout(10))
                    } else {
                        Ok::<_, ProviderError>(42)
                    }
                }
            },
            &config,
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_max_attempts_exceeded() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let config = RetryConfig::new(2, Duration::from_millis(10));
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_backoff(
            move || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>(ProviderError::Timeout(10))
                }
            },
            &config,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // Initial + 2 retries
    }

    #[tokio::test]
    async fn test_retry_non_retryable_error() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let config = RetryConfig::default();
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_backoff(
            move || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>(ProviderError::InvalidApiKey)
                }
            },
            &config,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // Should not retry
    }

    #[test]
    fn test_extract_retry_after() {
        let err = ProviderError::RateLimitExceeded(
            "Rate limit exceeded, retry in 60 seconds".to_string(),
        );
        let retry_after = extract_retry_after(&err);
        assert_eq!(retry_after, Some(Duration::from_secs(60)));

        let err = ProviderError::RateLimitExceeded("Please wait 30s".to_string());
        let retry_after = extract_retry_after(&err);
        assert_eq!(retry_after, Some(Duration::from_secs(30)));

        let err = ProviderError::InvalidApiKey;
        let retry_after = extract_retry_after(&err);
        assert_eq!(retry_after, None);
    }

    #[test]
    fn test_parse_retry_seconds() {
        assert_eq!(parse_retry_seconds("retry in 60 seconds"), Some(60));
        assert_eq!(parse_retry_seconds("wait 30s"), Some(30));
        assert_eq!(parse_retry_seconds("retry in 5"), Some(5));
        assert_eq!(parse_retry_seconds("no numbers here"), None);
    }
}
