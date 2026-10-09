//! #2008: a rebuild carries every session's own working directory, not just the
//! global one.
//!
//! `rebuild_agent_service` runs on every `/models` switch and on config reload.
//! It copied the GLOBAL directory onto the new service (`.with_working_directory`)
//! and then snapshotted and re-applied every per-session provider and model pin,
//! but did nothing for working directories. So one `/models` call in pane A
//! reverted pane B to the global directory, and pane B's Runtime Info then
//! reported a directory it had never chosen.

use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use crate::brain::agent::service::AgentService;
use crate::brain::provider::Provider;
use crate::db::Database;
use crate::services::ServiceContext;
use crate::tests::agent_service_mocks::MockProvider;

async fn make_service() -> AgentService {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    AgentService::new_for_test(provider, context).await
}

/// The replay contract: snapshot the old service, build a new one carrying only
/// the global, replay the snapshot, and every pane is still in its own repo.
#[tokio::test]
async fn a_rebuild_replays_every_sessions_own_directory() {
    let old = make_service().await;
    let global = PathBuf::from("/tmp/rebuild-global");
    old.set_working_directory(global.clone());

    let pane_a = Uuid::new_v4();
    let pane_b = Uuid::new_v4();
    let untouched = Uuid::new_v4();
    old.set_session_only_working_directory(pane_a, PathBuf::from("/tmp/rebuild-pane-a"));
    old.set_session_only_working_directory(pane_b, PathBuf::from("/tmp/rebuild-pane-b"));

    // What the rebuild reads: the global for the builder, the per-session map for replay.
    let carried_global = old.working_directory().read().unwrap().clone();
    let snapshot = old.session_working_dir_snapshot();

    // A fresh service as `rebuild_agent_service` builds it: provider rebuilt from
    // config, global carried, no session state of its own yet.
    let fresh = make_service()
        .await
        .with_working_directory(carried_global.clone());
    for (sid, dir) in snapshot {
        fresh.set_session_only_working_directory(sid, dir);
    }

    assert_eq!(
        fresh.get_working_directory_for_session(pane_a),
        PathBuf::from("/tmp/rebuild-pane-a"),
        "pane A lost its own directory across the rebuild (#2008)"
    );
    assert_eq!(
        fresh.get_working_directory_for_session(pane_b),
        PathBuf::from("/tmp/rebuild-pane-b"),
        "pane B lost its own directory across the rebuild (#2008)"
    );
    // A session with no directory of its own still resolves to the global (#703).
    assert_eq!(
        fresh.get_working_directory_for_session(untouched),
        carried_global,
        "a session that never chose a directory must follow the global"
    );
}

/// The wiring. The test above proves the mechanism works; it cannot prove the
/// rebuild USES it, and the rebuild path needs a loaded config and a live
/// provider, which a unit test must not depend on. So pin the call directly: if
/// someone drops the snapshot or the replay from `rebuild_agent_service`, this
/// fails.
#[test]
fn rebuild_agent_service_carries_the_session_directory_snapshot() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/app/state.rs"))
        .expect("read tui/app/state.rs");

    // Scope the scan to the function body: take the lines from the signature up
    // to the next method at the same indent level.
    let mut lines = src.lines();
    let mut body: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in lines.by_ref() {
        if line.contains("async fn rebuild_agent_service(") {
            inside = true;
            continue;
        }
        if inside {
            let is_next_method = line
                .strip_prefix("    ")
                .is_some_and(|rest| rest.starts_with("pub ") || rest.starts_with("fn "));
            if is_next_method {
                break;
            }
            body.push(line);
        }
    }
    assert!(inside, "rebuild_agent_service must still exist in state.rs");
    let body = body.join("\n");

    assert!(
        body.contains(".session_working_dir_snapshot()"),
        "#2008: rebuild_agent_service must snapshot the per-session working \
         directories before it replaces the service"
    );
    assert!(
        body.contains(".set_session_only_working_directory("),
        "#2008: rebuild_agent_service must replay the snapshot onto the new \
         service without moving the global"
    );
}
