//! #2006: the notice that tells the model the working directory just changed is
//! queued per session. It used to live on the app, so a `/cd` in one pane rode
//! into whichever session sent next, and that session's prompt claimed a
//! directory it had never chosen.

use std::sync::Arc;
use uuid::Uuid;

use crate::brain::agent::service::AgentService;
use crate::brain::provider::Provider;
use crate::db::Database;
use crate::services::ServiceContext;
use crate::tests::agent_service_mocks::MockProvider;
use crate::tui::app::App;

async fn app() -> App {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let service = Arc::new(AgentService::new_for_test(provider, context.clone()).await);
    #[cfg(feature = "whatsapp")]
    {
        App::new(
            service,
            context,
            Arc::new(crate::channels::whatsapp::WhatsAppState::new()),
        )
    }
    #[cfg(not(feature = "whatsapp"))]
    {
        App::new(service, context)
    }
}

/// The leak itself: a hint queued for session A must not be handed to session B,
/// and must still reach A.
#[tokio::test]
async fn a_hint_queued_for_one_session_never_reaches_another() {
    let mut app = app().await;
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();

    app.push_pending_context_for(a, "[User changed working directory to: /tmp/a]".to_string());

    assert!(
        app.take_pending_context_for(b).is_empty(),
        "session B inherited A's notice, which is the cross-session leak (#2006)"
    );

    let got = app.take_pending_context_for(a);
    assert_eq!(
        got,
        vec!["[User changed working directory to: /tmp/a]".to_string()]
    );

    // Draining is one-shot: the same notice cannot be prepended to a second turn.
    assert!(app.take_pending_context_for(a).is_empty());
    assert!(
        !app.pending_context.contains_key(&a),
        "a drained session must not leave an empty queue behind in the map"
    );
}

/// Two panes queueing at once stay in their own lanes and keep their order.
#[tokio::test]
async fn each_session_keeps_its_own_hints_in_order() {
    let mut app = app().await;
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();

    app.push_pending_context_for(a, "first for a".to_string());
    app.push_pending_context_for(b, "only for b".to_string());
    app.push_pending_context_for(a, "second for a".to_string());

    assert_eq!(
        app.take_pending_context_for(a),
        vec!["first for a".to_string(), "second for a".to_string()]
    );
    assert_eq!(
        app.take_pending_context_for(b),
        vec!["only for b".to_string()]
    );
}

/// Why this one is structural. The real `/cd` applier also calls
/// `process::set_current_dir`, so a test that drove it would move the cwd of
/// the whole test binary and delete it when the temp dir dropped. Two other
/// tests read `std::env::current_dir()` (`em_dash_guard_test.rs:56`,
/// `ralph_verification_gate_test.rs:396`), so that test would poison them at
/// random. The map above proves a hint cannot cross sessions; this proves the
/// `/cd` write site still hands the hint to the session it belongs to.
#[test]
fn cd_queues_through_the_session_keyed_helper() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tui/app/messaging.rs"
    ))
    .expect("read tui/app/messaging.rs");
    let start = src
        .find("pub(crate) async fn apply_working_directory_change")
        .expect("apply_working_directory_change exists");
    let end = src[start..]
        .find("\n    /// ")
        .map(|i| start + i)
        .unwrap_or(src.len());
    let body = &src[start..end];

    assert!(
        body.contains("push_pending_context_for("),
        "#2006: /cd must queue its notice through the per-session helper"
    );
    assert!(
        !body.contains("self.pending_context.push("),
        "#2006: the app-global Vec::push is the leak; the field is a map keyed by session now"
    );
}
