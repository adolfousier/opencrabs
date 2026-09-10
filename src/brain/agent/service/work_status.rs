//! Unified status files for detached work (#26 P1).
//!
//! One JSON status file per unit of detached work — both detached bash
//! commands (#1160) and spawned sub-agents (#1038) — at
//! `<opencrabs_home>/tmp/detached/<id>.json`. Before #26 this was two
//! copy-pasted modules (`service/detached_status.rs` for commands,
//! `tools/subagent/status.rs` for agents); the pattern is now shared: spawn
//! writes the record, the worker updates state/progress, and the terminal
//! transition stamps a `finish`. The main orchestrator can `read_file` these
//! at any time for real-time visibility — no `session_search` needed.
//!
//! Files older than 7 days are cleaned up on startup (both kinds now age out
//! — command files previously had no sweep at all).
//!
//! Legacy compat: pre-#26 binaries wrote agent files to `tmp/subagents/` in
//! the old schema. [`migrate_legacy_dir`] moves them into the unified dir at
//! startup, converting the schema, so an agent in flight across the binary
//! swap is still reconciled and reported (#1038). Legacy COMMAND files
//! already live in `tmp/detached/` but carry the old `DetachedTaskStatus`
//! schema and do not parse here; that is harmless — command interruption
//! recovery reads the DB row (#763), never the file — and they age out via
//! the mtime sweep.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Base directory for all detached-work status files.
pub fn status_dir() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(p) = test_override::get() {
            return p;
        }
    }
    crate::config::opencrabs_home().join("tmp").join("detached")
}

/// Where pre-#26 binaries wrote sub-agent status files: always the
/// `subagents` sibling of [`status_dir`].
pub fn legacy_dir() -> PathBuf {
    status_dir().with_file_name("subagents")
}

#[cfg(test)]
pub(crate) mod test_override {
    use std::cell::RefCell;
    use std::path::PathBuf;
    thread_local! {
        static DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }
    pub fn set(p: PathBuf) {
        DIR.with(|d| *d.borrow_mut() = Some(p));
    }
    pub fn get() -> Option<PathBuf> {
        DIR.with(|d| d.borrow().clone())
    }
    /// Reset the override so the production path is visible again — used by
    /// tests that assert on the real dir layout. Thread-local overrides
    /// otherwise persist across the test-runner's reused threads.
    pub fn clear() {
        DIR.with(|d| *d.borrow_mut() = None);
    }
}

/// Ensure the status directory exists.
pub fn ensure_dir() -> std::io::Result<()> {
    let dir = status_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(())
}

/// Path to a specific work item's status file.
pub fn status_path(id: &str) -> PathBuf {
    status_dir().join(format!("{}.json", id))
}

// ── Status data types ────────────────────────────────────────────────

/// What produced one unit of detached work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkKind {
    /// A detached bash command (#1160). Never enters
    /// [`WorkState::AwaitingInput`].
    Command,
    /// A spawned sub-agent (#1038).
    Agent,
}

impl Default for WorkKind {
    /// A file without a `kind` field was written by pre-#26 sub-agent code —
    /// those are all agents.
    fn default() -> Self {
        WorkKind::Agent
    }
}

/// Shared state machine for both kinds of detached work (#26).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkState {
    Pending,
    Running,
    /// Parked at a round boundary awaiting follow-up input (#1183): the
    /// round's work is done and its output is collectable, but the agent has
    /// not terminated. Written so a parked file stops reading `Running`,
    /// which misled every consumer into waiting on work that was already
    /// finished. Not terminal — [`Self::is_terminal`] is false. Agents only;
    /// commands never enter this state.
    AwaitingInput,
    Completed,
    Failed,
    /// The process that owned this work died before it reached a terminal
    /// state. Distinct from `Failed`: the work did not fail, it never
    /// finished, and nothing knows how far it got. Callers must not read it
    /// as either success or failure.
    Interrupted,
}

