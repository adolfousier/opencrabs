//! Platform shell selection for running a full command string.
//!
//! One source of truth for the (program, arg) pair that executes a
//! shell-command string: `cmd /C` on Windows, `sh -c` everywhere else.
//!
//! Why this module exists: five call sites each hand-rolled this decision
//! (or skipped it). Four of them hardcoded `sh -c` — `background_tasks.rs`
//! (detached command runner), `dynamic/tool.rs` (dynamic shell tools),
//! `plan_tool.rs` (Ralph verification gate) and `tui/app/input.rs` (the
//! `!command` bang operator). On Windows none of those spawn: `sh` is not
//! on PATH outside WSL/Git-Bash setups, so every detached background task
//! died with `program not found` while the inline bash tool — the one site
//! that had the `cfg!` — kept working. Linux CI never noticed because `sh`
//! exists there; Windows is not built in CI (#627) and the release job only
//! compiles, it never runs these paths.
//!
//! Use [`shell_pair`] + [`PushShellCommand`] whenever a full command STRING
//! must run through a shell. Do not add another inline
//! `cfg!(target_os = "windows")` — call these, so the platform decision
//! cannot drift between sites again.

/// The (program, flag) pair that runs a command string through the platform
/// shell: `("cmd", "/C")` on Windows, `("sh", "-c")` elsewhere.
///
/// Usage: `Command::new(shell).push_shell_command(shell_arg, command)`.
pub fn shell_pair() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    }
}

/// Append the platform shell flag and a full command string to a
/// [`std::process::Command`] or [`tokio::process::Command`].
///
/// The naive spelling — `Command::new(shell).arg(flag).arg(cmd)` — is
/// correct on Unix and WRONG on Windows: `arg()` applies MSVC-style
/// escaping (quote-wrap + backslash-escape inner quotes), but `cmd.exe /C`
/// re-parses the command line with its own quoting rules, so any command
/// containing quotes is corrupted. Live repros (guardrail-0003):
/// `python "C:/x/probe.py"` receives `C:\Windows\System32\"C:\Users\…"` as
/// argv; `dir "C:\Program Files"` fails with "filename syntax incorrect".
/// On Windows this appends both parts with `raw_arg` (verbatim, no
/// escaping); elsewhere it is a plain `arg`.
pub trait PushShellCommand {
    /// Append `shell_arg` and `command` to this command, verbatim on Windows.
    fn push_shell_command(&mut self, shell_arg: &str, command: &str) -> &mut Self;
}

#[cfg(windows)]
impl PushShellCommand for std::process::Command {
    fn push_shell_command(&mut self, shell_arg: &str, command: &str) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.raw_arg(shell_arg).raw_arg(command)
    }
}

#[cfg(not(windows))]
impl PushShellCommand for std::process::Command {
    fn push_shell_command(&mut self, shell_arg: &str, command: &str) -> &mut Self {
        self.arg(shell_arg).arg(command)
    }
}

#[cfg(windows)]
impl PushShellCommand for tokio::process::Command {
    fn push_shell_command(&mut self, shell_arg: &str, command: &str) -> &mut Self {
        self.raw_arg(shell_arg).raw_arg(command)
    }
}

#[cfg(not(windows))]
impl PushShellCommand for tokio::process::Command {
    fn push_shell_command(&mut self, shell_arg: &str, command: &str) -> &mut Self {
        self.arg(shell_arg).arg(command)
    }
}

/// Kill a process's descendant tree, best-effort, on the timeout path.
///
/// tokio's `kill()`/`kill_on_drop` terminates only the DIRECT child — the
/// shell — so the actual work process spawned by the command (cargo,
/// cmake, ping, …) survives as an orphan, still holding locks: a timed-out
/// `cargo build` keeps the target-dir and package-cache locks taken for
/// minutes afterwards. Failure (pid already exited, helper missing) never
/// aborts the caller — this is a best-effort sweep on the timeout error
/// path, never a reason to mask the Timeout itself — but it is reported,
/// see [`log_sweep`].
///
/// Windows: `taskkill /T /F` walks the whole tree including the shell pid
/// and force-terminates it.
#[cfg(windows)]
pub fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    log_sweep(
        pid,
        "taskkill",
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output(),
    );
}

/// Unix counterpart: signal the work processes the shell spawned.
///
/// The shells here are not process-group leaders, so a group kill
/// (`kill -pgid`) would signal the agent itself; instead `pkill -TERM -P`
/// targets the direct children of `pid` while the caller's
/// `kill_on_drop`/`kill()` reaps the shell. If the shell already exited,
/// its children are re-parented and pkill finds nothing — same best-effort
/// contract as the Windows path. pkill ships with procps on Linux and is
/// standard on macOS; if absent, the failed spawn is ignored.
#[cfg(not(windows))]
pub fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    log_sweep(
        pid,
        "pkill",
        std::process::Command::new("pkill")
            .args(["-TERM", "-P", &pid.to_string()])
            .output(),
    );
}

/// Record what the sweep did. Best-effort is a decision about whether to
/// abort, not a licence to say nothing: a helper that is missing from the
/// image looks exactly like a sweep that worked, and the orphan holding the
/// cargo lock is the only symptom anyone ever sees. Exit status 1 is the
/// ordinary "no such children" case for both helpers, so it stays at debug.
fn log_sweep(pid: u32, helper: &str, result: std::io::Result<std::process::Output>) {
    match result {
        Ok(out) if out.status.success() => {
            tracing::debug!("{helper} swept the process tree under pid {pid}");
        }
        Ok(out) => {
            tracing::debug!(
                "{helper} found nothing to sweep under pid {pid} ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Err(e) => {
            tracing::warn!(
                "could not run {helper} to sweep the process tree under pid {pid}: {e}. \
                 A timed-out command's work process may survive as an orphan still \
                 holding its locks."
            );
        }
    }
}
