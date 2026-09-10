//! Live system-brain rebuild (#213).
//!
//! The system brain was historically assembled once at startup and cached as a
//! static string, so edits to brain files (manual, `write_opencrabs_file`,
//! `self_improve`) stayed invisible until the process restarted. `BrainRebuild`
//! fixes that: it returns the startup seed verbatim while nothing changes (so
//! the provider prompt cache stays warm), and rebuilds from disk on the next
//! turn once a brain file's mtime advances.
//!
//! These tests drive `BrainRebuild` directly with deterministic mtimes (set via
//! `File::set_modified`, no sleeping) so they can't flake on filesystem clock
//! granularity.

use crate::brain::agent::BrainRebuild;
use crate::brain::prompt_builder::{BrainLoader, RuntimeInfo};
use crate::brain::tools::error::collapse_home;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

/// A fixed, well-past instant so the seeded mtime is deterministic.
fn t0() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000)
}

fn write_with_mtime(path: &std::path::Path, content: &str, mtime: SystemTime) {
    std::fs::write(path, content).expect("write brain file");
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open for mtime")
        .set_modified(mtime)
        .expect("set mtime");
}

#[test]
fn render_returns_seed_until_a_brain_file_changes() {
    let dir = TempDir::new().unwrap();
    let soul = dir.path().join("SOUL.md");
    write_with_mtime(&soul, "--- SOUL.md ---\noriginal personality\n", t0());

    let loader = BrainLoader::new(dir.path().to_path_buf());
    // Seed with a sentinel that the real assembly would never produce, so we
    // can prove the seed is returned verbatim on the warm path.
    let rebuild = BrainRebuild::new(
        loader,
        None,
        true,  // core brain
        false, // no lazy-tools suffix
        "SEED-VERBATIM".to_string(),
        None,  // no live-cwd handle
        false, // interactive surface
    );

    // Nothing changed since construction → exact seed, no disk rebuild.
    assert_eq!(rebuild.render(), "SEED-VERBATIM");
    assert_eq!(rebuild.render(), "SEED-VERBATIM", "still warm on repeat");

    // Edit the brain file and advance its mtime → next render rebuilds.
    write_with_mtime(
        &soul,
        "--- SOUL.md ---\nEDITED_PERSONALITY_MARKER\n",
        t0() + Duration::from_secs(60),
    );

    let rebuilt = rebuild.render();
    assert_ne!(rebuilt, "SEED-VERBATIM", "stale seed must be dropped");
    assert!(
        rebuilt.contains("EDITED_PERSONALITY_MARKER"),
        "rebuilt brain must reflect the on-disk edit, got:\n{rebuilt}"
    );
}

#[test]
fn rebuild_is_cached_after_a_change_until_the_next_change() {
    let dir = TempDir::new().unwrap();
    let soul = dir.path().join("SOUL.md");
    write_with_mtime(&soul, "--- SOUL.md ---\nv1\n", t0());

    let loader = BrainLoader::new(dir.path().to_path_buf());
    let rebuild = BrainRebuild::new(loader, None, true, false, "SEED".to_string(), None, false);

    // Trigger a rebuild.
    write_with_mtime(
        &soul,
        "--- SOUL.md ---\nMARKER_V2\n",
        t0() + Duration::from_secs(60),
    );
    let first = rebuild.render();
    assert!(first.contains("MARKER_V2"));

    // No further mtime change → identical render returned from cache (warm
    // prompt cache, byte-for-byte stable).
    assert_eq!(rebuild.render(), first, "no change → cached render reused");
}

#[test]
fn no_rebuild_handle_falls_back_to_static_brain() {
    // A loader pointed at an empty dir, used only to prove the seed survives
    // when nothing changes — the static-brain fallback path is exercised by
    // the existing agent tests via `default_system_brain`; here we assert the
    // seed is what render hands back when the dir has no newer files.
    let dir = TempDir::new().unwrap();
    let loader = BrainLoader::new(dir.path().to_path_buf());
    let rebuild = BrainRebuild::new(
        loader,
        None,
        true,
        false,
        "ONLY-SEED".to_string(),
        None,
        false,
    );
    assert_eq!(rebuild.render(), "ONLY-SEED");
}

/// Gap 1: the directive scan must follow `/cd`. The live working-directory
/// handle is shared with tool execution; mutating it (as `/cd` does) must make
/// the next render rebuild against the new directory, not the frozen startup
/// one.
#[test]
fn render_follows_working_directory_change() {
    let proj_a = TempDir::new().unwrap();
    std::fs::write(proj_a.path().join("AGENTS.md"), "project a rules").unwrap();
    let proj_b = TempDir::new().unwrap();
    std::fs::write(proj_b.path().join("CLAUDE.md"), "project b rules").unwrap();

    let brain_dir = TempDir::new().unwrap();
    let loader = BrainLoader::new(brain_dir.path().to_path_buf());
    let cwd = Arc::new(RwLock::new(proj_a.path().to_path_buf()));
    let rebuild = BrainRebuild::new(
        loader,
        Some(RuntimeInfo::default()),
        true,
        false,
        "SEED".to_string(),
        Some(Arc::clone(&cwd)),
        false, // interactive surface
    );

    // Simulate `/cd` into project B.
    *cwd.write().unwrap() = proj_b.path().to_path_buf();
    let out = rebuild.render();

    let b_header = format!(
        "Project Directive Files (in {}/)",
        collapse_home(proj_b.path())
    );
    assert!(
        out.contains(&b_header),
        "render must scan the new cwd (project B), got:\n{out}"
    );
    assert!(
        !out.contains(&collapse_home(proj_a.path())),
        "the old startup directory (project A) must not leak into the index"
    );
}

/// Gap 2: adding a directive file to the current directory must invalidate the
/// cache even though no `~/.opencrabs` brain file changed.
#[test]
fn render_rebuilds_when_a_directive_file_is_added() {
    let proj = TempDir::new().unwrap(); // starts with no directive files
    let brain_dir = TempDir::new().unwrap();
    let loader = BrainLoader::new(brain_dir.path().to_path_buf());
    let cwd = Arc::new(RwLock::new(proj.path().to_path_buf()));
    let rebuild = BrainRebuild::new(
        loader,
        Some(RuntimeInfo::default()),
        true,
        false,
        "SEED".to_string(),
        Some(Arc::clone(&cwd)),
        false, // interactive surface
    );

    // No directive files yet and no brain-file change → warm seed.
    assert_eq!(rebuild.render(), "SEED");

    // Drop an AGENTS.md into the project → directive mtime advances → rebuild.
    std::fs::write(proj.path().join("AGENTS.md"), "freshly added rules").unwrap();
    let out = rebuild.render();
    assert_ne!(
        out, "SEED",
        "a new directive file must invalidate the cache"
    );
    assert!(
        out.contains("Project Directive Files"),
        "rebuilt brain must include the directive index, got:\n{out}"
    );
}