impl WorkState {
    /// Whether this state means the work is no longer doing anything.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WorkState::Completed | WorkState::Failed | WorkState::Interrupted
        )
    }
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

/// Terminal outcome of one unit of detached work.
///
/// Commands fill the exit side (`success`/`code`/`elapsed_secs`/
/// `output_bytes`, #1160), agents the outcome side (`error`/
/// `output_full`, #147). `completed_at` is shared and is what the
/// stale sweep ages files out by.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkFinish {
    /// RFC3339 time of the terminal transition.
    pub completed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The COMPLETE final output of a finished agent (#147). Replaces the
    /// 200-char `output_summary` stub — zero production code paths ever
    /// read the summary's content, so the full report replaces it as the
    /// only persisted record. `None` for commands and legacy files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_full: Option<String>,
}

/// Exit information for a detached command (#1160). Bundled so
/// [`WorkStatus::finish_command`] stays under the clippy argument limit —
/// same shape as the pre-#26 `DetachedFinish`.
#[derive(Debug, Clone, Copy)]
pub struct CommandExit {
    pub success: bool,
    pub code: i32,
    pub elapsed_secs: f32,
    pub output_bytes: usize,
}

fn finish_now() -> WorkFinish {
    WorkFinish {
        completed_at: now_rfc3339(),
        success: None,
        code: None,
        elapsed_secs: None,
        output_bytes: None,
        error: None,
        output_full: None,
    }
}

/// Persisted status of one unit of detached work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkStatus {
    pub id: String,
    #[serde(default)]
    pub kind: WorkKind,
    /// The session recorded at spawn. For agents this is what recovery
    /// routes the interruption report by (the pre-#26 `parent_session_id`
    /// value); for commands, the owning session.
    pub session_id: String,
    /// The session that spawned this agent (#110). `None` for legacy files
    /// written before the field existed and for detached commands. The
    /// boot-resume path uses it to deliver a revived agent's result to the
    /// session that is waiting on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub label: String,
    /// The work itself: the shell command for [`WorkKind::Command`], the
    /// prompt for [`WorkKind::Agent`].
    pub task: String,
    /// RFC3339 time of spawn.
    pub spawned_at: String,
    pub state: WorkState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<WorkFinish>,
}

impl WorkStatus {
    /// Create a sub-agent status in `Pending` state and write the JSON file
    /// (#1038).
    pub fn new_agent(
        id: &str,
        label: &str,
        session_id: &str,
        prompt: &str,
        parent_session_id: Option<&str>,
    ) -> std::io::Result<Self> {
        ensure_dir()?;
        let status = Self {
            id: id.to_string(),
            kind: WorkKind::Agent,
            session_id: session_id.to_string(),
            parent_session_id: parent_session_id.map(str::to_string),
            label: label.to_string(),
            task: prompt.to_string(),
            spawned_at: now_rfc3339(),
            state: WorkState::Pending,
            progress: None,
            finish: None,
        };
        status.write()?;
        Ok(status)
    }

    /// Create a detached-command status in `Running` state and write the
    /// JSON file (#1160). A command goes straight to work — no Pending
    /// phase — and it never enters `AwaitingInput`.
    pub fn new_command(
        id: &str,
        session_id: &str,
        label: &str,
        command: &str,
    ) -> std::io::Result<Self> {
        ensure_dir()?;
        let status = Self {
            id: id.to_string(),
            kind: WorkKind::Command,
            session_id: session_id.to_string(),
            label: label.to_string(),
            task: command.to_string(),
            spawned_at: now_rfc3339(),
            state: WorkState::Running,
            progress: None,
            finish: None,
            parent_session_id: None,
        };
        status.write()?;
        Ok(status)
    }

