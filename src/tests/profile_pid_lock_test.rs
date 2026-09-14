//! Regression for #192: a corrupted or zero PID in a channel-credential lock
//! file must NOT look like a live owner, or the lock wedges forever and the
//! channel that owns the credential (e.g. Telegram) can never start.

use crate::config::profile::{is_pid_alive, parse_lock_owner_pid};

#[test]
fn pid_zero_is_never_alive() {
    // On Unix `kill(0, 0)` signals the CALLER's process group and succeeds, so
    // without the explicit guard this would wrongly report "alive".
    assert!(!is_pid_alive(0));
}

#[test]
fn pid_max_or_negative_wrap_is_never_alive() {
    // u32::MAX as i32 is -1. In POSIX kill(-1, 0) signals all processes the caller
    // has permission to signal and returns 0, which would look "alive" without
    // the signed_pid > 0 check.
    assert!(!is_pid_alive(u32::MAX));
    assert!(!is_pid_alive(i32::MAX as u32 + 1));
}

#[test]
fn current_process_is_alive() {
    // Our own PID is, by definition, a live process.
    assert!(is_pid_alive(std::process::id()));
}

#[test]
fn lock_owner_pid_rejects_corrupt_or_zero() {
    // A real PID parses.
    assert_eq!(parse_lock_owner_pid("103104"), Some(103104));
    assert_eq!(parse_lock_owner_pid("  103104 "), Some(103104));

    // Zero, empty, and corrupted (concatenated entries) name no live owner →
    // None → the caller takes the stale lock over instead of bailing.
    assert_eq!(parse_lock_owner_pid("0"), None);
    assert_eq!(parse_lock_owner_pid(""), None);
    assert_eq!(parse_lock_owner_pid("101528ops:103104family:101507"), None);
    assert_eq!(parse_lock_owner_pid("notapid"), None);
}
