//! Tests for `wait_for_profile_unlock` — the backoff loop that gives
//! the user's main browser a few seconds to release its profile lock
//! before we fall back to the empty opencrabs profile.
//!
//! Scenario the retry protects against: user just clicked the Brave
//! dock icon, Chrome's `SingletonLock` appears for ~1–3 seconds while
//! the browser starts up, opencrabs tries to launch in that window,
//! sees the lock, falls through to the empty fallback profile —
//! user's cookies and logins gone.

#![cfg(feature = "browser")]

use crate::brain::tools::browser::wait_for_profile_unlock;
use std::fs;

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "opencrabs-profile-wait-test-{}-{}",
        tag,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mk scratch");
    dir
}

#[tokio::test]
async fn returns_true_immediately_when_unlocked() {
    let dir = scratch("unlocked");
    // No lock file → first poll sees unlocked, returns true instantly.
    let start = std::time::Instant::now();
    assert!(wait_for_profile_unlock(&dir, 10_000).await);
    assert!(
        start.elapsed() < std::time::Duration::from_millis(100),
        "unlocked profile must return immediately, took {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn returns_true_after_lock_removed_during_wait() {
    // Simulates the real scenario: lock exists at launch time, the
    // main browser finishes starting up, releases the lock ~400ms
    // later, we detect it on the next poll and proceed.
    let dir = scratch("late-release");
    fs::write(dir.join("SingletonLock"), b"main-browser-starting").unwrap();
    let lock = dir.join("SingletonLock");

    let remover = tokio::spawn({
        let lock = lock.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            let _ = fs::remove_file(&lock);
        }
    });

    let start = std::time::Instant::now();
    let result = wait_for_profile_unlock(&dir, 10_000).await;
    let elapsed = start.elapsed();

    let _ = remover.await;
    assert!(result, "should return true once lock is released");
    assert!(
        elapsed < std::time::Duration::from_millis(2_000),
        "should detect release within one or two backoff steps, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn returns_false_when_cap_elapses() {
    // Lock never goes away → cap exhausts → returns false so the
    // caller can fall through to the empty opencrabs profile.
    let dir = scratch("permanent");
    fs::write(dir.join("SingletonLock"), b"stays-locked").unwrap();

    let start = std::time::Instant::now();
    let result = wait_for_profile_unlock(&dir, 600).await; // short cap
    let elapsed = start.elapsed();

    assert!(!result, "permanently-locked profile must return false");
    assert!(
        elapsed >= std::time::Duration::from_millis(600),
        "must honour the cap — elapsed was {:?}",
        elapsed
    );
    assert!(
        elapsed < std::time::Duration::from_millis(1_500),
        "shouldn't block much beyond the cap — elapsed was {:?}",
        elapsed
    );
}

#[tokio::test]
async fn zero_cap_returns_false_without_sleeping() {
    // Belt-and-braces: cap_ms = 0 means "one check only, no wait".
    // If locked on first check, return false immediately.
    let dir = scratch("zero-cap");
    fs::write(dir.join("SingletonLock"), b"x").unwrap();

    let start = std::time::Instant::now();
    let result = wait_for_profile_unlock(&dir, 0).await;
    assert!(!result);
    assert!(
        start.elapsed() < std::time::Duration::from_millis(100),
        "zero cap must not incur any real sleep"
    );
}
