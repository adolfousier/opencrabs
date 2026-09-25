//! Tests for the shared platform shell pair: `cmd /C` on Windows, a real
//! `bash` elsewhere where the host has one, POSIX `sh` only as a fallback.
//!
//! Two bug families are guarded here. The live-probe tests are the regression
//! guard for the `sh`-hardcode bug: four spawn sites (background tasks,
//! dynamic tools, the plan verification gate, the TUI `!` operator) hard-coded
//! `sh -c` and were dead on Windows while Linux CI stayed green. The #1704
//! block is the guard for the mirror image of that bug: the tool is NAMED
//! `bash` and its description teaches bash idioms, but the pair used to answer
//! `sh -c` on every Unix, so on Debian/Ubuntu (dash) `IFS=$'\t'` assigned the
//! three literal characters `$`, `\`, `t` and exited 0 — corrupt data dressed
//! as success, with no error to find.

use crate::utils::shell::shell_pair;
#[cfg(not(windows))]
use crate::utils::shell::{BASH, BASH_CANDIDATES, probes_as_bash};

/// The pair must name a shell this platform can actually spawn, and must keep
/// the platform flag. This replaced `shell_pair_matches_platform`, which
/// pinned `("sh", "-c")` on every Unix and would have blocked the #1704 fix
/// rather than caught it: a test that freezes the old answer is not a guard.
#[test]
fn shell_pair_names_a_spawnable_shell() {
    let (program, flag) = shell_pair();
    if cfg!(target_os = "windows") {
        assert_eq!((program, flag), ("cmd", "/C"));
    } else {
        assert_eq!(flag, "-c", "Unix shell flag drifted");
        #[cfg(not(windows))]
        assert!(
            program == "sh" || BASH_CANDIDATES.contains(&program),
            "shell_pair returned {program:?}, which is neither the sh fallback nor a \
             candidate bash — the dialect in force is now unpredictable"
        );
    }
}

#[cfg(not(windows))]
#[test]
fn shell_pair_uses_sh_only_when_no_bash_exists() {
    let (program, _) = shell_pair();
    let available = BASH_CANDIDATES.iter().copied().find(|c| probes_as_bash(c));
    match available {
        Some(bash) => assert_eq!(
            program, bash,
            "a real bash is installed at {bash:?} but command strings were routed to \
             {program:?} (#1704)"
        ),
        None => assert_eq!(
            program, "sh",
            "no bash on this host, so the fallback must be plain sh, got {program:?}"
        ),
    }
}

