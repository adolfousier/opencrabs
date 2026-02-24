//! Shared test helpers for A2A module tests.

#[cfg(test)]
pub mod helpers {
    use crate::brain::agent::service::AgentService;
    use crate::brain::provider::PlaceholderProvider;
    use crate::db::Database;
    use crate::services::ServiceContext;
    use std::sync::Arc;

    /// Create a placeholder `AgentService` for tests (no real LLM).
    pub async fn placeholder_agent_service() -> Arc<AgentService> {
        let provider = Arc::new(PlaceholderProvider);
        let ctx = placeholder_service_context().await;
        Arc::new(AgentService::new(provider, ctx))
    }

    /// Create a `ServiceContext` backed by an in-memory SQLite database.
    pub async fn placeholder_service_context() -> ServiceContext {
        let db = Database::connect_in_memory().await.expect("in-memory db");
        db.run_migrations().await.expect("migrations");
        ServiceContext::new(db.pool().clone())
    }
}
