//! Per-project `ralph_loop.toml` resolution (#947).
//!
//! The machine-wide safety copy used to govern every session, so its
//! cargo-flavored verification table leaked into non-Rust projects: a Zig
//! plan task was gated on `cargo test` against a repo it had nothing to do
//! with. A project's own `ralph_loop.toml` at the session working dir now
//! wins outright; the safety copy is only the fallback for projects without
//! one.
//!
//! These tests pin that precedence, hot reload per resolved path (#852), and
//! the rule that a broken project file yields `None` instead of silently
//! falling back to the machine-wide config (a silent fallback would
//! resurrect the exact leak this resolves).
//!
//! Fixtures live in unique tempdirs, so tests run in parallel safely and
//! nothing ever writes to `~/.opencrabs/safety/`.

use crate::brain::tools::plan_tool::{ralph_config_path, ralph_loop_config};
use crate::brain::tools::toml_hot_reload::safety_path;
use std::fs;

/// A tempdir unique to this test: distinct paths mean distinct cache entries
/// in the process-wide resolver cache, so parallel tests cannot interfere.
fn scratch(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("ralph947_{name}_"))
        .tempdir()
        .expect("tempdir")
}

#[test]
fn ralph_config_path_prefers_project_file() {
    let dir = scratch("prefer_project");
    let project_file = dir.path().join("ralph_loop.toml");
    fs::write(&project_file, "[forward]\nmax_iterations = 7\n").unwrap();

    assert_eq!(ralph_config_path(dir.path()), Some(project_file));
}

#[test]
fn ralph_config_path_falls_back_to_safety_copy() {
    let dir = scratch("fallback");
    // No project file: the machine-wide safety copy is the fallback.
    assert_eq!(
        ralph_config_path(dir.path()),
        safety_path("ralph_loop.toml")
    );
}

#[test]
fn project_ralph_file_overrides_machine_default() {
    let dir = scratch("override");
    fs::write(
        dir.path().join("ralph_loop.toml"),
        "[forward]\nmax_iterations = 7\n",
    )
    .unwrap();

    let config = ralph_loop_config(dir.path()).expect("project file present");
    // The machine-wide default is 20; the project file says 7 and must win
    // regardless of what (or whether) the global file exists.
    assert_eq!(config.forward.max_iterations, 7);
}

#[test]
fn project_ralph_file_hot_reloads_on_change() {
    let dir = scratch("hot_reload");
    let path = dir.path().join("ralph_loop.toml");
    fs::write(&path, "[forward]\nmax_iterations = 7\n").unwrap();
    assert_eq!(
        ralph_loop_config(dir.path())
            .expect("file present")
            .forward
            .max_iterations,
        7
    );

    // Different content AND different length, so the mtime+length stamp
    // changes even on filesystems with coarse mtime granularity.
    fs::write(&path, "[forward]\nmax_iterations = 42\n").unwrap();
    assert_eq!(
        ralph_loop_config(dir.path())
            .expect("file present")
            .forward
            .max_iterations,
        42
    );
}

#[test]
fn broken_project_file_yields_none_not_the_global_leak() {
    let dir = scratch("broken");
    fs::write(dir.path().join("ralph_loop.toml"), "not toml [[[\n").unwrap();

    // Authoritative-when-present means a parse failure is None for THIS
    // project, never a silent slide back onto the machine-wide config.
    assert!(ralph_loop_config(dir.path()).is_none());
}

/// #1640: the `[epistemic]` section was silently dropped because
/// `RalphLoopConfig` had no `epistemic` field. Users could edit
/// `decay_enabled` or `decay_interval_hours` and get no error and no
/// effect. Pin that the section is now deserialized correctly.
#[test]
fn epistemic_section_is_deserialized_not_silently_dropped() {
    let dir = scratch("epistemic_1640");
    fs::write(
        dir.path().join("ralph_loop.toml"),
        r#"
[forward]
max_iterations = 5

[epistemic]
enabled = true
decay_enabled = false
decay_interval_hours = 48
contradiction_detection = true
source_required = false
"#,
    )
    .unwrap();

    let config = ralph_loop_config(dir.path()).expect("file present");
    assert_eq!(config.forward.max_iterations, 5);
    assert!(config.epistemic.enabled);
    assert!(!config.epistemic.decay_enabled);
    assert_eq!(config.epistemic.decay_interval_hours, 48);
    assert!(config.epistemic.contradiction_detection);
    assert!(!config.epistemic.source_required);
}

/// #1640: a `ralph_loop.toml` with no `[epistemic]` section still parses
/// (defaults kick in via `#[serde(default)]`).
#[test]
fn missing_epistemic_section_uses_defaults() {
    let dir = scratch("epistemic_defaults");
    fs::write(
        dir.path().join("ralph_loop.toml"),
        "[forward]\nmax_iterations = 10\n",
    )
    .unwrap();

    let config = ralph_loop_config(dir.path()).expect("file present");
    assert!(config.epistemic.enabled);
    assert!(config.epistemic.decay_enabled);
    assert_eq!(config.epistemic.decay_interval_hours, 720); // 30 days
    assert!(config.epistemic.contradiction_detection);
    assert!(config.epistemic.source_required);
}
