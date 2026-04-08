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

/// Expand a leading `~` or `~/` in a user-provided path into the current
/// user's home directory. Everything else passes through unchanged.
///
/// Models routinely paste tilde paths (`~/.opencrabs/logs`) and without
/// expansion `PathBuf::is_absolute()` returns false, so the path gets
/// joined to the process working directory as literal `~` — which never
/// exists. This helper normalizes that so tools don't all have to
/// reinvent the wheel.
pub fn expand_tilde(path: &str) -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    if path == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }
    PathBuf::from(path)
}

/// Resolve a user-provided path into an absolute `PathBuf`.
///
/// 1. Leading `~` / `~/` is expanded to the user's home directory.
/// 2. Absolute paths pass through.
/// 3. Relative paths are joined to the supplied working directory.
///
/// This is the single source of truth for path resolution across all
/// path-taking tools so they stay consistent.
pub fn resolve_tool_path(
    requested_path: &str,
    working_directory: &std::path::Path,
) -> std::path::PathBuf {
    let expanded = expand_tilde(requested_path);
    if expanded.is_absolute() {
        expanded
    } else {
        working_directory.join(expanded)
    }
}

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
    let path = resolve_tool_path(requested_path, working_directory);

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

    #[test]
    fn test_expand_tilde_prefix() {
        let home = dirs::home_dir().expect("home dir required for this test");
        assert_eq!(expand_tilde("~/foo/bar"), home.join("foo/bar"));
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn test_expand_tilde_passthrough() {
        // Tilde in the middle of a path is NOT a home reference — leave it alone.
        assert_eq!(
            expand_tilde("/tmp/~backup").to_string_lossy(),
            "/tmp/~backup"
        );
        assert_eq!(expand_tilde("foo/bar").to_string_lossy(), "foo/bar");
        assert_eq!(expand_tilde("/abs/path").to_string_lossy(), "/abs/path");
    }

    #[test]
    fn test_resolve_tool_path_tilde_becomes_absolute() {
        // The classic custom-provider bug: model sends `~/.opencrabs/logs`,
        // cwd is `/Users/adolfo/srv/rs/opencrabs`. Before the fix this
        // produced `/Users/adolfo/srv/rs/opencrabs/~/.opencrabs/logs`.
        let cwd = std::path::Path::new("/Users/adolfo/srv/rs/opencrabs");
        let resolved = resolve_tool_path("~/.opencrabs/logs", cwd);
        let home = dirs::home_dir().expect("home dir required");
        assert_eq!(resolved, home.join(".opencrabs/logs"));
        assert!(resolved.is_absolute());
    }

    #[test]
    fn test_resolve_tool_path_relative_joins_cwd() {
        let cwd = std::path::Path::new("/tmp/project");
        assert_eq!(
            resolve_tool_path("src/main.rs", cwd),
            std::path::PathBuf::from("/tmp/project/src/main.rs"),
        );
    }

    #[test]
    fn test_resolve_tool_path_absolute_passthrough() {
        let cwd = std::path::Path::new("/tmp/project");
        assert_eq!(
            resolve_tool_path("/etc/hosts", cwd),
            std::path::PathBuf::from("/etc/hosts"),
        );
    }
}
