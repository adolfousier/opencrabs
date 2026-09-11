//! Regression tests for #1464: display surfaces read the GLOBAL provider even
//! when the loaded session ran a per-session provider swap, so the
//! "Session loaded" log (and the help / debug screens) reported a provider
//! that was neither the one stored on the session nor the one just created
//! for it.
//!
//! The App-level helpers `provider_{name,model}_for_current_session` resolve
//! the current session through `provider_{name,model}_for_session`
//! (per-session override -> sticky sub-provider -> global default). These
//! tests pin the divergence: the session-aware helpers must report the
//! session's provider, and the global getters must keep reporting the
//! global.

use std::sync::Arc;

use crate::brain::agent::service::AgentService;
use crate::brain::provider::Provider;
use crate::db::{Database, Session};
use crate::services::ServiceContext;
use crate::tests::agent_service_mocks::{MockProvider, MockProviderWithModel};
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

/// The #1464 divergence: a session served by a swapped per-session provider
/// must be reported through the session-aware helpers, not the global
/// default. Before the fix, every display surface read the right-hand side
/// of this test only.
#[tokio::test]
async fn swapped_session_reports_session_provider_not_global() {
    let mut app = app().await;
    let session = Session::new(Some("Swapped".to_string()), None, None);
    let sid = session.id;
    let swapped: Arc<dyn Provider> =
        Arc::new(MockProviderWithModel::new("claude-cli", "claude-opus"));
    app.agent_service
        .swap_provider_for_session(sid, swapped, "claude-opus".to_string());
    app.current_session = Some(session);

    assert_eq!(app.provider_name_for_current_session(), "claude-cli");
    assert_eq!(app.provider_model_for_current_session(), "claude-opus");

    // The global provider is untouched by a per-session swap: this is the
    // divergence the display surfaces used to paper over by reading only
    // the global side.
    assert_eq!(app.provider_name(), "mock");
    assert_eq!(app.provider_model(), "mock-model");
}

/// With no session loaded the helpers fall back to the global getters, so
/// pre-session surfaces (the "App created" log, the empty-state footer)
/// keep their existing honest value.
#[tokio::test]
async fn no_current_session_falls_back_to_global() {
    let app = app().await;
    assert!(app.current_session.is_none());
    assert_eq!(app.provider_name_for_current_session(), "mock");
    assert_eq!(app.provider_model_for_current_session(), "mock-model");
}

/// A loaded session WITHOUT a per-session override resolves to the global
/// default through the helpers — they must not invent a divergence where
/// none exists.
#[tokio::test]
async fn session_without_override_resolves_global_default() {
    let mut app = app().await;
    app.current_session = Some(Session::new(Some("Plain".to_string()), None, None));

    assert_eq!(app.provider_name_for_current_session(), "mock");
    assert_eq!(app.provider_model_for_current_session(), "mock-model");
}
