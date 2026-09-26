//! Background task manager (#722).
//!
//! Runs a genuinely long command detached (so it doesn't churn the bash 600s
//! cap) and, on completion, enqueues a synthetic `QueuedUserMessage` into the
//! originating session via the surface enqueue callback. The tool loop drains
//! that at the next iteration boundary — injected mid-turn if the agent is still
//! working, or starting a fresh turn if it went idle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use uuid::Uuid;

use super::types::{BgTaskMeta, PushOrigin, QueuedUserMessage};

/// Result of a finished background command.
#[derive(Debug, Clone)]
pub struct CmdResult {
    pub success: bool,
    pub code: i32,
    pub output: String,
}

/// Everything a completion hook learns about a finished command: the result
/// plus the wall-clock runtime the generic receipt carries (#15).
pub struct HookContext {
    pub result: CmdResult,
    pub elapsed_secs: f32,
}

/// Post-completion action run INSTEAD of the generic session delivery
/// (#1748). The detached rebuild uses it to exec-restart into the fresh
/// binary: an in-memory enqueue would be orphaned the moment exec()
/// replaces the process, so the hook delivers its own outcome text through
/// the routes it needs (session route, channel targets).
pub type CompletionHook = Box<
    dyn FnOnce(HookContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send,
>;

/// One in-flight background command.
#[derive(Debug, Clone)]
pub struct RunningTask {
    /// Short label for the command, e.g. `cargo test`.
    pub label: String,
    /// When it was spawned, for the elapsed time a surface displays.
    pub started: std::time::Instant,
}

/// Manages background commands and resumes their sessions on completion.
pub struct BackgroundTaskManager {
    /// In-flight background tasks per session.
    ///
    /// Holds the label and start time, not just a count, because a surface has
    /// to be able to say WHAT is running and for how long. A detached task
    /// takes the turn idle, so without this the TUI has nothing at all to draw
    /// while a long build runs and the wait looks like a hang (#762).
    running: Mutex<HashMap<Uuid, Vec<RunningTask>>>,
}

use super::work_status::{CommandExit, WorkStatus};

impl BackgroundTaskManager {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(HashMap::new()),
        }
    }

    /// How many background tasks are currently running for `session_id`.
    pub fn running_for(&self, session_id: Uuid) -> usize {
        self.running
            .lock()
            .map(|m| m.get(&session_id).map(Vec::len).unwrap_or(0))
            .unwrap_or(0)
    }

    /// What is running for `session_id`, oldest first, for surfaces that show
    /// progress. Returns owned data so the caller never holds the lock.
    pub fn running_tasks(&self, session_id: Uuid) -> Vec<RunningTask> {
        self.running
            .lock()
            .map(|m| m.get(&session_id).cloned().unwrap_or_default())
            .unwrap_or_default()
    }

    fn mark_started(&self, session_id: Uuid, label: &str) {
        if let Ok(mut m) = self.running.lock() {
            m.entry(session_id).or_default().push(RunningTask {
                label: label.to_string(),
                started: std::time::Instant::now(),
            });
        }
    }

    fn mark_finished(&self, session_id: Uuid, label: &str) {
        if let Ok(mut m) = self.running.lock()
            && let Some(tasks) = m.get_mut(&session_id)
        {
            // Remove the OLDEST entry with this label: two `cargo test` runs are
            // indistinguishable here, and dropping the oldest keeps the elapsed
            // time shown for the survivor honest.
            if let Some(pos) = tasks.iter().position(|t| t.label == label) {
                tasks.remove(pos);
            }
            if tasks.is_empty() {
                m.remove(&session_id);
            }
        }
    }

    /// Spawn `command` (via `sh -c`) in `cwd`, detached; on completion enqueue a
    /// system message into `session_id` summarizing the result. Returns
    /// immediately — the caller's turn is free to end.
    pub fn spawn_command(
        self: std::sync::Arc<Self>,
        session_id: Uuid,
        cwd: PathBuf,
        label: String,
        command: String,
    ) {
        self.spawn_inner(session_id, cwd, label, command, None);
    }

    /// Spawn like [`Self::spawn_command`], but hand the outcome to `hook`
    /// instead of the generic session delivery (#1748). Identical lifecycle:
    /// timer, status file, DB accounting; only the completion route differs.
    pub fn spawn_command_with_hook(
        self: std::sync::Arc<Self>,
        session_id: Uuid,
        cwd: PathBuf,
        label: String,
        command: String,
        hook: CompletionHook,
    ) {
        self.spawn_inner(session_id, cwd, label, command, Some(hook));
    }

    fn spawn_inner(
        self: std::sync::Arc<Self>,
        session_id: Uuid,
        cwd: PathBuf,
        label: String,
        command: String,
        hook: Option<CompletionHook>,
    ) {
        self.mark_started(session_id, &label);
        let this = std::sync::Arc::clone(&self);
        let task_id = Uuid::new_v4();
        // Gap 2 (#1160): mid-run visibility. The status file exists from
        // spawn with label/command/session, so tasks_list consumers can see
        // what a detached command IS before it finishes. Best-effort: never
        // fatal to the command itself.
        if let Err(e) = WorkStatus::new_command(
            &task_id.to_string(),
            &session_id.to_string(),
            &label,
            &command,
        ) {
            tracing::warn!(
                target: "background_task",
                "Could not write detached status for {task_id}: {e}"
            );
        }
        tokio::spawn(async move {
            // Log the START as well as the finish. Only completions were
            // logged, so a task that never finished left no trace of having
            // begun, and reconstructing which commands got detached meant
            // inferring it from the completions that did arrive.
            tracing::info!(
                target: "background_task",
                "Background task '{label}' started for session {session_id} \
                 (id={task_id}, cwd={})",
                cwd.display()
            );
            // Persist BEFORE running: a restart mid-command must find a row to
            // report as interrupted, otherwise the session waits forever on a
            // resume that can no longer come (#763).
            if let Some(repo) = task_repo() {
                let cwd_str = cwd.to_string_lossy().to_string();
                if let Err(e) = repo
                    .record(task_id, session_id, &label, &command, &cwd_str)
                    .await
                {
                    // Not fatal: the command still runs and still resumes the
                    // session in this process. Only restart accounting is lost.
                    tracing::error!(
                        target: "background_task",
                        "Failed to persist background task '{label}': {e:#}"
                    );
                }
            }
            let started = std::time::Instant::now();
            let result = run_detached(&command, &cwd, session_id).await;
            // Capture ONCE: the log line, the status file and the receipt
            // payload (#15) must all report the same runtime.
            let elapsed_secs = started.elapsed().as_secs_f32();
            // Exit code and elapsed time, not just a boolean: how long a task
            // actually took is the only way to tell a correct detach from a
            // wasteful one, and it was nowhere in the log.
            tracing::info!(
                target: "background_task",
                "Background task '{label}' for session {session_id} finished \
                 (success={}, exit={}, elapsed={:.1}s)",
                result.success,
                result.code,
                elapsed_secs
            );
            // Gap 2 (#1160): rewrite the status file with exit info, so any
            // reader between process-exit and session-resume sees the
            // terminal state instead of a forever-running spawn record.
            if let Err(e) = WorkStatus::finish_command(
                &task_id.to_string(),
                &session_id.to_string(),
                &label,
                &command,
                CommandExit {
                    success: result.success,
                    code: result.code,
                    elapsed_secs,
                    output_bytes: result.output.len(),
                },
            ) {
                tracing::warn!(
                    target: "background_task",
                    "Could not write detached status for {task_id}: {e}"
                );
            }
            if let Some(repo) = task_repo()
                && let Err(e) = repo.clear(task_id).await
            {
                // A stale row makes the NEXT startup report a phantom
                // interruption, so this must be visible even though the
                // command itself succeeded.
                tracing::error!(
                    target: "background_task",
                    "Failed to clear background task '{label}' after completion: {e:#}"
                );
            }
            // Clear the indicator BEFORE delivering, not after. The task is
            // over the moment the process exits, but mark_finished sat behind
            // the enqueue callback, so the "running" badge outlived the work by
            // however long delivery took — on a killed task the user saw the
            // agent confirm it had stopped while the input border still showed
            // it running.
            //
            // Only touches the in-memory map, so moving it earlier cannot
            // affect what gets delivered.
            this.mark_finished(session_id, &label);
            if let Some(hook) = hook {
                // A hook replaces the generic delivery entirely (#1748): the
                // rebuild exec-replaces the process on success, so an
                // in-memory enqueue would be orphaned mid-flight. The hook
                // delivers its own outcome text through the routes it needs.
                hook(HookContext {
                    result,
                    elapsed_secs,
                })
                .await;
                return;
            }
            // Deliver through the ONE gated route (fork #19): the same
            // `deliver_to_session` that sub-agent completions and the
            // session_notify tool use, so channel-ownership, mid-turn and
            // redirect decisions live in exactly one place instead of being
            // re-derived per surface. Resolves the owner by SESSION, never by
            // whichever service executed the command — a channel session
            // driven from the TUI runs on the TUI's service, and the old
            // direct-resolve would answer into the TUI and leave the channel
            // that asked for the work waiting on a reply that never comes
            // (#940). interrupt=true: a completion is the origin's own
            // awaited work, exactly like a sub-agent's; it must reach it even
            // mid-turn (fork #13).
            let msg = completion_message(&label, &command, &result, elapsed_secs);
            let outcome = super::session_routes::deliver_to_session(session_id, msg, true);
            match outcome {
                super::session_routes::Delivery::Redirected { to } => {
                    tracing::info!(
                        target: "background_task",
                        "Background task '{label}' completion for session {session_id} was \
                         redirected to session {to}, which now owns its channel"
                    );
                }
                super::session_routes::Delivery::Parked => {
                    tracing::info!(
                        target: "background_task",
                        "Background task '{label}' completion for session {session_id} is \
                         parked until its channel claims the session"
                    );
                }
                super::session_routes::Delivery::NoRoute => {
                    tracing::warn!(
                        target: "background_task",
                        "Background task '{label}' completion for session {session_id} had \
                         nowhere to go; the session will not hear about it"
                    );
                }
                super::session_routes::Delivery::RefusedInFlight { .. } => {
                    // Unreachable by construction: interrupt=true is passed
                    // above, so the fork #13 gate cannot refuse. Kept explicit
                    // so a future change to the flag cannot drop the outcome
                    // silently (port seam: upstream's match has no catch-all).
                    tracing::warn!(
                        target: "background_task",
                        "Background task '{label}' completion for session {session_id} was \
                         refused by the mid-turn gate despite interrupt=true"
                    );
                }
                super::session_routes::Delivery::Delivered => {}
            }
        });
    }
}

