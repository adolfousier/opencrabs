//! Database Retry Logic
//!
//! Handles SQLite lock errors and retries with exponential backoff.
//!
//! ## Features
//! - Automatic retry for SQLITE_BUSY errors
//! - Exponential backoff for lock contention
//! - Configurable retry attempts
//! - Logging for debugging lock issues

use anyhow::{Context, Result};
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

/// Configuration for database retry logic
#[derive(Debug, Clone)]
pub struct DbRetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Initial delay before first retry
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Backoff multiplier (typically 2.0 for exponential)
    pub backoff_multiplier: f64,
}

impl Default for DbRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 2.0,
        }
    }
}

impl DbRetryConfig {
    /// Create a new retry config with custom settings
    pub fn new(max_attempts: u32, initial_delay: Duration) -> Self {
        Self {
            max_attempts,
            initial_delay,
            ..Default::default()
        }
    }

    /// Create config for aggressive retry (for high contention)
    pub fn aggressive() -> Self {
        Self {
            max_attempts: 10,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 1.5,
        }
    }

    /// Calculate delay for a given attempt
    fn calculate_delay(&self, attempt: u32) -> Duration {
        let base_delay = self.initial_delay.as_millis() as f64;
        let exponential = base_delay * self.backoff_multiplier.powi(attempt as i32);
        let max_delay_ms = self.max_delay.as_millis() as f64;

        let delay = exponential.min(max_delay_ms);
        Duration::from_millis(delay as u64)
    }
}

/// Check if an error is a SQLite busy/locked error
fn is_database_locked(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db_err) => {
            let msg = db_err.message().to_lowercase();
            msg.contains("locked") || msg.contains("busy")
        }
        _ => false,
    }
}

/// Retry a database operation with exponential backoff
///
/// # Example
/// ```no_run
/// use opencrabs::db::retry::{retry_db_operation, DbRetryConfig};
/// use sqlx::SqlitePool;
///
/// async fn example() {
///     async fn insert_record(pool: &SqlitePool) -> Result<(), sqlx::Error> {
///         // ... database operation
///         Ok(())
///     }
///
///     let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
///     let config = DbRetryConfig::default();
///     let result = retry_db_operation(|| insert_record(&pool), &config).await;
/// }
/// ```
pub async fn retry_db_operation<F, Fut, T, E>(
    mut operation: F,
    config: &DbRetryConfig,
) -> std::result::Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0;
    let mut last_error;

    loop {
        // Try the operation
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!("Database operation succeeded after {} retries", attempt);
                }
                return Ok(result);
            }
            Err(err) => {
                // Store error for potential final return
                let error_msg = err.to_string();
                last_error = err;

                // Check if we should retry
                let is_locked = error_msg.to_lowercase().contains("locked")
                    || error_msg.to_lowercase().contains("busy");

                if !is_locked {
                    tracing::debug!("Database error is not retryable: {}", error_msg);
                    return Err(last_error);
                }

                // Check if we've exhausted attempts
                if attempt >= config.max_attempts {
                    tracing::warn!(
                        "Max database retry attempts ({}) exceeded for lock error",
                        config.max_attempts
                    );
                    return Err(last_error);
                }

                // Calculate delay for this attempt
                let delay = config.calculate_delay(attempt);

                tracing::info!(
                    "Database locked (attempt {}/{}), retrying after {}ms",
                    attempt + 1,
                    config.max_attempts,
                    delay.as_millis()
                );

                // Wait before retrying
                sleep(delay).await;

                attempt += 1;
            }
        }
    }
}

/// Retry a database operation that returns anyhow::Result
pub async fn retry_db_anyhow<F, Fut, T>(operation: F, config: &DbRetryConfig) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    retry_db_operation(operation, config)
        .await
        .context("Database operation failed after retries")
}

/// Retry a database operation that returns sqlx::Result
pub async fn retry_db_sqlx<F, Fut, T>(
    mut operation: F,
    config: &DbRetryConfig,
) -> std::result::Result<T, sqlx::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, sqlx::Error>>,
{
    let mut attempt = 0;
    let mut last_error;

    loop {
        // Try the operation
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!("Database operation succeeded after {} retries", attempt);
                }
                return Ok(result);
            }
            Err(err) => {
                // Check if this is a lock error
                let is_locked = is_database_locked(&err);

                if !is_locked {
                    tracing::debug!("Database error is not retryable: {}", err);
                    return Err(err);
                }

                // Store error
                last_error = err;

                // Check if we've exhausted attempts
                if attempt >= config.max_attempts {
                    tracing::warn!(
                        "Max database retry attempts ({}) exceeded for lock error",
                        config.max_attempts
                    );
                    return Err(last_error);
                }

                // Calculate delay for this attempt
                let delay = config.calculate_delay(attempt);

                tracing::info!(
                    "Database locked (attempt {}/{}), retrying after {}ms",
                    attempt + 1,
                    config.max_attempts,
                    delay.as_millis()
                );

                // Wait before retrying
                sleep(delay).await;

                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_config_defaults() {
        let config = DbRetryConfig::default();
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.initial_delay, Duration::from_millis(50));
        assert_eq!(config.max_delay, Duration::from_secs(5));
    }

    #[test]
    fn test_retry_config_aggressive() {
        let config = DbRetryConfig::aggressive();
        assert_eq!(config.max_attempts, 10);
        assert_eq!(config.initial_delay, Duration::from_millis(100));
    }

    #[test]
    fn test_calculate_delay() {
        let config = DbRetryConfig {
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 2.0,
            max_attempts: 5,
        };

        let delay0 = config.calculate_delay(0);
        assert_eq!(delay0, Duration::from_millis(50));

        let delay1 = config.calculate_delay(1);
        assert_eq!(delay1, Duration::from_millis(100));

        let delay2 = config.calculate_delay(2);
        assert_eq!(delay2, Duration::from_millis(200));

        // Should cap at max_delay
        let delay10 = config.calculate_delay(10);
        assert_eq!(delay10, Duration::from_secs(5));
    }

    #[test]
    fn test_is_database_locked() {
        use sqlx::Error;

        // Test with a mock database error containing "locked"
        let err = Error::Io(std::io::Error::other("database is locked"));
        assert!(!is_database_locked(&err)); // IO errors don't count

        // Test with non-lock error
        let err = Error::RowNotFound;
        assert!(!is_database_locked(&err));
    }

    #[tokio::test]
    async fn test_retry_success_immediate() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let config = DbRetryConfig::default();
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_db_operation(
            move || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, String>(42)
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

        let config = DbRetryConfig::new(3, Duration::from_millis(10));
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_db_operation(
            move || {
                let count = call_count_clone.clone();
                async move {
                    let current = count.fetch_add(1, Ordering::SeqCst) + 1;
                    if current < 3 {
                        Err("database is locked".to_string())
                    } else {
                        Ok::<_, String>(42)
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

        let config = DbRetryConfig::new(2, Duration::from_millis(10));
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_db_operation(
            move || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>("database is locked".to_string())
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

        let config = DbRetryConfig::default();
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_db_operation(
            move || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>("constraint violation".to_string())
                }
            },
            &config,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // Should not retry
    }
}
