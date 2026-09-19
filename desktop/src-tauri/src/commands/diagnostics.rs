use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

const MAX_LOG_BYTES: u64 = 128 * 1024;
const MAX_LOG_LINES: usize = 120;

#[derive(Serialize)]
pub struct DiagnosticsSnapshot {
    pub app_version: String,
    pub config_present: bool,
    pub database_present: bool,
    pub log_path: String,
    pub log_tail: Vec<String>,
    pub notes: Vec<String>,
}

fn today_log_path() -> PathBuf {
    let date = Utc::now().format("%Y-%m-%d");
    opencrabs::config::opencrabs_home()
        .join("logs")
        .join(format!("opencrabs.{date}"))
}

fn sensitive_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "api_key",
        "secret",
        "password",
        "api-key",
        "x-api-key",
        "authorization:",
        "bearer ",
        "token=",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn safe_log_tail(path: &PathBuf) -> Result<Vec<String>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(format!("Unable to inspect diagnostics log: {error}")),
    };
    if metadata.len() > MAX_LOG_BYTES {
        return Ok(vec![format!(
            "Log preview omitted: {} bytes exceeds {} byte limit",
            metadata.len(),
            MAX_LOG_BYTES
        )]);
    }

    let content = fs::read_to_string(path)
        .map_err(|error| format!("Unable to read diagnostics log: {error}"))?;
    Ok(content
        .lines()
        .rev()
        .filter(|line| !sensitive_line(line))
        .take(MAX_LOG_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(str::to_owned)
        .collect())
}

fn snapshot() -> Result<DiagnosticsSnapshot, String> {
    let home = opencrabs::config::opencrabs_home();
    let log_path = today_log_path();
    let mut notes = vec![
        "Diagnostic previews redact common credential-bearing log lines.".to_string(),
        "Use the production release runbook before attaching diagnostics to an issue.".to_string(),
    ];
    let log_tail = safe_log_tail(&log_path)?;
    if log_tail.is_empty() {
        notes.push("No readable log entries found for today.".to_string());
    }

    Ok(DiagnosticsSnapshot {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        config_present: home.join("config.toml").exists(),
        database_present: home.join("opencrabs.db").exists(),
        log_path: log_path.to_string_lossy().to_string(),
        log_tail,
        notes,
    })
}

#[tauri::command]
pub async fn get_diagnostics() -> Result<DiagnosticsSnapshot, String> {
    snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_bearing_log_lines_are_excluded() {
        assert!(sensitive_line("api_key=shh"));
        assert!(sensitive_line("Authorization: Bearer shh"));
        assert!(sensitive_line("password=shh"));
        assert!(sensitive_line("x-api-key: shh"));
        assert!(!sensitive_line("request completed in 4ms"));
    }

    #[test]
    fn missing_log_is_a_valid_empty_preview() {
        let missing = std::env::temp_dir().join("opencrabs-desktop-no-log");
        assert!(
            safe_log_tail(&missing)
                .expect("missing log should be harmless")
                .is_empty()
        );
    }
}