/// The background-task repository, when a pool exists.
///
/// Resolved per call through the global pool rather than threaded through the
/// manager, because `spawn_command` is reached from the bash tool which has no
/// pool in its context. `None` before the DB is initialized (early startup,
/// tests), which simply means restart accounting is skipped.
pub(super) fn task_repo() -> Option<crate::db::BackgroundTaskRepository> {
    crate::db::global_pool().map(|p| crate::db::BackgroundTaskRepository::new(p.clone()))
}

/// Run `command` through the platform shell (`cmd /C` on Windows, `sh -c`
/// elsewhere) in `cwd`, capturing merged stdout+stderr.
async fn run_detached(command: &str, cwd: &std::path::Path, session_id: Uuid) -> CmdResult {
    use crate::utils::shell::PushShellCommand;
    use tokio::process::Command;
    let (shell, shell_arg) = crate::utils::shell::shell_pair();
    let output = Command::new(shell)
        .push_shell_command(shell_arg, command)
        .current_dir(cwd)
        .env("OPENCRABS_SESSION_ID", session_id.to_string())
        .output()
        .await;
    match output {
        Ok(out) => {
            let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&err);
            }
            CmdResult {
                success: out.status.success(),
                code: out.status.code().unwrap_or(-1),
                output: combined,
            }
        }
        Err(e) => {
            // Distinct from a command that ran and failed: nothing executed at
            // all, so the exit code below is not one the command produced.
            tracing::error!(
                target: "background_task",
                "Background command could not be launched in {}: {e}",
                cwd.display()
            );
            CmdResult {
                success: false,
                code: -1,
                output: format!("failed to launch: {e}"),
            }
        }
    }
}

