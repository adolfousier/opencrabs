//! Tests for launch-cwd sanitization: a process started by a Windows
//! service/scheduled task lands in `%SystemRoot%\System32`, which must not
//! become the agent's working directory.

use crate::utils::cwd::{launch_cwd, sanitize_launch_cwd};
use std::path::PathBuf;

#[test]
fn normal_cwd_passes_through_unchanged() {
    let p = PathBuf::from(if cfg!(windows) {
        "C:\\Users\\someone\\projects\\app"
    } else {
        "/home/someone/projects/app"
    });
    assert_eq!(sanitize_launch_cwd(p.clone()), p);
}

#[cfg(windows)]
#[test]
fn system_dir_launch_falls_back_to_home() {
    let root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| "C:\\Windows".into());
    for cand in [
        root.clone(),
        format!("{root}\\System32"),
        // alternate separators + case, as canonicalization may emit them
        format!("{root}\\system32\\")
            .replace('\\', "/")
            .to_ascii_uppercase(),
        format!("{root}\\SysWOW64"),
        format!("{root}\\Sysnative"),
    ] {
        assert_eq!(
            sanitize_launch_cwd(PathBuf::from(&cand)),
            dirs::home_dir().expect("home dir on windows"),
            "cwd {cand} must fall back to home"
        );
    }
}

#[cfg(windows)]
#[test]
fn lookalike_user_dir_is_not_treated_as_system() {
    // "C:\Users\dev\Windows\System32" only looks like the system dir —
    // it is a user directory and must pass through untouched.
    let p = PathBuf::from("C:\\Users\\dev\\Windows\\System32");
    assert_eq!(sanitize_launch_cwd(p.clone()), p);
}

#[cfg(not(windows))]
#[test]
fn unix_cwd_is_never_rewritten() {
    let p = PathBuf::from("/usr/bin");
    assert_eq!(sanitize_launch_cwd(p.clone()), p);
}

#[test]
fn launch_cwd_returns_a_real_directory() {
    // Whatever the environment, the launcher must resolve to a usable dir:
    // never the empty path `unwrap_or_default()` can produce.
    let cwd = launch_cwd();
    assert!(cwd.is_dir(), "launch_cwd() must be an existing directory");
}
