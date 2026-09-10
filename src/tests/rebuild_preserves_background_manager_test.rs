//! A provider switch must not drop the background-task manager (#1504).
//!
//! `rebuild_agent_service` rebuilds the whole AgentService when the provider
//! changes. It carried over the approval, progress, queue, sudo, ssh, subagent
//! and session-update wiring but not the enqueue callback, so the rebuilt
//! service had `background_manager == None` — and long commands (cargo
//! test/build/clippy, …) stopped detaching for every session after the switch.
//! The gate is `context.background_manager.is_some()` in bash's detach path.
//!
//! The rebuild now carries the EXISTING manager across, so it keeps working and
//! any in-flight detached task stays tracked by the same manager.

use crate::brain::agent::service::{AgentService, MessageEnqueueCallback, QueuedUserMessage};
use crate::db::Database;
use crate::services::ServiceContext;
use crate::tests::agent_service_mocks::MockProvider;
use std::sync::Arc;

async fn wired_service() -> AgentService {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let cb: MessageEnqueueCallback = Arc::new(|_sid, _msg: QueuedUserMessage| {});
    AgentService::new_for_test(Arc::new(MockProvider), context)
        .await
        .with_message_enqueue_callback(Some(cb))
}

#[tokio::test]
async fn a_wired_service_has_a_background_manager() {
    let svc = wired_service().await;
    assert!(
        svc.background_manager().is_some(),
        "wiring the enqueue callback must create the background manager"
    );
}

#[tokio::test]
async fn carrying_the_manager_across_a_rebuild_keeps_it_and_reuses_the_same_arc() {
    let old = wired_service().await;
    let old_mgr = old
        .background_manager()
        .expect("wired service has a manager");

    // Simulate rebuild_agent_service: a fresh service (as a provider switch
    // builds) has NO manager until we carry the old one across.
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let rebuilt = AgentService::new_for_test(Arc::new(MockProvider), context).await;
    assert!(
        rebuilt.background_manager().is_none(),
        "a fresh service without the enqueue callback has no manager — the bug's precondition"
    );

    let rebuilt = rebuilt
        .with_existing_background_manager(old.background_manager(), old.message_enqueue_callback());

    let new_mgr = rebuilt
        .background_manager()
        .expect("the rebuilt service must keep the background manager (#1504)");
    assert!(
        Arc::ptr_eq(&old_mgr, &new_mgr),
        "the rebuild must reuse the SAME manager Arc, or in-flight detached tasks are orphaned"
    );
    assert!(
        rebuilt.message_enqueue_callback().is_some(),
        "the enqueue route must survive too, or a finished background task cannot wake the session"
    );
}
