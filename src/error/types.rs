//! Error types and error codes for OpenCrabs.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum OpenCrabsError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {message}")]
    Config { message: String, code: ErrorCode },

    #[error("LLM provider error: {provider} - {message}")]
    Provider {
        provider: String,
        message: String,
        code: ErrorCode,
    },

    #[error("Tool execution error: {tool} - {message}")]
    ToolExecution {
        tool: String,
        message: String,
        code: ErrorCode,
    },

    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorCode {
    // Configuration errors (1000-1999)
    ConfigNotFound = 1000,
    ConfigInvalid = 1001,
    ConfigMergeError = 1002,

    // Provider errors (2000-2999)
    ProviderNotFound = 2000,
    ProviderAuthFailed = 2001,
    ProviderRateLimit = 2002,
    ProviderTimeout = 2003,

    // Tool errors (3000-3999)
    ToolNotFound = 3000,
    ToolExecutionFailed = 3001,
    ToolTimeout = 3002,

    // Permission errors (4000-4999)
    PermissionDenied = 4000,
    PermissionNotGranted = 4001,
}

impl OpenCrabsError {
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Config { code, .. } => Some(*code),
            Self::Provider { code, .. } => Some(*code),
            Self::ToolExecution { code, .. } => Some(*code),
            _ => None,
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::Config { message, .. } => {
                format!(
                    "Configuration error: {}\nPlease check your opencrabs.json file.",
                    message
                )
            }
            Self::Provider {
                provider, message, ..
            } => {
                format!(
                    "Error with {} provider: {}\nPlease verify your API key.",
                    provider, message
                )
            }
            Self::ToolExecution { tool, message, .. } => {
                format!("Tool '{}' failed: {}", tool, message)
            }
            Self::PermissionDenied(tool) => {
                format!(
                    "Permission denied for tool '{}'. Grant permission or add to whitelist.",
                    tool
                )
            }
            _ => self.to_string(),
        }
    }
}
