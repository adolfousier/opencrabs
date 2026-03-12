//! Tool error types

use thiserror::Error;

/// Tool error types
#[derive(Debug, Error)]
pub enum ToolError {
    /// Tool not found
    #[error("Tool not found: {0}")]
    NotFound(String),

    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Execution error
    #[error("Execution error: {0}")]
    Execution(String),

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Approval required
    #[error("Tool requires approval: {0}")]
    ApprovalRequired(String),

    /// File not found
    #[error("File not found: {0}")]
    FileNotFound(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Timeout
    #[error("Tool execution timed out after {0}s")]
    Timeout(u64),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type for tool operations
pub type Result<T> = std::result::Result<T, ToolError>;

/// Resolve a path relative to the working directory.
///
/// Absolute paths pass through as-is. Relative paths are joined to the
/// working directory. For new files the parent directory must exist.
///
/// Security is enforced at the tool level via `requires_approval` and
/// capability flags — not by restricting paths to a single directory.
pub fn validate_path_safety(
    requested_path: &str,
    working_directory: &std::path::Path,
) -> Result<std::path::PathBuf> {
    use std::path::PathBuf;

    let path = if PathBuf::from(requested_path).is_absolute() {
        PathBuf::from(requested_path)
    } else {
        working_directory.join(requested_path)
    };

    // For new files, verify the parent directory exists
    if !path.exists() {
        let parent = path
            .parent()
            .ok_or_else(|| ToolError::InvalidInput("Invalid path: no parent directory".into()))?;
        if !parent.exists() {
            return Err(ToolError::InvalidInput(format!(
                "Parent directory does not exist: {}",
                parent.display()
            )));
        }
    }

    Ok(path)
}

/// Resolve a path, check it exists, and confirm it's a file.
///
/// Returns a user-friendly error message suitable for ToolResult::error()
pub fn validate_file_path(
    requested_path: &str,
    working_directory: &std::path::Path,
) -> std::result::Result<std::path::PathBuf, String> {
    let path = match validate_path_safety(requested_path, working_directory) {
        Ok(p) => p,
        Err(ToolError::InvalidInput(msg)) => {
            return Err(format!("Invalid path: {}", msg));
        }
        Err(e) => {
            return Err(format!("Path validation failed: {}", e));
        }
    };

    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }

    if !path.is_file() {
        return Err(format!("Path is not a file: {}", path.display()));
    }

    Ok(path)
}

/// Resolve a path, check it exists, and confirm it's a directory.
///
/// Similar to validate_file_path but checks for directories instead of files.
pub fn validate_directory_path(
    requested_path: &str,
    working_directory: &std::path::Path,
) -> std::result::Result<std::path::PathBuf, String> {
    let path = match validate_path_safety(requested_path, working_directory) {
        Ok(p) => p,
        Err(ToolError::InvalidInput(msg)) => {
            return Err(format!("Invalid path: {}", msg));
        }
        Err(e) => {
            return Err(format!("Path validation failed: {}", e));
        }
    };

    if !path.exists() {
        return Err(format!("Directory not found: {}", path.display()));
    }

    if !path.is_dir() {
        return Err(format!("Path is not a directory: {}", path.display()));
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_error_display() {
        let err = ToolError::NotFound("test_tool".to_string());
        assert_eq!(err.to_string(), "Tool not found: test_tool");

        let err = ToolError::PermissionDenied("dangerous_operation".to_string());
        assert_eq!(err.to_string(), "Permission denied: dangerous_operation");
    }
}
