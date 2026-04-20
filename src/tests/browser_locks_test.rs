//! Tests for the browser profile stale-lock sweeper.
//!
//! Pins the P0 fix: before launching Chrome against the opencrabs-owned
//! fallback profile, we sweep leftover `SingletonLock` / `SingletonSocket`
//! / `Lock` files. A previous opencrabs Chrome process that crashed leaves
//! these behind, and the next launch refuses to start with
//! `"Failed to create SingletonLock: File exists (17)"` (see the 2026-04-11
//! 16:57 and 2026-04-17 15:00 log incidents).

#![cfg(feature = "browser")]

use crate::brain::tools::browser::{LOCK_FILES, clean_stale_locks};
use std::fs;
use std::path::PathBuf;

/// Per-test scratch directory. Each test gets its own so they can run in
/// parallel without stepping on each other.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "opencrabs-browser-locks-test-{}-{}",
        tag,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mk scratch");
    dir
}

#[test]
fn removes_singleton_lock_when_present() {
    let dir = scratch("singleton");
    let lock = dir.join("SingletonLock");
    fs::write(&lock, b"12345").unwrap();
    assert!(lock.exists(), "precondition: lock should exist");

    clean_stale_locks(&dir);

    assert!(
        !lock.exists(),
        "SingletonLock should be removed by clean_stale_locks"
    );
}

#[test]
fn removes_all_known_lock_variants() {
    let dir = scratch("all-variants");
    for name in LOCK_FILES {
        fs::write(dir.join(name), b"stale").unwrap();
    }
    for name in LOCK_FILES {
        assert!(dir.join(name).exists(), "precondition: {name} exists");
    }

    clean_stale_locks(&dir);

    for name in LOCK_FILES {
        assert!(
            !dir.join(name).exists(),
            "{name} should be removed by clean_stale_locks"
        );
    }
}

#[test]
fn preserves_unrelated_files() {
    // The profile directory holds real state (cookies, history, prefs).
    // The sweeper must only touch the three named lock artefacts and
    // leave everything else alone — losing the user's cookies would be
    // a far worse regression than a stale lock.
    let dir = scratch("unrelated");
    fs::write(dir.join("SingletonLock"), b"stale").unwrap();
    fs::write(dir.join("Cookies"), b"real-cookie-data").unwrap();
    fs::write(dir.join("Preferences"), b"{\"profile\":{}}").unwrap();
    fs::create_dir_all(dir.join("Default/Cache")).unwrap();
    fs::write(dir.join("Default/Cache/index"), b"cache").unwrap();

    clean_stale_locks(&dir);

    assert!(!dir.join("SingletonLock").exists(), "lock removed");
    assert!(dir.join("Cookies").exists(), "Cookies preserved");
    assert!(dir.join("Preferences").exists(), "Preferences preserved");
    assert!(
        dir.join("Default/Cache/index").exists(),
        "nested cache preserved"
    );
}

#[test]
fn noop_on_clean_profile_directory() {
    // Running the sweeper on a profile that has no locks must not error,
    // not log warnings in a way that changes test expectations, and must
    // leave the directory intact.
    let dir = scratch("clean");
    fs::write(dir.join("Cookies"), b"x").unwrap();

    clean_stale_locks(&dir); // should not panic

    assert!(dir.join("Cookies").exists());
    for name in LOCK_FILES {
        assert!(!dir.join(name).exists(), "still no {name}");
    }
}

#[test]
fn noop_on_missing_directory() {
    // Directory doesn't exist yet. The first launch path creates the
    // directory, so this shouldn't happen in practice, but the sweeper
    // must be robust to it — the individual remove_file calls error but
    // the sweeper swallows them with a warn log.
    let dir = scratch("missing");
    let nonexistent = dir.join("does-not-exist");
    assert!(!nonexistent.exists());

    clean_stale_locks(&nonexistent); // must not panic
}
