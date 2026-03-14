//! Agent error types

use crate::brain::provider::ProviderError;
use thiserror::Error;

/// Agent error types
#[derive(Debug, Error)]
pub enum AgentError {
    /// Provider error
    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),

    /// Database error
    #[error("Database error: {0}")]
    Database(String),

    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(uuid::Uuid),

    /// Invalid request
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Context too large
    #[error("Context too large: {current} tokens exceeds limit of {limit}")]
    ContextTooLarge { current: usize, limit: usize },

    /// Tool execution error
    #[error("Tool execution error: {0}")]
    ToolError(String),

    /// Tool not found
    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    /// Maximum tool iterations exceeded
    #[error("Maximum tool iterations exceeded: {0}")]
    MaxIterationsExceeded(usize),

    /// Operation cancelled by user (e.g. /stop)
    #[error("Cancelled")]
    Cancelled,

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type for agent operations
pub type Result<T> = std::result::Result<T, AgentError>;
