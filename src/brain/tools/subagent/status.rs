//! Sub-agent progress streaming via JSON status files.
//!
//! Each sub-agent writes its state/progress to
//! `~/.opencrabs/tmp/subagents/<agent_id>.json`. The main orchestrator
//! can `read_file` these at any time for real-time visibility — no
//! `session_search` needed.
//!
//! Files older than 7 days are cleaned up on startup.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Base directory for all sub-agent status files.
pub fn status_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".opencrabs")
        .join("tmp")
        .join("subagents")
}

/// Ensure the status directory exists.
pub fn ensure_dir() -> std::io::Result<()> {
    let dir = status_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(())
}

/// Path to a specific sub-agent's status file.
pub fn status_path(agent_id: &str) -> PathBuf {
    status_dir().join(format!("{}.json", agent_id))
}

// ── Status data types ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentState {
    Pending,
    Running,
    Completed,
    Failed,
}

/// Snapshot of the latest tool-use event in a running sub-agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProgressSnapshot {
    #[serde(default = "usize::default")]
    pub iteration: usize,
    #[serde(default)]
    pub last_tool: Option<String>,
    #[serde(default)]
    pub last_event: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Persisted status of a single sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub id: String,
    pub label: String,
    pub parent_session_id: String,
    pub state: AgentState,
    pub prompt: String,
    pub started_at: String,
    #[serde(default)]
    pub progress: Option<ProgressSnapshot>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub output_summary: Option<String>,
}

impl AgentStatus {
    /// Create a new status in `Pending` state and write the JSON file.
    pub fn new(
        agent_id: &str,
        label: &str,
        parent_session_id: &str,
        prompt: &str,
    ) -> std::io::Result<Self> {
        ensure_dir()?;
        let now = now_rfc3339();
        let status = Self {
            id: agent_id.to_string(),
            label: label.to_string(),
            parent_session_id: parent_session_id.to_string(),
            state: AgentState::Pending,
            prompt: prompt.to_string(),
            started_at: now.clone(),
            progress: None,
            completed_at: None,
            error: None,
            output_summary: None,
        };
        status.write()?;
        Ok(status)
    }

    /// Transition to `Running`.
    pub fn mark_running(&mut self) -> std::io::Result<()> {
        self.state = AgentState::Running;
        self.write()
    }

    /// Update the progress snapshot after each tool-loop iteration.
    pub fn update_progress(
        &mut self,
        iteration: usize,
        last_tool: Option<String>,
        last_event: Option<String>,
    ) -> std::io::Result<()> {
        self.progress = Some(ProgressSnapshot {
            iteration,
            last_tool,
            last_event,
            updated_at: Some(now_rfc3339()),
        });
        self.write()
    }

    /// Mark the agent as completed with a short output summary.
    pub fn mark_completed(&mut self, output_summary: String) -> std::io::Result<()> {
        self.state = AgentState::Completed;
        self.completed_at = Some(now_rfc3339());
        self.output_summary = Some(output_summary);
        self.write()
    }

    /// Mark the agent as failed with an error message.
    pub fn mark_failed(&mut self, error: String) -> std::io::Result<()> {
        self.state = AgentState::Failed;
        self.completed_at = Some(now_rfc3339());
        self.error = Some(error);
        self.write()
    }

    /// Read the persisted status for an agent, if the file exists.
    pub fn read(agent_id: &str) -> Option<Self> {
        let path = status_path(agent_id);
        if !path.exists() {
            return None;
        }
        let data = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Persist status to disk. Uses atomic rename for crash safety.
    fn write(&self) -> std::io::Result<()> {
        let path = status_path(&self.id);
        ensure_dir()?;
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_string_pretty(self)
            .map_err(std::io::Error::other)?;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
        fs::rename(tmp, path)
    }

    /// List all known sub-agent status files (by agent_id).
    pub fn list_all() -> std::io::Result<Vec<String>> {
        let dir = status_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str()
                && let Some(id) = name.strip_suffix(".json") {
                    ids.push(id.to_string());
                }
        }
        ids.sort();
        Ok(ids)
    }
}

// ── Auto-cleanup ─────────────────────────────────────────────────────

/// Remove status files whose `completed_at` is older than `max_age`
/// or whose on-disk mtime is older than `max_age` (for files without
/// a `completed_at` field — covers old/corrupted files).
pub fn cleanup_stale(max_age: Duration) -> std::io::Result<(usize, usize)> {
    let dir = status_dir();
    if !dir.exists() {
        return Ok((0, 0));
    }

    let cutoff = SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut scanned = 0usize;
    let mut removed = 0usize;

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        scanned += 1;

        let should_delete = if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(status) = serde_json::from_str::<AgentStatus>(&data) {
                status.completed_at.as_ref().is_some_and(|ts| parse_completed_at(&cutoff, ts))
                    || status.completed_at.is_none() && file_stale(&path, &cutoff)
            } else {
                file_stale(&path, &cutoff)
            }
        } else {
            file_stale(&path, &cutoff)
        };

        if should_delete {
            fs::remove_file(&path)?;
            removed += 1;
        }
    }

    Ok((scanned, removed))
}