/// #1704, the anchor case: ANSI-C quoting must expand to the byte it names.
/// dash accepts the syntax without complaining and emits `$`, `\`, `t`.
#[cfg(not(windows))]
#[test]
fn ansi_c_quoting_expands_to_the_byte_it_names() {
    let stdout = run_shell(r#"printf '%s' $'\t'"#);
    assert_eq!(
        stdout, "\t",
        "ANSI-C quoting did not expand: got {stdout:?}, expected a single tab byte"
    );
}

/// #1704 as the reporter hit it: a tab-separated value split on a tab IFS.
/// Under dash `IFS=$'\t'` assigns the three characters `$`, `\`, `t`, so the
/// split shreds every field containing a `t` — measured on this host as
/// `|h|op||a|op|` (six fields, two of them empty) against bash's correct
/// `htop|atop|`. Exit status is 0 either way, which is the whole point: the
/// corruption is invisible to any caller that only checks success.
///
/// The value is assigned to `h` and split via the UNQUOTED `$h`, not written
/// inline as `set -- $'htop\tatop'`: `$'...'` is quoted syntax in bash, so an
/// inline ANSI-C word never undergoes field splitting, and an inline version
/// of this assertion passes on bash for a reason unrelated to IFS.
#[cfg(not(windows))]
#[test]
fn tab_field_split_does_not_chop_at_the_letter_t() {
    let stdout = run_shell(r#"h=$'htop\tatop'; IFS=$'\t'; set -- $h; printf '%s|' "$@""#);
    assert_eq!(
        stdout, "htop|atop|",
        "tab-separated field split was corrupted by a non-bash IFS: got {stdout:?}"
    );
}

/// The discriminating power of the probe, measured rather than asserted from
/// documentation: where the host has a dash, it must be REJECTED as bash, and
/// routing the reporter's command through it must reproduce the corruption
/// this issue is about. Guards against a future probe that loosens to
/// "anything that spawns" and against anyone concluding dash would have been
/// fine — on a host with dash this proves, in CI, that it would not.
#[cfg(not(windows))]
#[test]
fn dash_is_rejected_and_would_have_corrupted_the_split() {
    const DASH: &str = "/bin/dash";
    if !std::path::Path::new(DASH).exists() {
        eprintln!("no dash on this host (macOS ships bash as /bin/sh); skipped");
        return;
    }
    assert!(
        !probes_as_bash(DASH),
        "the probe accepted dash — #1704 would come straight back"
    );

    // And here is what dash does to the exact command above, for the record.
    let out = std::process::Command::new(DASH)
        .arg("-c")
        .arg(r#"h=$'htop\tatop'; IFS=$'\t'; set -- $h; printf '%s|' "$@""#)
        .output()
        .expect("spawn dash");
    let corrupt = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "dash exits 0 on the misparse — that is why this bug shipped as data loss, not as \
         an error: status={:?} stdout={corrupt:?}",
        out.status
    );
    assert_eq!(
        corrupt, "|h|op||a|op|",
        "dash diverged from the recorded #1704 repro: got {corrupt:?}"
    );
}

/// The second silent case from the same audit, measured against dash: `echo -e`
/// leaks its flag into the caller's data, and `printf` of an ANSI-C literal
/// emits `$\t`. Both exit 0.
#[cfg(not(windows))]
#[test]
fn dash_is_rejected_on_both_silent_misparses() {
    const DASH: &str = "/bin/dash";
    if !std::path::Path::new(DASH).exists() {
        eprintln!("no dash on this host; skipped");
        return;
    }
    let run = |arg: &str| -> String {
        let out = std::process::Command::new(DASH)
            .arg("-c")
            .arg(arg)
            .output()
            .expect("spawn dash");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(
        run(r#"printf "[%s]" $'\t'"#),
        r"[$\t]",
        "dash should emit a literal dollar then backslash-t"
    );
    assert_eq!(
        run("echo -e x"),
        "-e x\n",
        "dash should print the -e flag itself"
    );
}

/// The sibling silent case found while auditing #1704: dash has no `-e` flag,
/// so `echo -e` prints the flag itself into the caller's data. Like the ANSI-C
/// case it errors nowhere, which is why it belongs in the same guard.
#[cfg(not(windows))]
#[test]
fn echo_dash_e_does_not_leak_the_flag() {
    let stdout = run_shell(r#"echo -e 'x'"#);
    assert_eq!(
        stdout, "x\n",
        "echo -e leaked its flag, so command strings are running under a non-bash \
         shell: got {stdout:?}"
    );
}

/// The probe spawns a process, so it must be answered once and memoised —
/// otherwise every command in every surface pays a spawn for the answer.
#[cfg(not(windows))]
#[test]
fn the_probe_is_memoised_not_re_run_per_command() {
    let first = shell_pair();
    let cached = *BASH
        .get()
        .expect("shell_pair must populate the BASH cache on first call");
    assert_eq!(
        first,
        shell_pair(),
        "two calls disagreed, so the decision is not stable"
    );
    assert_eq!(
        cached.map(|p| (p, "-c")),
        Some(first),
        "the cached decision is not the one shell_pair returned"
    );
}

/// Nothing in the resolution trusts a filename, so a path that is not a
/// working bash must be rejected rather than selected.
#[cfg(not(windows))]
#[test]
fn the_probe_rejects_anything_that_is_not_a_working_bash() {
    assert!(
        !probes_as_bash("/nonexistent/opencrabs-bash"),
        "a missing path was accepted as bash"
    );
    assert!(
        !probes_as_bash(""),
        "an empty program name was accepted as bash"
    );
    assert!(
        !probes_as_bash("/"),
        "a directory was accepted as bash — the spawn error must be false, not a panic"
    );
}

/// Run a command string through the selected pair and return stdout.
/// Panics with both streams attached so a failure names the shell at fault.
#[cfg(not(windows))]
#[track_caller]
fn run_shell(command: &str) -> String {
    let (program, flag) = shell_pair();
    let out = std::process::Command::new(program)
        .arg(flag)
        .arg(command)
        .output()
        .unwrap_or_else(|e| panic!("spawn {program} {flag}: {e}"));
    assert!(
        out.status.success(),
        "{program} failed on {command:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
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
