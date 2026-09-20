//! Tests for the shared platform shell pair (`cmd /C` on Windows, `sh -c`
//! elsewhere). The live-probe test is the regression guard for the
//! `sh`-hardcode bug: four spawn sites (background tasks, dynamic tools,
//! the plan verification gate, the TUI `!` operator) hard-coded `sh -c`
//! and were dead on Windows while Linux CI stayed green.

use crate::utils::shell::shell_pair;

#[test]
fn shell_pair_matches_platform() {
    let (program, flag) = shell_pair();
    if cfg!(target_os = "windows") {
        assert_eq!((program, flag), ("cmd", "/C"));
    } else {
        assert_eq!((program, flag), ("sh", "-c"));
    }
}

/// Run a trivial command through the pair and require success. On Windows
/// this fails against any site that still hardcodes `sh -c` (program not
/// found); on Unix it exercises the same path CI runs.
#[test]
fn shell_pair_runs_a_command_on_this_platform() {
    let (program, flag) = shell_pair();
    let out = std::process::Command::new(program)
        .arg(flag)
        .arg("echo shell_pair_probe_ok")
        .output()
        .expect("spawn platform shell");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "shell {program} {flag} failed: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("shell_pair_probe_ok"),
        "expected probe token in stdout, got: {stdout}"
    );
}

/// H-05 regression: a command containing quoted arguments must reach the
/// shell verbatim. With plain `.arg()` on Windows, MSVC-style escaping
/// rewrites `echo "x y"` so cmd.exe sees `echo \"x y\"` and the echo output
/// carries literal backslash-quotes. Live repro class: guardrail-0003
/// (`python "C:/x/probe.py"` receiving a mangled argv path).
#[tokio::test]
async fn push_shell_command_passes_quoted_command_verbatim() {
    use crate::utils::shell::PushShellCommand;
    let (shell, flag) = shell_pair();
    let mut cmd = tokio::process::Command::new(shell);
    cmd.push_shell_command(flag, "echo \"push_shell_verbatim_ok\"");
    let out = cmd.output().await.expect("spawn platform shell");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("push_shell_verbatim_ok"),
        "expected probe token in stdout, got: {stdout:?}"
    );
    assert!(
        !stdout.contains("\\\""),
        "quoted command was mangled through MSVC arg escaping: {stdout:?}"
    );
}

/// H-02 regression: `kill_process_tree` must terminate a spawned command
/// AND its descendants. Spawns `cmd /C ping -n 30 …` (ping is a grandchild
/// of the cmd.exe pid we kill by), then asserts the pid is gone. Without
/// the tree kill, a timed-out command's work processes survive as orphans
/// holding file locks (live repro: cargo kept `target/` + package-cache
/// locks for minutes after its parent cmd.exe was killed).
#[cfg(windows)]
#[test]
fn kill_process_tree_terminates_the_tree() {
    use crate::utils::shell::PushShellCommand;
    let (shell, flag) = shell_pair();
    let mut cmd = std::process::Command::new(shell);
    cmd.push_shell_command(flag, "ping -n 30 127.0.0.1 > NUL");
    let child = cmd.spawn().expect("spawn ping tree");
    let pid = child.id();
    // Let the shell spawn its grandchild before killing.
    std::thread::sleep(std::time::Duration::from_millis(400));
    crate::utils::shell::kill_process_tree(pid);
    std::thread::sleep(std::time::Duration::from_millis(400));
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}")])
        .output()
        .expect("run tasklist");
    let listing = String::from_utf8_lossy(&out.stdout);
    assert!(
        !listing.contains(&pid.to_string()),
        "pid {pid} (or its tree) survived kill_process_tree: {listing}"
    );
}

/// The Unix half of the tree kill, which shipped with no rig behind it: the
/// existing regression above is `cfg(windows)`, so `pkill -TERM -P` reached
/// every macOS and Linux bash timeout untested. Spawns a shell that keeps a
/// backgrounded `sleep` as a real child (a bare `sh -c "sleep"` execs into
/// sleep and leaves no child to sweep), asserts the child exists first so a
/// vacuous pass is impossible, then asserts the sweep took it.
#[cfg(not(windows))]
#[test]
fn kill_process_tree_sweeps_the_children_on_unix() {
    fn children_of(pid: u32) -> String {
        let out = std::process::Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
            .expect("run pgrep");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    use crate::utils::shell::PushShellCommand;
    let (shell, flag) = shell_pair();
    let mut cmd = std::process::Command::new(shell);
    cmd.push_shell_command(flag, "sleep 37 & wait");
    let mut child = cmd.spawn().expect("spawn sleep tree");
    let pid = child.id();

    // Let the shell fork its grandchild before sweeping.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let before = children_of(pid);
    assert!(
        !before.is_empty(),
        "precondition: the shell should own a child to sweep, found none"
    );

    crate::utils::shell::kill_process_tree(pid);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let after = children_of(pid);

    // Reap the shell itself regardless of the outcome: a failed assertion
    // must not leave a 37-second sleep on the developer's machine.
    if let Err(e) = child.kill() {
        eprintln!("could not kill the test shell pid {pid}: {e}");
    }
    if let Err(e) = child.wait() {
        eprintln!("could not reap the test shell pid {pid}: {e}");
    }

    assert!(
        after.is_empty(),
        "children of pid {pid} survived kill_process_tree: before={before:?} after={after:?}"
    );
}