fn parse_completed_at(cutoff: &SystemTime, ts: &str) -> bool {
    // Naïve UTC parser — enough for RFC3339 without subseconds.
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) else {
        return false; // can't parse — skip, let cleanup catch it later
    };
    let completed = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(dt.timestamp() as u64))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    completed < *cutoff
}

fn file_stale(path: &Path, cutoff: &SystemTime) -> bool {
    path.metadata()
        .and_then(|m| m.modified())
        .map(|mtime| mtime < *cutoff)
        .unwrap_or(true) // can't stat → delete to be safe
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_dir_returns_correct_path() {
        let home = dirs::home_dir().unwrap_or_default();
        let expected = home.join(".opencrabs").join("tmp").join("subagents");
        assert_eq!(status_dir(), expected);
    }

    #[test]
    fn status_path_ends_with_json() {
        let p = status_path("abc123");
        assert_eq!(
            p.file_name().unwrap().to_str().unwrap(),
            "abc123.json"
        );
    }

    #[test]
    fn new_status_is_pending() {
        let s = AgentStatus::new("test-1", "test", "sess-1", "do things").unwrap();
        assert_eq!(s.state, AgentState::Pending);
        assert_eq!(s.id, "test-1");
        assert_eq!(s.label, "test");
    }

    #[test]
    fn status_transitions_to_running() {
        let mut s = AgentStatus::new("test-2", "test", "sess-1", "do things").unwrap();
        s.mark_running().unwrap();
        assert_eq!(s.state, AgentState::Running);
    }

    #[test]
    fn status_progress_snapshot() {
        let mut s = AgentStatus::new("test-3", "test", "sess-1", "do things").unwrap();
        s.mark_running().unwrap();
        s.update_progress(1, Some("bash".into()), Some("cargo check ok".into()))
            .unwrap();
        assert!(s.progress.is_some());
        let p = s.progress.unwrap();
        assert_eq!(p.iteration, 1);
        assert_eq!(p.last_tool, Some("bash".to_string()));
        assert_eq!(p.last_event, Some("cargo check ok".to_string()));
    }

    #[test]
    fn status_completed_sets_timestamp() {
        let mut s = AgentStatus::new("test-4", "test", "sess-1", "do things").unwrap();
        s.mark_completed("done".into()).unwrap();
        assert_eq!(s.state, AgentState::Completed);
        assert!(s.completed_at.is_some());
        assert_eq!(s.output_summary, Some("done".to_string()));
    }

    #[test]
    fn status_failed_sets_error() {
        let mut s = AgentStatus::new("test-5", "test", "sess-1", "do things").unwrap();
        s.mark_failed("something broke".into()).unwrap();
        assert_eq!(s.state, AgentState::Failed);
        assert_eq!(s.error, Some("something broke".to_string()));
        assert!(s.completed_at.is_some());
    }

    #[test]
    fn status_read_roundtrip() {
        let mut s = AgentStatus::new("test-6", "test", "sess-1", "do things").unwrap();
        s.mark_running().unwrap();
        s.update_progress(2, Some("write_file".into()), None)
            .unwrap();

        let read = AgentStatus::read("test-6").expect("should read back");
        assert_eq!(read.id, "test-6");
        assert_eq!(read.state, AgentState::Running);
        assert_eq!(read.progress.unwrap().iteration, 2);
    }

    #[test]
    fn cleanup_removes_old_files() {
        let _ = fs::remove_dir_all(status_dir()); // start clean
        let mut s = AgentStatus::new("old-1", "old", "sess", "task").unwrap();
        s.mark_completed("done".into()).unwrap();

        // Override completed_at to be 8 days ago.
        let old_ts = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(8))
            .unwrap()
            .to_rfc3339();
        let mut raw = fs::read_to_string(status_path("old-1")).unwrap();
        let mut parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        parsed["completed_at"] = serde_json::json!(old_ts);
        raw = serde_json::to_string_pretty(&parsed).unwrap();
        fs::write(status_path("old-1"), raw).unwrap();

        let cleanup_result = cleanup_stale(Duration::from_secs(7 * 86400)).unwrap();
        assert!(cleanup_result.1 >= 1, "should have removed at least 1 file");
        assert!(AgentStatus::read("old-1").is_none());
    }
}
