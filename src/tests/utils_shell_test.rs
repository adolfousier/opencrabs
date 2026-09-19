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
