//! External-scope hits render repo-relative paths (#89).
//!
//! External documents are keyed by absolute path (#1051), so every hit used
//! to echo its full configured root back — the same 16+ char prefix on every
//! line, drowning the informative part. When the path sits under a
//! `[memory]` `extra_paths` root, `resolve_path` now emits it relative to
//! that root; paths outside every configured root pass through unchanged.

use crate::config::profile::with_home_override;
use crate::memory::COLLECTION_EXTERNAL;
use crate::memory::search::resolve_path;
use std::path::Path;

const CONFIG_WITH_EXTRA_PATHS: &str = r#"
[memory]
extra_paths = ["/home/u/notes"]
"#;

/// Path under a configured extra_path root → relative to that root.
#[test]
fn external_path_under_root_strips_prefix() {
    let (_temp, home) = temp_home_with(CONFIG_WITH_EXTRA_PATHS);
    with_home_override(home, || {
        let got = resolve_path(
            Path::new("/unused"),
            COLLECTION_EXTERNAL,
            "/home/u/notes/rust/opencrabs/src/memory/search.rs",
        );
        assert_eq!(got, "rust/opencrabs/src/memory/search.rs");
    });
}

/// Root with a trailing slash still strips (prefix boundary is normalized).
#[test]
fn external_path_strips_trailing_slash_root() {
    let (_temp, home) = temp_home_with(
        r#"[memory]
extra_paths = ["/home/u/notes/"]
"#,
    );
    with_home_override(home, || {
        let got = resolve_path(
            Path::new("/unused"),
            COLLECTION_EXTERNAL,
            "/home/u/notes/a.md",
        );
        assert_eq!(got, "a.md");
    });
}

/// A sibling directory sharing the root's prefix must NOT be stripped —
/// `/home/u/notes` must not turn `/home/u/notes2/x.md` into `2/x.md`.
#[test]
fn external_path_prefix_boundary_is_respected() {
    let (_temp, home) = temp_home_with(CONFIG_WITH_EXTRA_PATHS);
    with_home_override(home, || {
        let got = resolve_path(
            Path::new("/unused"),
            COLLECTION_EXTERNAL,
            "/home/u/notes2/x.md",
        );
        assert_eq!(got, "/home/u/notes2/x.md");
    });
}

/// Path outside every configured root passes through unchanged (#1051).
#[test]
fn external_path_outside_roots_passes_through() {
    let (_temp, home) = temp_home_with(CONFIG_WITH_EXTRA_PATHS);
    with_home_override(home, || {
        let got = resolve_path(
            Path::new("/unused"),
            COLLECTION_EXTERNAL,
            "/var/data/other.md",
        );
        assert_eq!(got, "/var/data/other.md");
    });
}

/// Brain/memory paths are home-anchored as before — the strip is
/// external-only (#1051 behavior preserved).
#[test]
fn brain_path_still_home_anchored() {
    let (_temp, home) = temp_home_with(CONFIG_WITH_EXTRA_PATHS);
    let anchor = home.clone();
    with_home_override(home, || {
        let got = resolve_path(&anchor, "brain", "SOUL.md");
        assert_eq!(got, anchor.join("SOUL.md").to_string_lossy());
    });
}

/// A tempdir laid out as an `.opencrabs` home, plus the config it starts with
/// (same fixture shape as config_last_good_recovery_test).
fn temp_home_with(config_toml: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let opencrabs = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&opencrabs).expect("create .opencrabs");
    std::fs::write(opencrabs.join("config.toml"), config_toml).expect("write config");
    std::fs::write(opencrabs.join("keys.toml"), b"").expect("write keys");
    (dir, opencrabs)
}
