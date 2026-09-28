//! One writer at a time, per path.
//!
//! Agents sharing a working tree write the same bytes with no arbitration.
//! Sub-agents get their own checkout now (#1151), but that isolation degrades
//! to the parent's directory outside a repository or when git refuses, and in
//! those cases a fan-out is back to sharing one tree.
//!
//! Nothing else here guards a file. The locks that exist are process- or
//! session-scoped: one instance per profile, one scheduler, one credential
//! holder, one turn per Telegram session.
//!
//! The rule is enforced by the write itself rather than by asking agents to
//! check first. A prompt-layer gate would not reach two sub-agents colliding
//! in the degraded path, since neither reads the parent's instructions, and
//! nothing would reveal that one had skipped the check.
//!
//! Deliberately weak in three ways, because a lock that can block real work is
//! worse than the collision it prevents:
//!
//! - Advisory and held across one write, never across a turn.
//! - A contended write waits briefly, then proceeds with a warning.
//! - Every failure to lock degrades to writing anyway.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::Duration;
// `Instant` paces the flock wait loop below, which is unix-only.
#[cfg(unix)]
use std::time::Instant;

/// How long to wait for another writer before going ahead regardless. Long
/// enough to serialise two agents saving the same file, short enough that a
/// crashed holder costs a pause rather than a wedge.
const WAIT_FOR_HOLDER: Duration = Duration::from_millis(750);

/// Gap between attempts while waiting.
const RETRY_EVERY: Duration = Duration::from_millis(25);

/// Held for the duration of one write. The lock is released when this drops,
/// including on a panic, so a writer that dies mid-write does not keep it.
#[derive(Debug)]
pub(crate) struct PathWriteLock {
    /// Kept because the kernel releases the `flock` when this descriptor
    /// closes. Nothing reads it.
    #[allow(dead_code)]
    file: std::fs::File,
    /// Whether this writer actually holds the lock. `false` means it waited,
    /// gave up, and is writing anyway.
    held: bool,
}

impl PathWriteLock {
    /// Did this writer get the lock, or is it proceeding uncontended-anyway?
    pub(crate) fn is_held(&self) -> bool {
        self.held
    }
}

/// Where a path's lock file lives: under the profile home, keyed by a hash of
/// the absolute path.
///
/// Beside the target would be simpler and wrong: it would litter the user's
/// tree with lock files, and a repository would see them as untracked
/// additions the agent then tries to explain.
fn lock_path_for(target: &Path) -> Option<PathBuf> {
    let absolute = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&absolute, &mut hasher);
    let key = format!("{:016x}", std::hash::Hasher::finish(&hasher));

    let dir = crate::config::profile::resolve_profile_home().join("locks/paths");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!("path lock: cannot create {}: {e}", dir.display());
        return None;
    }
    Some(dir.join(format!("{key}.lock")))
}

/// Resolve `target`'s lock path, on the CALLING task.
///
/// This is deliberately separate from [`acquire_at`]. The profile home can be
/// a `tokio::task_local` override (`resolve_profile_home`), and a
/// `spawn_blocking` closure runs on a NEW task that cannot see it. Resolving
/// inside the blocking closure silently keys the lock to the real home, so two
/// writers sharing one scoped context take two DIFFERENT locks, both report
/// `is_held()` true, and the arbitration fails without a word — the lock looks
/// applied and does nothing. Measure the path here, block there.
pub(crate) fn lock_path(target: &Path) -> Option<PathBuf> {
    lock_path_for(target)
}

/// Take the lock at an already-resolved lock-file path, waiting briefly for
/// another writer.
///
/// Never fails: the returned guard reports whether the lock was actually
/// acquired, and the caller writes either way. On a platform or filesystem
/// where locking is unavailable this is a no-op that reports `is_held()`
/// false, which is exactly today's behaviour. Safe to call from
/// `spawn_blocking`, since it resolves nothing that lives in a task-local.
pub(crate) fn acquire_at(lock_path: &Path) -> Option<PathWriteLock> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
        .ok()?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let deadline = Instant::now() + WAIT_FOR_HOLDER;
        loop {
            match crate::config::flock::exclusive(fd, true) {
                crate::config::flock::FlockOutcome::Acquired => {
                    return Some(PathWriteLock { file, held: true });
                }
                crate::config::flock::FlockOutcome::Held => {
                    if Instant::now() >= deadline {
                        // Another writer is still in there. Proceed rather
                        // than refuse: a blocked write is worse than an
                        // interleaved one, and the caller reports the overlap.
                        return Some(PathWriteLock { file, held: false });
                    }
                    std::thread::sleep(RETRY_EVERY);
                }
                crate::config::flock::FlockOutcome::Failed(e) => {
                    tracing::debug!(
                        "path lock: cannot lock {}: {e} — writing without it",
                        lock_path.display()
                    );
                    return Some(PathWriteLock { file, held: false });
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        Some(PathWriteLock { file, held: false })
    }
}

/// Take the write lock for `target`, waiting briefly for another writer.
/// Resolves and locks in one call — the common shape, where the caller is
/// already on the task that can see the profile home.
pub(crate) fn acquire(target: &Path) -> Option<PathWriteLock> {
    let lock_path = lock_path_for(target)?;
    acquire_at(&lock_path)
}

/// The note appended to a tool result when a write went ahead without the
/// lock, so an overlap is visible instead of silent.
pub(crate) fn contention_notice(target: &Path) -> String {
    format!(
        "\n\nNote: another agent was writing `{}` at the same time. Both writes went \
         through, so this file may not contain what either intended — re-read it before \
         relying on it.",
        target.display()
    )
}
