//! Tool context hint extraction for channel display.
//!
//! Extracts a short, meaningful description from a tool's input parameters
//! so channels can show what each tool call is doing (e.g. "grep (`pattern`)").

use super::sanitize::redact_tool_input;
use super::string::truncate_str;

/// Extract a short, meaningful context hint from a tool's input for channel display.
/// Runs the input through the secret sanitizer first so no API keys or tokens
/// can leak into the streaming indicator via command or url fields.
/// Returns a formatted string like ` ("hint")` or empty string if no hint found.
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
        "cron_manage" => {
            let action = safe.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let name = safe.get("name").and_then(|v| v.as_str());
            match (action, name) {
                ("list", _) => Some("list jobs".to_string()),
                (act, Some(n)) => Some(format!("{} '{}'", act, n)),
                (act, None) => Some(act.to_string()),
            }
        }
        "plan" => {
            let op = safe.get("operation").and_then(|v| v.as_str()).unwrap_or("?");
            let title = safe
                .get("title")
                .or_else(|| safe.get("name"))
                .and_then(|v| v.as_str());
            match title {
                Some(t) => Some(format!("{}: {}", op, t)),
                None => Some(op.to_string()),
            }
        }
        "task_manager" => {
            let op = safe.get("operation").and_then(|v| v.as_str()).unwrap_or("?");
            let title = safe.get("title").and_then(|v| v.as_str());
            match title {
                Some(t) => Some(format!("{}: {}", op, t)),
                None => Some(op.to_string()),
            }
        }
        "lsp" => safe
            .get("operation")
            .and_then(|v| v.as_str())
            .map(String::from),
        // Fallback: build "action: detail" from common field patterns
        _ => safe.as_object().and_then(|m| {
            let action = m
                .get("action")
                .or_else(|| m.get("operation"))
                .and_then(|v| v.as_str());
            let detail_keys = [
                "name", "prompt", "query", "path", "file_path", "pattern",
                "description", "title", "url", "command", "id", "job_id",
            ];
            let detail = detail_keys
                .iter()
                .find_map(|k| m.get(*k).and_then(|v| v.as_str()));

            match (action, detail) {
                (Some(act), Some(det)) => Some(format!("{}: {}", act, det)),
                (None, Some(det)) => Some(det.to_string()),
                (Some(act), None) => {
                    // Action-only — find any other string field
                    let other = m
                        .iter()
                        .find(|(k, v)| *k != "action" && *k != "operation" && v.is_string())
                        .and_then(|(_, v)| v.as_str());
                    match other {
                        Some(o) => Some(format!("{}: {}", act, o)),
                        None => Some(act.to_string()),
                    }
                }
                (None, None) => m.values().find_map(|v| match v {
                    serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                }),
            }
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
