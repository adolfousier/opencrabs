//! #722 phase 2: the background-task manager runs a command detached and, on
//! completion, enqueues a system QueuedUserMessage into the originating session.

use crate::brain::agent::service::MessageEnqueueCallback;
use crate::brain::agent::service::QueuedUserMessage;
use crate::brain::agent::service::background_tasks::{
    BackgroundTaskManager, CmdResult, completion_message, format_elapsed, short_label, tail_lines,
};
use crate::brain::agent::service::restart_recovery;
use crate::brain::agent::service::session_routes;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[test]
fn tail_keeps_last_n_lines() {
    let text = (1..=100)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let tail = tail_lines(&text, 3);
    assert_eq!(tail, "98\n99\n100");
    // Fewer lines than n -> whole text.
    assert_eq!(tail_lines("a\nb", 10), "a\nb");
}

#[test]
fn completion_message_reflects_success_and_failure() {
    let ok = completion_message(
        "cargo test",
        "cargo test --all-features",
        &CmdResult {
            success: true,
            code: 0,
            output: "test result: ok. 5 passed".into(),
        },
        12.0,
    );
    assert!(ok.context_text.contains("exit 0 (success)"));
    assert!(ok.context_text.contains("cargo test --all-features"));
    assert!(ok.context_text.contains("Do not re-run"));
    assert!(ok.display_text.contains("finished"));
    // #15: the typed receipt payload rides along for the echo card.
    let meta = ok.bg_meta.expect("bg completion carries BgTaskMeta");
    assert!(meta.success);
    assert_eq!(meta.label, "cargo test");
    assert_eq!(meta.elapsed_secs, 12.0);
    assert_eq!(meta.tail, "test result: ok. 5 passed");

    let fail = completion_message(
        "build",
        "cargo build",
        &CmdResult {
            success: false,
            code: 101,
            output: "error[E0001]".into(),
        },
        3.0,
    );
    assert!(fail.context_text.contains("exit 101 (failure)"));
    assert!(fail.display_text.contains("failed"));
    let meta = fail.bg_meta.expect("failed completion still carries meta");
    assert!(!meta.success);
    assert_eq!(meta.elapsed_secs, 3.0);
}

#[test]
fn format_elapsed_buckets_match_the_spec() {
    assert_eq!(format_elapsed(0.4), "0s");
    assert_eq!(format_elapsed(42.0), "42s");
    assert_eq!(format_elapsed(59.6), "1m 0s"); // rounds up into the minute bucket
    assert_eq!(format_elapsed(185.0), "3m 5s");
    assert_eq!(format_elapsed(3599.4), "59m 59s");
    assert_eq!(format_elapsed(3720.0), "1h 2m");
}

#[test]
fn short_label_takes_the_command_after_cd() {
    assert_eq!(short_label("cd ~/proj && cargo test"), "cargo test");
    assert_eq!(short_label("cargo build"), "cargo build");
}

#[tokio::test]
// The test_guard serializes suites touching the process-global parked-queue
// state; holding it across the polling `.await`s below is the entire point —
// same shape as the session_notify suite (#22).
#[allow(clippy::await_holding_lock)]
async fn spawn_command_enqueues_on_completion() {
    // register_session_route → claim_session touches the process-global
    // parked-queue state, so serialize against other suites that do too.
    let _guard = restart_recovery::test_guard();
    #[allow(clippy::type_complexity)]
    let recorded: Arc<Mutex<Vec<(Uuid, QueuedUserMessage)>>> = Arc::new(Mutex::new(Vec::new()));
    let rec = recorded.clone();
    let enqueue: MessageEnqueueCallback = Arc::new(move |sid, msg| {
        rec.lock().unwrap().push((sid, msg));
    });

    let mgr = Arc::new(BackgroundTaskManager::new());
    let sid = Uuid::new_v4();
    // The manager no longer carries its own route (fork #19 — delivery goes
    // through the one gated route, which resolves the session's registered
    // route), so claim the session the way a channel would: register the
    // recording callback as its route.
    session_routes::register_session_route(sid, enqueue);
    let cwd = std::env::temp_dir();

    mgr.clone().spawn_command(
        sid,
        cwd,
        "echo probe".to_string(),
        "echo BG_DONE_MARKER".to_string(),
    );

    // Wait (bounded) for the detached command to finish and enqueue.
    let mut waited = 0;
    while recorded.lock().unwrap().is_empty() && waited < 50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += 1;
    }

    let r = recorded.lock().unwrap();
    assert_eq!(r.len(), 1, "completion should have enqueued exactly once");
    assert_eq!(r[0].0, sid);
    assert!(r[0].1.context_text.contains("BG_DONE_MARKER"));
    assert!(r[0].1.context_text.contains("exit 0 (success)"));
    // Running count drops back to zero after completion.
    assert_eq!(mgr.running_for(sid), 0);
}

/// #1748: `spawn_command_with_hook` hands the outcome to the hook INSTEAD of
/// the generic session delivery; the detached rebuild's hook exec-replaces
/// the process on success, so an in-memory enqueue would be orphaned
/// mid-flight. Same lifecycle (timer, status file, DB accounting), different
/// completion route.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn spawn_command_with_hook_replaces_generic_delivery() {
    // register_session_route → claim_session touches the process-global
    // parked-queue state, so serialize against other suites that do too.
    let _guard = restart_recovery::test_guard();
    #[allow(clippy::type_complexity)]
    let recorded: Arc<Mutex<Vec<(Uuid, QueuedUserMessage)>>> = Arc::new(Mutex::new(Vec::new()));
    let rec = recorded.clone();
    let enqueue: MessageEnqueueCallback = Arc::new(move |sid, msg| {
        rec.lock().unwrap().push((sid, msg));
    });

    let mgr = Arc::new(BackgroundTaskManager::new());
    let sid = Uuid::new_v4();
    session_routes::register_session_route(sid, enqueue);

    // The hook records what it learned; the session route must stay silent.
    let hooked: Arc<Mutex<Option<(bool, bool)>>> = Arc::new(Mutex::new(None));
    let hk = hooked.clone();
    let hook: crate::brain::agent::service::background_tasks::CompletionHook = Box::new(
        move |ctx: crate::brain::agent::service::background_tasks::HookContext| {
            Box::pin(async move {
                *hk.lock().unwrap() = Some((ctx.result.success, ctx.elapsed_secs >= 0.0));
            })
        },
    );

    mgr.clone().spawn_command_with_hook(
        sid,
        std::env::temp_dir(),
        "hook probe".to_string(),
        "echo HOOK_DONE_MARKER".to_string(),
        hook,
    );

    // Wait (bounded) for the detached command to finish and fire the hook.
    let mut waited = 0;
    while hooked.lock().unwrap().is_none() && waited < 50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += 1;
    }

    let h = hooked.lock().unwrap();
    let (success, elapsed_ok) = h.expect("hook must fire on completion");
    assert!(success, "echo must report success to the hook");
    assert!(elapsed_ok, "hook carries the command runtime");

    // The generic delivery did NOT run: the session route got nothing.
    assert!(
        recorded.lock().unwrap().is_empty(),
        "a hook replaces generic session delivery"
    );
    // Running count drops back to zero after completion.
    assert_eq!(mgr.running_for(sid), 0);
}
