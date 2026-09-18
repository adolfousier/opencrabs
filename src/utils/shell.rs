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

/// Kill a process and its entire descendant tree (Windows only).
///
/// tokio's `kill()`/`kill_on_drop` TerminateProcess()es only the DIRECT
/// child — on Windows that's `cmd.exe`, so the actual work process spawned
/// by the command (cargo, cmake, ping, …) survives as an orphan, still
/// holding locks: a timed-out `cargo build` keeps the target-dir and
/// package-cache locks taken for minutes afterwards. `taskkill /T /F`
/// walks the whole tree and force-terminates it. Failure (pid already
/// exited, taskkill missing) is ignored: this is a best-effort sweep on
/// the timeout error path, never a reason to mask the Timeout itself.
#[cfg(windows)]
pub fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output();
}
