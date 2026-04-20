//! Tests for the browser CDP handler liveness check.
//!
//! Pins the P1 fix: `ensure_browser` now detects when the underlying
//! CDP handler task has finished (indicating the Chrome process died,
//! the socket broke, or the user closed the window) and relaunches
//! instead of handing back a zombie `Browser` handle that produces
//! `Navigation failed: channel closed` on the next goto call
//! (2026-04-19 09:49 log).

#![cfg(feature = "browser")]

use crate::brain::tools::browser::handler_is_dead;

/// `None` handle means "never launched or already torn down" — the
/// caller must treat this as dead so the next branch in
/// `ensure_browser` runs a full launch.
#[test]
fn no_handle_means_dead() {
    assert!(handler_is_dead(None));
}

/// A still-running handle is alive. We simulate Chrome's long-lived
/// event-stream pump with a task that sleeps far longer than the test
/// runtime.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn running_task_is_alive() {
    let handle = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    });

    // Yield once so the task gets a chance to start — under
    // current_thread + paused time this is essentially instant.
    tokio::task::yield_now().await;

    assert!(
        !handler_is_dead(Some(&handle)),
        "a sleeping task should be considered alive"
    );
    handle.abort();
}

/// A handle whose task has completed is dead. Matches the production
/// failure: Chrome died, the CDP event loop in manager.rs exited its
/// `while let Some(event) = handler.next().await { ... }` loop, the
/// task finished, and `is_finished()` returned true.
#[tokio::test]
async fn completed_task_is_dead() {
    // Matches production: the CDP handler's `while let Some(event) =
    // handler.next().await { ... }` loop exits when the socket closes,
    // so the task finishes. We don't `.await` the handle here because
    // that'd consume it — instead we poll `is_finished()` until the
    // runtime flips the flag, which happens after the task body runs.
    let handle = tokio::spawn(async {});
    for _ in 0..128 {
        if handle.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        handler_is_dead(Some(&handle)),
        "a completed task should be considered dead"
    );
}

/// An aborted task is also dead. Matches the `set_headless` teardown
/// path where the previous handler is aborted before a new one spawns.
#[tokio::test]
async fn aborted_task_is_dead() {
    let handle = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    });
    handle.abort();

    // Give the runtime a tick to mark the abort as finished.
    for _ in 0..16 {
        if handle.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert!(
        handler_is_dead(Some(&handle)),
        "an aborted task should be considered dead"
    );
}
