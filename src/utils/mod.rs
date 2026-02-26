//! Utility modules for common functionality

pub mod retry;
mod string;

pub use retry::{RetryConfig, RetryableError, retry, retry_with_check};
pub use string::truncate_str;
