//! Error types for LLM providers

use thiserror::Error;

/// Provider error types
#[derive(Debug, Error)]
pub enum ProviderError {
    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// API returned an error
    #[error("API error ({status}){}: {message}", error_type.as_ref().map(|t| format!(" [{}]", t)).unwrap_or_default())]
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