    /// Rewrite a command record with exit information (#1160). Falls back
    /// to a minimal fresh record if the spawn write never landed.
    pub fn finish_command(
        id: &str,
        session_id: &str,
        label: &str,
        command: &str,
        exit: CommandExit,
    ) -> std::io::Result<()> {
        let mut status = Self::read(id).unwrap_or_else(|| Self {
            id: id.to_string(),
            kind: WorkKind::Command,
            session_id: session_id.to_string(),
            label: label.to_string(),
            task: command.to_string(),
            spawned_at: now_rfc3339(),
            state: WorkState::Running,
            progress: None,
            finish: None,
            parent_session_id: None,
        });
        status.state = if exit.success {
            WorkState::Completed
        } else {
            WorkState::Failed
        };
        let mut finish = finish_now();
        finish.success = Some(exit.success);
        finish.code = Some(exit.code);
        finish.elapsed_secs = Some(exit.elapsed_secs);
        finish.output_bytes = Some(exit.output_bytes);
        status.finish = Some(finish);
        status.write()
    }

    /// Transition to `Running`.
    pub fn mark_running(&mut self) -> std::io::Result<()> {
        self.state = WorkState::Running;
        self.write()
    }

    /// Park at a round boundary awaiting follow-up input (#1183). Mirrors
    /// `SubAgentManager::mark_awaiting_input`: before this the file kept
    /// reading `Running` with no finish while the agent sat idle, so audits
    /// kept waiting on work that was already finished
    /// (`state: "Running"` on a parked agent, #1183). No finish is stamped —
    /// this is not a terminal state, and flipping back via
    /// [`Self::mark_running`] when input arrives is a normal transition.
    pub fn mark_awaiting_input(&mut self) -> std::io::Result<()> {
        self.state = WorkState::AwaitingInput;
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

    /// Mark the work completed with the COMPLETE final output (#147).
    pub fn mark_completed(&mut self, output_full: String) -> std::io::Result<()> {
        self.state = WorkState::Completed;
        let mut finish = finish_now();
        finish.output_full = Some(output_full);
        self.finish = Some(finish);
        self.write()
    }

    /// Mark the work failed with an error message.
    pub fn mark_failed(&mut self, error: String) -> std::io::Result<()> {
        self.state = WorkState::Failed;
        let mut finish = finish_now();
        finish.error = Some(error);
        self.finish = Some(finish);
        self.write()
    }

    /// Mark the work interrupted by a restart.
    ///
    /// A finish is stamped so [`cleanup_stale`] can age the file out on the
    /// same schedule as any other terminal state. Its `error` carries the
    /// reason rather than being left empty, so anything reading the file can
    /// tell a restart apart from a genuine failure. The wording is
    /// kind-aware but keeps the exact agent sentence from before #26.
    pub fn mark_interrupted(&mut self) -> std::io::Result<()> {
        self.state = WorkState::Interrupted;
        let subject = match self.kind {
            WorkKind::Command => "this command",
            WorkKind::Agent => "this agent",
        };
        let mut finish = finish_now();
        finish.error = Some(format!(
            "OpenCrabs restarted while {subject} was running, so it was killed before finishing"
        ));
        self.finish = Some(finish);
        self.write()
    }

    /// Read the persisted status for a work item, if the file exists.
    pub fn read(id: &str) -> Option<Self> {
        let path = status_path(id);
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
        let data = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
        fs::rename(tmp, path)
    }

    /// List all known status files (by work id).
    pub fn list_all() -> std::io::Result<Vec<String>> {
        let dir = status_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str()
                && let Some(id) = name.strip_suffix(".json")
            {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Find a sub-agent status whose **child** session is `session_id` and
    /// whose outcome is not yet decided (#110).
    ///
    /// The boot-resume path uses this to recognize a revived sub-agent
    /// session: its result must go to the spawning session, not to the
    /// recorded channel. `Interrupted` files qualify on purpose — upstream
    /// reconciliation marks the file at boot, before the resumed session
    /// finishes — as do `Running`/`Pending`/`AwaitingInput`. `None` for
    /// unknown, outcome-terminal (`Completed`/`Failed`), or non-agent work —
    /// those keep the existing resume behavior.
    pub fn find_agent_by_session(session_id: &str) -> Option<WorkStatus> {
        let ids = Self::list_all().ok()?;
        ids.into_iter().filter_map(|id| Self::read(&id)).find(|s| {
            matches!(s.kind, WorkKind::Agent)
                && s.session_id == session_id
                && !matches!(s.state, WorkState::Completed | WorkState::Failed)
        })
    }
}

// ── Legacy migration ─────────────────────────────────────────────────

/// On-disk schema of a pre-#26 sub-agent status file (`tmp/subagents/`).
#[derive(Deserialize)]
struct LegacySubagentStatus {
    id: String,
    label: String,
    parent_session_id: String,
    state: WorkState,
    prompt: String,
    started_at: String,
    #[serde(default)]
    progress: Option<ProgressSnapshot>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Move pre-#26 sub-agent status files from `legacy` into [`status_dir`],
/// converting them to the unified schema. Returns the number migrated.
///
/// Runs at startup before reconciliation, so an agent in flight across the
/// binary swap is still interrupted and reported (#1038): the dir changes
/// homes, the semantics must survive the upgrade.
pub fn migrate_legacy_dir(legacy: &Path) -> usize {
    let entries = match fs::read_dir(legacy) {
        Ok(entries) => entries,
        // No legacy dir = nothing to migrate — the common case.
        Err(_) => return 0,
    };
    let mut migrated = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let data = match fs::read_to_string(&path) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!(
                    target: "subagent",
                    "Could not read legacy sub-agent status file {}: {e}",
                    path.display()
                );
                continue;
            }
        };
        let old: LegacySubagentStatus = match serde_json::from_str(&data) {
            Ok(old) => old,
            Err(e) => {
                tracing::warn!(
                    target: "subagent",
                    "Could not parse legacy sub-agent status file {}: {e}",
                    path.display()
                );
                continue;
            }
        };
        if status_path(&old.id).exists() {
            // The unified dir already has a record for this id (written by
            // the new code): it is fresher by definition. Drop the duplicate.
            let _ = fs::remove_file(&path);
            continue;
        }
        let status = WorkStatus {
            id: old.id.clone(),
            kind: WorkKind::Agent,
            session_id: old.parent_session_id,
            parent_session_id: None,
            label: old.label,
            task: old.prompt,
            spawned_at: old.started_at,
            state: old.state,
            progress: old.progress,
            finish: old.completed_at.map(|completed_at| {
                let mut finish = WorkFinish {
                    completed_at,
                    success: None,
                    code: None,
                    elapsed_secs: None,
                    output_bytes: None,
                    error: None,
                    output_full: None,
                };
                finish.error = old.error.clone();
                finish
            }),
        };
        match status.write() {
            Ok(()) => {
                let _ = fs::remove_file(&path);
                migrated += 1;
            }
            Err(e) => {
                // Leave the legacy file in place: a retry on the next boot
                // beats a silent loss of a possibly-stranded agent.
                tracing::warn!(
                    target: "subagent",
                    "Could not migrate legacy sub-agent status {} into the unified dir: {e}",
                    old.id
                );
            }
        }
    }
    if migrated > 0 {
        // Best effort: drop the legacy dir once it is empty.
        let _ = fs::remove_dir(legacy);
    }
    migrated
}

// ── Auto-cleanup ─────────────────────────────────────────────────────

/// Remove status files whose finish `completed_at` is older than `max_age`
/// or whose on-disk mtime is older than `max_age` (for files without a
/// finish — covers in-flight, old, and corrupted files).
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
            if let Ok(status) = serde_json::from_str::<WorkStatus>(&data) {
                status
                    .finish
                    .as_ref()
                    .is_some_and(|f| parse_completed_at(&cutoff, &f.completed_at))
                    || status.finish.is_none() && file_stale(&path, &cutoff)
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
