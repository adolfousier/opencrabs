//! Tool input sanitization for safe display in approval messages and UI.
//!
//! When the agent calls tools like `http_request` or `bash`, the inputs may
//! contain API keys, Authorization headers, passwords, or tokens. This module
//! redacts those values before they are shown to users in Telegram/WhatsApp
//! approval dialogs or the TUI, while preserving enough context (field names,
//! non-sensitive values) for the user to understand what the tool is doing.

use serde_json::{Map, Value};

/// Field name patterns (case-insensitive) whose values are always redacted.
const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "api_key",
    "apikey",
    "api-key",
    "x-api-key",
    "x-auth-token",
    "x-access-token",
    "token",
    "secret",
    "password",
    "passwd",
    "pass",
    "credential",
    "credentials",
    "access_token",
    "refresh_token",
    "client_secret",
    "private_key",
    "auth",
    "bearer",
];

/// Regex-like patterns in bash commands to redact inline secrets.
/// Each tuple is (prefix_to_find, chars_to_keep_after_prefix).
/// We redact the rest of the token after these prefixes.
const COMMAND_SENSITIVE_PATTERNS: &[&str] = &[
    "bearer ",
    "authorization: ",
    "x-api-key: ",
    "x-auth-token: ",
    "api_key=",
    "apikey=",
    "api-key=",
    "token=",
    "secret=",
    "password=",
    "passwd=",
    "access_token=",
];

/// Returns true if a JSON object key looks like it holds a sensitive value.
fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SENSITIVE_KEYS.iter().any(|&pat| lower == pat || lower.contains(pat))
}

/// Redact sensitive values from a bash command string.
/// Handles patterns like:
///   -H "Authorization: Bearer sk-xxx"
///   --header "X-Api-Key: abc123"
///   https://user:password@host/path
///   api_key=abc123
fn redact_command(cmd: &str) -> String {
    let mut result = cmd.to_string();

    // Redact URL passwords: https://user:PASSWORD@host → https://user:[REDACTED]@host
    // Simple approach: find ://word:word@ patterns
    if let Some(at_pos) = result.find("://") {
        let rest = &result[at_pos + 3..];
        if let Some(at_sign) = rest.find('@')
            && let Some(colon) = rest[..at_sign].find(':')
        {
            let pass_start = at_pos + 3 + colon + 1;
            let pass_end = at_pos + 3 + at_sign;
            if pass_start < pass_end && pass_end <= result.len() {
                result.replace_range(pass_start..pass_end, "[REDACTED]");
            }
        }
    }

    // Redact inline header values and query params (case-insensitive)
    let lower = result.to_lowercase();
    for pattern in COMMAND_SENSITIVE_PATTERNS {
        let mut search_start = 0;
        while let Some(pos) = lower[search_start..].find(pattern) {
            let match_pos = search_start + pos + pattern.len();
            // Find end of the secret: whitespace, quote, or end of string
            let secret_end = result[match_pos..]
                .find(['"', '\'', ' ', '&', '\n'])
                .map(|p| match_pos + p)
                .unwrap_or(result.len());
            if secret_end > match_pos {
                result.replace_range(match_pos..secret_end, "[REDACTED]");
            }
            // Advance past the pattern to avoid infinite loop
            search_start = match_pos;
            if search_start >= result.len() {
                break;
            }
        }
    }

    result
}

/// Recursively redact sensitive fields from a tool input JSON value.
///
/// - Object keys matching `SENSITIVE_KEYS` have their string values replaced
///   with `"[REDACTED]"`
/// - The `command` field (bash) has inline secret patterns redacted
/// - The `headers` object has all values for sensitive header names redacted
/// - Arrays and nested objects are recursively processed
pub fn redact_tool_input(value: &Value) -> Value {
    redact_value(value, None)
}

fn redact_value(value: &Value, parent_key: Option<&str>) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                let redacted = if is_sensitive_key(k) {
                    // Redact the value regardless of type
                    Value::String("[REDACTED]".to_string())
                } else if k == "command" {
                    // Bash command: apply inline pattern redaction
                    match v.as_str() {
                        Some(cmd) => Value::String(redact_command(cmd)),
                        None => redact_value(v, Some(k)),
                    }
                } else if k == "headers" {
                    // Headers object: redact values for sensitive header names
                    redact_headers_object(v)
                } else if k == "query" || k == "params" {
                    // Query params object: redact sensitive param values
                    redact_value(v, Some(k))
                } else if k == "url" {
                    // URLs may have passwords embedded
                    match v.as_str() {
                        Some(url) => Value::String(redact_command(url)),
                        None => redact_value(v, Some(k)),
                    }
                } else {
                    redact_value(v, Some(k))
                };
                out.insert(k.clone(), redacted);
            }
            Value::Object(out)
        }
        Value::Array(arr) => {
            // If parent key is sensitive, redact the whole array
            if parent_key.map(is_sensitive_key).unwrap_or(false) {
                Value::String("[REDACTED]".to_string())
            } else {
                Value::Array(arr.iter().map(|v| redact_value(v, None)).collect())
            }
        }
        Value::String(s) => {
            if parent_key.map(is_sensitive_key).unwrap_or(false) {
                Value::String("[REDACTED]".to_string())
            } else {
                Value::String(s.clone())
            }
        }
        other => other.clone(),
    }
}

/// Redact values inside a headers object for known sensitive header names.
fn redact_headers_object(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                let redacted = if is_sensitive_key(k) {
                    Value::String("[REDACTED]".to_string())
                } else {
                    v.clone()
                };
                out.insert(k.clone(), redacted);
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_authorization_header() {
        let input = json!({
            "method": "POST",
            "url": "https://api.trello.com/1/cards",
            "headers": {
                "Authorization": "Bearer sk-trello-abc123",
                "Content-Type": "application/json"
            }
        });
        let out = redact_tool_input(&input);
        assert_eq!(out["headers"]["Authorization"], "[REDACTED]");
        assert_eq!(out["headers"]["Content-Type"], "application/json");
    }

    #[test]
    fn redacts_api_key_field() {
        let input = json!({"api_key": "secret123", "query": "something"});
        let out = redact_tool_input(&input);
        assert_eq!(out["api_key"], "[REDACTED]");
        assert_eq!(out["query"], "something");
    }

    #[test]
    fn redacts_bash_bearer_token() {
        let input = json!({
            "command": "curl -H \"Authorization: Bearer sk-abc123\" https://api.example.com"
        });
        let out = redact_tool_input(&input);
        let cmd = out["command"].as_str().unwrap();
        assert!(cmd.contains("[REDACTED]"), "expected REDACTED in: {cmd}");
        assert!(!cmd.contains("sk-abc123"), "secret still present: {cmd}");
    }

    #[test]
    fn redacts_url_password() {
        let input = json!({
            "url": "https://user:mysecretpass@api.example.com/v1"
        });
        let out = redact_tool_input(&input);
        let url = out["url"].as_str().unwrap();
        assert!(url.contains("[REDACTED]"), "expected REDACTED in: {url}");
        assert!(!url.contains("mysecretpass"), "password still present: {url}");
    }

    #[test]
    fn preserves_non_sensitive_fields() {
        let input = json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "timeout_secs": 30
        });
        let out = redact_tool_input(&input);
        assert_eq!(out["method"], "GET");
        assert_eq!(out["timeout_secs"], 30);
    }
}
