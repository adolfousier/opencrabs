//! Utility modules for common functionality

pub mod approval;
pub mod config_watcher;
pub mod file_extract;
pub mod image;
pub mod install;
pub mod retry;
pub mod sanitize;
mod string;

pub use approval::{
    check_approval_policy, persist_auto_always_policy, persist_auto_session_policy,
};
pub use file_extract::{FileContent, classify_file};
pub use image::extract_img_markers;
pub use retry::{RetryConfig, RetryableError, retry, retry_with_check};
pub use sanitize::redact_tool_input;
pub use string::truncate_str;

/// Extract a short, meaningful context hint from a tool's input for channel display.
/// Runs the input through the secret sanitizer first so no API keys or tokens
/// can leak into the streaming indicator via command or url fields.
/// Returns a formatted string like `("hint")` or empty string if no hint found.
pub fn tool_context_hint(name: &str, input: &serde_json::Value) -> String {
    let safe = redact_tool_input(input);
    let hint: Option<String> = match name {
        "bash" => safe
            .get("command")
            .and_then(|v| v.as_str())
            .map(String::from),
        "read" | "read_file" | "write" | "write_file" | "edit" | "edit_file" => safe
            .get("path")
            .or_else(|| safe.get("file_path"))
            .and_then(|v| v.as_str())
            .map(String::from),
        "glob" => safe
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(String::from),
        "grep" => safe
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(String::from),
        "ls" => safe.get("path").and_then(|v| v.as_str()).map(String::from),
        "http_request" | "web_fetch" => safe.get("url").and_then(|v| v.as_str()).map(String::from),
        "brave_search" | "exa_search" | "web_search" | "memory_search" | "session_search" => {
            safe.get("query").and_then(|v| v.as_str()).map(String::from)
        }
        "telegram_send" | "discord_send" | "slack_send" | "trello_send" => safe
            .get("action")
            .and_then(|v| v.as_str())
            .map(String::from),
        "agent" | "Agent" => safe
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        "plan" => safe
            .get("operation")
            .and_then(|v| v.as_str())
            .map(String::from),
        "task_manager" => safe
            .get("operation")
            .and_then(|v| v.as_str())
            .map(String::from),
        "lsp" => safe
            .get("operation")
            .and_then(|v| v.as_str())
            .map(String::from),
        // Fallback: first string value, or first scalar stringified
        _ => safe.as_object().and_then(|m| {
            m.values()
                .find_map(|v| v.as_str().map(String::from))
                .or_else(|| {
                    m.values().find_map(|v| match v {
                        serde_json::Value::Number(n) => Some(n.to_string()),
                        serde_json::Value::Bool(b) => Some(b.to_string()),
                        _ => None,
                    })
                })
        }),
    };
    match hint {
        Some(h) if !h.is_empty() => {
            let truncated = truncate_str(&h, 60);
            format!(" (`{truncated}`)")
        }
        _ => String::new(),
    }
}
