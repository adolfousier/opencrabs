//! Platform shell selection for running a full command string.
//!
//! One source of truth for the (program, arg) pair that executes a
//! shell-command string: `cmd /C` on Windows; on every other host a real
//! `bash` where the host has one, and plain `sh` where it does not.
//!
//! Why bash and not `sh`: the tool that hands commands to this pair is
//! NAMED `bash` and its description teaches bash idioms, so the model
//! writes `$'...'`, `[[ ]]` and arrays on purpose. Debian and Ubuntu ship
//! dash as `/bin/sh`, and dash does not reject most of that syntax —
//! `IFS=$'\t'` assigns the three literal characters `$`, `\`, `t` and
//! exits 0, so every later field split chops at the letter `t` and the
//! caller gets corrupt data dressed as success (#1704). Running a real
//! bash makes the idiom mean what it looks like. The `sh` fallback stays
//! because a host may legitimately have no bash at all; when that happens
//! the fallback is logged once, so the dialect in force is discoverable
//! instead of inferred from a silent misparse.
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

#[cfg(not(windows))]
use std::sync::OnceLock;

/// Candidate paths for a real bash, tried in order.
///
/// Absolute paths only, no `PATH` walk: the daemon's `PATH` is whatever the
/// supervisor handed it, and a `bash` resolved late in that list is more
/// often a wrapper than a shell. These four cover the Linux distros
/// (merged-usr put it in `/usr/bin`, older layouts in `/bin`) and Homebrew
/// on both Intel and Apple-silicon macOS.
#[cfg(not(windows))]
pub(crate) const BASH_CANDIDATES: [&str; 4] = [
    "/bin/bash",
    "/usr/bin/bash",
    "/usr/local/bin/bash",
    "/opt/homebrew/bin/bash",
];

/// The question every candidate is asked before it is believed.
///
/// `printf '%s' $'\t'` is the promise the tool makes to the model: ANSI-C
/// quoting expands to the byte it names. dash answers that with the three
/// literal characters `$`, `\`, `t` and exit code 0 (#1704), so a candidate
/// is accepted only when the output is exactly one tab byte. The
/// `BASH_VERSION` guard is there so a POSIX shell that grew ANSI-C quoting
/// of its own (busybox ash can) is not mistaken for the rest of the bash
/// surface the tool description teaches.
#[cfg(not(windows))]
const BASH_PROBE: &str = "test -n \"$BASH_VERSION\" && printf '%s' $'\\t'";

/// Cached result of [`resolve_bash`]: the first candidate that behaved like
/// bash, or `None` when the host has none at all.
///
/// Once per PROCESS, not once per call. [`shell_pair`] sits on the hot path
/// of every shell command in every surface, and answering the question is a
/// spawn.
///
/// Crate-visible so the memoisation claim in the paragraph above is backed by
/// a test (`the_probe_is_memoised_not_re_run_per_command`) instead of having
/// to be taken on faith from a doc comment.
#[cfg(not(windows))]
pub(crate) static BASH: OnceLock<Option<&'static str>> = OnceLock::new();

/// Does `candidate` actually behave like bash, or merely exist?
///
/// Behaviour is the test. A missing path, a broken binary and a shell that
/// disagrees with the probe all answer `false` and move on to the next
/// candidate; nothing here trusts a filename. `output()` closes the child's
/// stdin, so a candidate that reads (a login wrapper sourcing an rc) gets
/// EOF rather than blocking the first command of the session.
#[cfg(not(windows))]
pub(crate) fn probes_as_bash(candidate: &str) -> bool {
    match std::process::Command::new(candidate)
        .arg("-c")
        .arg(BASH_PROBE)
        .output()
    {
        Ok(out) => out.status.success() && out.stdout == *b"\t",
        Err(_) => false,
    }
}

/// Find this host's bash, once, and say so in the log either way.
///
/// The fallback is announced because it is a downgrade the operator did not
/// choose: a silent `sh` is what made #1704 a data-corruption bug instead of
/// a bug report, and the one warn line is what turns the next occurrence
/// into a two-second diagnosis.
#[cfg(not(windows))]
fn resolve_bash() -> Option<&'static str> {
    match BASH_CANDIDATES.into_iter().find(|c| probes_as_bash(c)) {
        Some(program) => {
            tracing::debug!("running command strings under {program}");
            Some(program)
        }
        None => {
            tracing::warn!(
                "no usable bash found on this host (checked {}); command strings fall back \
                 to POSIX sh, where bash-only syntax ($'…', [[ ]], arrays, <<<, PIPESTATUS) \
                 is NOT supported (#1704)",
                BASH_CANDIDATES.join(", ")
            );
            None
        }
    }
}

/// The (program, flag) pair that runs a command string through the platform
/// shell: `("cmd", "/C")` on Windows; elsewhere `("«real bash»", "-c")` when
/// the host has one and `("sh", "-c")` when it does not.
///
/// The resolution is inside this function on purpose: callers keep the same
/// `(&'static str, &'static str)` they always had, no spawn site learns that
/// a probe happened, and all five surfaces move together instead of drifting
/// apart the way they did before this module existed.
pub fn shell_pair() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        unix_shell_pair()
    }
}

/// Non-Windows half of [`shell_pair`].
#[cfg(not(windows))]
fn unix_shell_pair() -> (&'static str, &'static str) {
    match BASH.get_or_init(resolve_bash) {
        Some(program) => (*program, "-c"),
        None => ("sh", "-c"),
    }
}

/// Windows never evaluates this — `cfg!(target_os = "windows")` takes the
/// other branch — but the call has to typecheck there. Same paired-`#[cfg]`
/// shape as the [`PushShellCommand`] impls below.
#[cfg(windows)]
fn unix_shell_pair() -> (&'static str, &'static str) {
    ("sh", "-c")
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
