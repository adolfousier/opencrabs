//! #1748: the rebuild runs as a DETACHED background command through the
//! shared `BackgroundTaskManager` (live timer, status file, DB accounting)
//! with a rebuild-specific completion hook that exec-restarts into the fresh
//! binary. The process-replacing arm (`restart_into`) is never executed in
//! tests — these pins cover the pure surfaces of the migration.

use crate::brain::tools::rebuild::{detached_build_command, rebuild_deliver_target};

/// The detached command must compile the same artifact the old in-process
/// `SelfUpdater::build` did: release profile, native CPU target.
#[test]
fn detached_build_command_pins_the_compiler_semantics() {
    let cmd = detached_build_command();
    assert!(
        cmd.contains("cargo build --release"),
        "release build required: {cmd}"
    );
    assert!(
        cmd.contains("RUSTFLAGS='-C target-cpu=native'"),
        "must match SelfUpdater::build compiler semantics: {cmd}"
    );
}

/// Delivery-target mapping still routes every channel arm after the
/// migration (#305): telegram (with and without the #1451 topic form),
/// discord, slack; TUI and unknown channels map to None.
#[test]
fn delivery_target_mapping_unchanged_by_the_migration() {
    assert_eq!(
        rebuild_deliver_target("telegram", Some("42"), None).as_deref(),
        Some("telegram:42")
    );
    assert_eq!(
        rebuild_deliver_target("telegram", Some("42"), Some("7")).as_deref(),
        Some("telegram:42:7")
    );
    assert_eq!(
        rebuild_deliver_target("discord", Some("99"), None).as_deref(),
        Some("discord:99")
    );
    assert_eq!(rebuild_deliver_target("tui", Some("1"), None), None);
    assert_eq!(rebuild_deliver_target("telegram", None, None), None);
    assert_eq!(rebuild_deliver_target("telegram", Some("  "), None), None);
}
