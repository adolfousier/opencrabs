//! Per-profile cron scheduler lock (#444).
//!
//! Duplicate cron execution came from two schedulers polling the same
//! profile's `cron_jobs` table (a multi-profile `daemon` plus a `-p <profile>`
//! daemon, or the TUI). The lock guarantees exactly one scheduler per profile
//! machine-wide: whoever holds it schedules, everyone else skips. These tests
//! pin that contract against a TempDir so they never touch the real
//! `~/.opencrabs/locks/`.
//!
//! `#![cfg(unix)]`: the exclusion is a `flock`, compiled only on unix. On
//! Windows the lock is a best-effort no-op, so the "second caller is denied"
//! contract can't hold there.
#![cfg(unix)]

use crate::config::profile::acquire_scheduler_lock_in;
use tempfile::TempDir;

#[test]
fn first_caller_acquires_the_profile_scheduler() {
    let dir = TempDir::new().unwrap();
    let guard = acquire_scheduler_lock_in(dir.path(), "default");
    assert!(guard.is_some(), "an unheld profile lock must be acquirable");
}

#[test]
fn second_caller_for_same_profile_is_denied_while_held() {
    let dir = TempDir::new().unwrap();
    let held = acquire_scheduler_lock_in(dir.path(), "ops");
    assert!(held.is_some(), "first acquire should win");

    // Same profile, lock still held → the second scheduler must be turned away
    // so it never polls the same cron_jobs table in parallel (#444).
    let denied = acquire_scheduler_lock_in(dir.path(), "ops");
    assert!(
        denied.is_none(),
        "a second scheduler for a held profile must be denied"
    );
}

#[test]
fn different_profiles_do_not_contend() {
    let dir = TempDir::new().unwrap();
    let ops = acquire_scheduler_lock_in(dir.path(), "ops");
    let family = acquire_scheduler_lock_in(dir.path(), "family");
    assert!(ops.is_some(), "ops lock is independent");
    assert!(
        family.is_some(),
        "family lock must not be blocked by the ops lock"
    );
}

#[test]
fn lock_is_released_on_drop_and_can_be_retaken() {
    let dir = TempDir::new().unwrap();
    {
        let guard = acquire_scheduler_lock_in(dir.path(), "default");
        assert!(guard.is_some(), "first acquire should win");
        // guard dropped at end of block → flock released
    }
    // Under heavy CI load (8k+ tests in parallel on macOS runners), the
    // kernel's flock release may not be visible to a fresh open+flock in
    // the very next instruction. Retry briefly rather than asserting on a
    // single attempt — the contract is that the lock is *eventually*
    // retakeable, not that it is retakeable within one syscall.
    let reacquired = (0..10).find_map(|_| {
        acquire_scheduler_lock_in(dir.path(), "default").or_else(|| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            None
        })
    });
    assert!(
        reacquired.is_some(),
        "after the holder drops, the profile lock must be retakeable (a crashed daemon must not wedge scheduling)"
    );
}
