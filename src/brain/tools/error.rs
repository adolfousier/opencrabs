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

impl ToolError {
    /// True when the tool never actually ran: the call was rejected before
    /// execution because the name didn't resolve (`NotFound`) or the arguments
    /// failed validation (`InvalidInput`). These are model tool-USE mistakes,
    /// not reliability failures of the tool itself, so the feedback ledger
    /// records them as `discovery_miss` rather than `tool_failure` and they're
    /// kept out of a tool's success rate (#214).
    pub fn is_pre_execution_miss(&self) -> bool {
        matches!(self, ToolError::NotFound(_) | ToolError::InvalidInput(_))
    }
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
    // Windows backslash form: models routinely paste `~\.opencrabs\logs`
    // with native separators. Without this the `~` stays literal, the path
    // is relative, and it silently resolves against the working directory.
    if let Some(rest) = path.strip_prefix("~\\")
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

/// Inverse of `expand_tilde`: replace a leading home-dir prefix in an
/// absolute path with `~`. Falls back to the original path string when
/// no replacement applies.
///
/// Used everywhere a path lands in user-visible output OR in the
/// model's system prompt — the goal is twofold:
///   1. Don't leak the local username (`/Users/$you/srv/...`) into
///      every prompt; that's a privacy/identity leak that also varies
///      between machines, hurting prompt-cache hit rates.
///   2. Save tokens — `~/srv/myapp/...` is consistently shorter
///      than `/Users/alice/srv/myapp/...`.
pub fn collapse_home(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_string();
        }
        let suffix = rest.display().to_string();
        if let Some(stripped) = suffix.strip_prefix('/') {
            return format!("~/{}", stripped);
        }
        return format!("~/{}", suffix);
    }
    path.display().to_string()
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

/// Best-effort human name for a non-regular file type. Errors name the
/// kind so a model that probed a device file gets "character device" back
/// and does not retry the same class with a different path (#1164).
fn describe_file_type(md: &std::fs::Metadata) -> &'static str {
    let ft = md.file_type();
    if ft.is_dir() {
        return "directory";
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if ft.is_char_device() {
            return "character device";
        }
        if ft.is_block_device() {
            return "block device";
        }
        if ft.is_fifo() {
            return "FIFO (named pipe)";
        }
        if ft.is_socket() {
            return "socket";
        }
    }
    "special file"
}

/// Strip markdown/backtick/quote wrappers from a model-supplied path.
///
/// Models sometimes emit paths as `**/tmp/f.rs**` or `` `/tmp/f.rs` `` when
/// echoing formatted text back. Returns the inner slice when both ends carry
/// the same wrapper pair, otherwise the trimmed input unchanged.
pub fn strip_path_wrappers(raw: &str) -> &str {
    let t = raw.trim();
    for (open, close) in [("**", "**"), ("`", "`"), ("\"", "\""), ("'", "'")] {
        if t.len() >= open.len() + close.len()
            && let Some(inner) = t.strip_prefix(open).and_then(|s| s.strip_suffix(close))
            && !inner.is_empty()
        {
            return inner.trim();
        }
    }
    t
}

/// Case-insensitive Levenshtein distance for fuzzy filename suggestions.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Build a self-describing "file not found" error with fuzzy suggestions.
///
/// Scans up to 50 sibling entries of the parent directory and appends the
/// closest name matches (max 3, within a similarity threshold), so the model
/// can self-correct instead of retrying blind. A missing or unreadable
/// parent keeps the plain error: nothing to suggest from.
fn missing_file_hint(path: &std::path::Path) -> String {
    let base = format!("File not found: {}", path.display());
    let Some(parent) = path.parent() else {
        return base;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return base;
    };
    let target = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if target.is_empty() {
        return base;
    }
    let mut candidates: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .take(50)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    candidates.sort_by_key(|name| edit_distance(name, &target));
    let suggestions: Vec<String> = candidates
        .iter()
        .filter(|name| edit_distance(name, &target) * 8 <= name.len().max(target.len()) * 5)
        .take(3)
        .cloned()
        .collect();
    if suggestions.is_empty() {
        return base;
    }
    format!(
        "{} — did you mean: {}?",
        base,
        suggestions
            .iter()
            .map(|s| format!("'{}'", s))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Resolve a path, check it exists, and confirm it's a file.
///
/// Returns a user-friendly error message suitable for ToolResult::error()
pub fn validate_file_path(
    requested_path: &str,
    working_directory: &std::path::Path,
) -> std::result::Result<std::path::PathBuf, String> {
    // Models sometimes wrap paths in markdown formatting (`**path**`,
    // backticks, quotes). If the unwrapped form resolves to a regular file,
    // succeed with it directly instead of failing on decoration.
    let stripped = strip_path_wrappers(requested_path);
    if stripped != requested_path.trim()
        && let Ok(p) = validate_path_safety(stripped, working_directory)
        && p.is_file()
    {
        return Ok(p);
    }

    let path = match validate_path_safety(requested_path, working_directory) {
        Ok(p) => p,
        Err(ToolError::InvalidInput(msg)) => {
            // Absence miss (nested missing parents): safety rejects it as
            // InvalidInput, but the model needs the self-describing
            // not-found shape, not a validation wrapper (#1169). Only
            // genuine absence converts, re-derived here rather than
            // string-matched, so security rejections keep their message.
            let resolved = resolve_tool_path(stripped, working_directory);
            if !resolved.exists() && resolved.parent().is_some_and(|p| !p.exists()) {
                return Err(missing_file_hint(&resolved));
            }
            return Err(format!("Invalid path: {}", msg));
        }
        Err(e) => {
            return Err(format!("Path validation failed: {}", e));
        }
    };

    if !path.exists() {
        return Err(missing_file_hint(&path));
    }

    if !path.is_file() {
        let kind = std::fs::metadata(&path)
            .map(|md| describe_file_type(&md))
            .unwrap_or("special file");
        return Err(format!(
            "Path is not a regular file: {} ({}) — file tools read regular \
             files only; devices, pipes and directories are not readable",
            path.display(),
            kind
        ));
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
