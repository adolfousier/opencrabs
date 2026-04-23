//! Error types for LLM providers

use thiserror::Error;

/// Provider error types
#[derive(Debug, Error)]
pub enum ProviderError {
    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// API returned an error
    #[error(
        "API error ({status}){}: {message}",
        error_type
            .as_ref()
            .filter(|t| !t.is_empty())
            .map(|t| format!(" [{}]", t))
            .unwrap_or_default()
    )]
    ApiError {
        status: u16,
        message: String,
        error_type: Option<String>,
    },

    /// Invalid API key
    #[error("Invalid API key")]
    InvalidApiKey,

    /// Rate limit exceeded
    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),

    /// Invalid request
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Model not found
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    /// Context length exceeded
    #[error("Context length exceeded: {0} tokens")]
    ContextLengthExceeded(u32),

    /// Streaming not supported
    #[error("Streaming not supported by this provider")]
    StreamingNotSupported,

    /// Tools not supported
    #[error("Tools not supported by this provider")]
    ToolsNotSupported,

    /// JSON parsing error
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// Streaming error
    #[error("Streaming error: {0}")]
    StreamError(String),

    /// Timeout
    #[error("Request timed out after {0}s")]
    Timeout(u64),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl ProviderError {
    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        match self {
            ProviderError::HttpError(_)
            | ProviderError::RateLimitExceeded(_)
            | ProviderError::Timeout(_) => true,
            ProviderError::ApiError { status, .. } if *status >= 500 => true,
            // HTTP 400 with a generic proxy-style body (empty error_type
            // AND a message that doesn't describe an actionable client
            // problem) is almost always a transient upstream failure
            // forwarded by the proxy. opencode.ai's "Provider returned
            // error" is the canonical case — the user's payload is fine,
            // their upstream is having a moment. Retry before falling
            // back. Real client-side 400s (invalid_model, validation
            // errors, bad JSON) carry specific error_type or message
            // strings and stay non-retryable.
            ProviderError::ApiError {
                status: 400,
                message,
                error_type,
            } => is_transient_proxy_400(message, error_type.as_deref()),
            _ => false,
        }
    }

    /// Get HTTP status code if available
    pub fn status_code(&self) -> Option<u16> {
        match self {
            ProviderError::ApiError { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// True when the server rejected the REQUEST's model id (not the
    /// credential). Some OpenAI-compatible proxies — notably
    /// `opencode.ai/zen` — return HTTP 401 with
    /// `{"error":{"type":"ModelError","message":"Model X not supported"}}`
    /// for "this key can't use that model", which collides with real
    /// auth failures. Downstream code uses this to keep the actual
    /// "invalid key" classification meaningful and route model-mismatch
    /// errors to a different UX path.
    pub fn is_model_unsupported(&self) -> bool {
        match self {
            ProviderError::ModelNotFound(_) => true,
            ProviderError::ApiError {
                error_type,
                message,
                ..
            } => {
                let type_hit = error_type.as_ref().is_some_and(|t| {
                    let t = t.to_ascii_lowercase();
                    t == "modelerror"
                        || t == "model_error"
                        || t == "model_not_found"
                        || t == "invalid_model"
                });
                let msg = message.to_ascii_lowercase();
                let msg_hit = msg.contains("model")
                    && (msg.contains("not supported")
                        || msg.contains("not found")
                        || msg.contains("unsupported"));
                type_hit || msg_hit
            }
            _ => false,
        }
    }
}

/// True when an HTTP 400 response body looks like a proxy passthrough of
/// an upstream hiccup rather than a real client-side error. Used by
/// `is_retryable` so opencode.ai-style "Provider returned error" 400s
/// go through the 3-retry backoff instead of bailing to fallback on
/// the first try.
pub(crate) fn is_transient_proxy_400(message: &str, error_type: Option<&str>) -> bool {
    // Real client errors always carry an error_type (OpenAI: "invalid_request_error",
    // "model_not_found", "validation_error", etc.). Treat any non-empty type as
    // non-transient so we don't retry bad payloads.
    if error_type.is_some_and(|t| !t.is_empty()) {
        return false;
    }
    let m = message.trim().to_ascii_lowercase();
    if m.is_empty() {
        return true;
    }
    // Known proxy-passthrough phrases. Add new strings here when a proxy
    // invents a different one.
    const TRANSIENT_HINTS: &[&str] = &[
        "provider returned error",
        "upstream error",
        "internal error",
        "temporary",
        "try again",
        "bad gateway",
    ];
    TRANSIENT_HINTS.iter().any(|h| m.contains(h))
}

/// Result type for provider operations
pub type Result<T> = std::result::Result<T, ProviderError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_retryable() {
        let rate_limit = ProviderError::RateLimitExceeded("Try again later".to_string());
        assert!(rate_limit.is_retryable());

        let invalid_key = ProviderError::InvalidApiKey;
        assert!(!invalid_key.is_retryable());

        let server_error = ProviderError::ApiError {
            status: 500,
            message: "Internal Server Error".to_string(),
            error_type: None,
        };
        assert!(server_error.is_retryable());

        let client_error = ProviderError::ApiError {
            status: 400,
            message: "Bad Request".to_string(),
            error_type: None,
        };
        assert!(!client_error.is_retryable());
    }

    #[test]
    fn test_status_code() {
        let error = ProviderError::ApiError {
            status: 429,
            message: "Too many requests".to_string(),
            error_type: Some("rate_limit_error".to_string()),
        };
        assert_eq!(error.status_code(), Some(429));

        let invalid_key = ProviderError::InvalidApiKey;
        assert_eq!(invalid_key.status_code(), None);
    }
}