/// A short human label for a command (first meaningful token sequence), for the
/// "running in the background" acknowledgement and the completion tag.
pub(crate) fn short_label(command: &str) -> String {
    let after_cd = crate::utils::command_label::command_label(command);
    let label: String = after_cd.chars().take(60).collect();
    if after_cd.chars().count() > 60 {
        format!("{label}…")
    } else {
        label
    }
}

/// Keep only the last `n` lines of `text`.
pub(crate) fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Build the resume message from a finished background command (#722). Pure so
/// the framing is unit-testable without spawning anything. `elapsed_secs` is
/// the detached command's wall-clock runtime; it rides along in the typed
/// `BgTaskMeta` payload (#15) so the receipt card renders a duration without
/// parsing the context text.
pub(crate) fn completion_message(
    label: &str,
    command: &str,
    result: &CmdResult,
    elapsed_secs: f32,
) -> QueuedUserMessage {
    let status = if result.success {
        "exit 0 (success)".to_string()
    } else {
        format!("exit {} (failure)", result.code)
    };
    let tail = tail_lines(&result.output, 50);
    let context = format!(
        "[System: the background task you started has finished.\n\
         Task: {label}\n\
         Command: {command}\n\
         Status: {status}\n\
         Output (last 50 lines):\n{tail}\n\n\
         Report the result to the user and continue anything that was waiting on it. \
         Do not re-run the command — this IS its result.]"
    );
    let display = format!(
        "🔧 background task {}: {label}",
        if result.success { "finished" } else { "failed" }
    );
    let mut msg = QueuedUserMessage::system(context, display);
    // #1221: marks this delivery for the Telegram collapsible echo bubble.
    msg.origin = PushOrigin::BackgroundTask;
    // #15: typed receipt payload — the echo renders the card from this,
    // never from the `[System: ...]` context text.
    msg.bg_meta = Some(BgTaskMeta {
        success: result.success,
        label: label.to_string(),
        elapsed_secs,
        tail,
    });
    msg
}

/// Human duration for the receipt card (#15): `42s`, `3m 5s`, `1h 12m`.
/// Rounds to whole seconds; sub-second tasks show `0s`.
pub(crate) fn format_elapsed(secs: f32) -> String {
    let total = secs.max(0.0).round() as u64;
    if total < 60 {
        format!("{total}s")
    } else if total < 3600 {
        format!("{}m {}s", total / 60, total % 60)
    } else {
        format!("{}h {}m", total / 3600, (total % 3600) / 60)
    }
}
