//! Session Service
//!
//! Provides business logic for session management operations.

use crate::db::{
    models::Session,
    repository::{SessionListOptions, SessionRepository},
};
use crate::services::ServiceContext;
use anyhow::{Context, Result};
use chrono::Utc;
use uuid::Uuid;

/// Service for managing sessions
#[derive(Clone)]
pub struct SessionService {
    context: ServiceContext,
}

impl SessionService {
    /// Create a new session service
    pub fn new(context: ServiceContext) -> Self {
        Self { context }
    }

    /// Create a new session
    pub async fn create_session(&self, title: Option<String>) -> Result<Session> {
        self.create_session_with_provider(title, None, None).await
    }

    /// Create a new session with explicit provider and model
    pub async fn create_session_with_provider(
        &self,
        title: Option<String>,
        provider_name: Option<String>,
        model: Option<String>,
    ) -> Result<Session> {
        let repo = SessionRepository::new(self.context.pool());

        let session = Session {
            id: Uuid::new_v4(),
            title,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            model,
            provider_name,
            token_count: 0,
            total_cost: 0.0,
        };

        repo.create(&session)
            .await
            .context("Failed to create session")?;

        tracing::info!("Created new session: {}", session.id);
        Ok(session)
    }

    /// Get a session by ID
    pub async fn get_session(&self, id: Uuid) -> Result<Option<Session>> {
        let repo = SessionRepository::new(self.context.pool());
        repo.find_by_id(id).await.context("Failed to get session")
    }

    /// Get a session by ID, returning an error if not found
    pub async fn get_session_required(&self, id: Uuid) -> Result<Session> {
        self.get_session(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", id))
    }

    /// List all sessions
    pub async fn list_sessions(&self, options: SessionListOptions) -> Result<Vec<Session>> {
        let repo = SessionRepository::new(self.context.pool());
        repo.list(options).await.context("Failed to list sessions")
    }

    /// Update a session
    pub async fn update_session(&self, session: &Session) -> Result<()> {
        let repo = SessionRepository::new(self.context.pool());

        // Update the updated_at timestamp
        let mut updated_session = session.clone();
        updated_session.updated_at = Utc::now();

        repo.update(&updated_session)
            .await
            .context("Failed to update session")?;

        tracing::debug!("Updated session: {}", session.id);
        Ok(())
    }

    /// Update session title
    pub async fn update_session_title(&self, id: Uuid, title: Option<String>) -> Result<()> {
        let mut session = self.get_session_required(id).await?;
        session.title = title;
        session.updated_at = Utc::now();

        let repo = SessionRepository::new(self.context.pool());
        repo.update(&session)
            .await
            .context("Failed to update session title")?;

        tracing::info!("Updated session title: {}", id);
        Ok(())
    }

    /// Update session usage statistics
    pub async fn update_session_usage(&self, id: Uuid, token_count: i32, cost: f64) -> Result<()> {
        let mut session = self.get_session_required(id).await?;
        session.token_count += token_count;
        session.total_cost += cost;
        session.updated_at = Utc::now();

        let repo = SessionRepository::new(self.context.pool());
        repo.update(&session)
            .await
            .context("Failed to update session usage")?;

        tracing::debug!(
            "Updated session usage: {} (+{} tokens, +${:.4})",
            id,
            token_count,
            cost
        );
        Ok(())
    }

    /// Archive a session
    pub async fn archive_session(&self, id: Uuid) -> Result<()> {
        let repo = SessionRepository::new(self.context.pool());
        repo.archive(id)
            .await
            .context("Failed to archive session")?;

        tracing::info!("Archived session: {}", id);
        Ok(())
    }

    /// Unarchive a session
    pub async fn unarchive_session(&self, id: Uuid) -> Result<()> {
        let repo = SessionRepository::new(self.context.pool());
        repo.unarchive(id)
            .await
            .context("Failed to unarchive session")?;

        tracing::info!("Unarchived session: {}", id);
        Ok(())
    }

    /// Delete a session permanently
    pub async fn delete_session(&self, id: Uuid) -> Result<()> {
        let repo = SessionRepository::new(self.context.pool());
        repo.delete(id).await.context("Failed to delete session")?;

        tracing::info!("Deleted session: {}", id);
        Ok(())
    }

    /// Get the most recent active session
    pub async fn get_most_recent_session(&self) -> Result<Option<Session>> {
        let repo = SessionRepository::new(self.context.pool());
        let options = SessionListOptions {
            include_archived: false,
            limit: Some(1),
            offset: 0,
        };

        let sessions = repo.list(options).await?;
        Ok(sessions.into_iter().next())
    }

    /// Count total sessions (excluding archived)
    pub async fn count_sessions(&self) -> Result<i64> {
        let repo = SessionRepository::new(self.context.pool());
        repo.count(false).await.context("Failed to count sessions")
    }

    /// Count archived sessions
    pub async fn count_archived_sessions(&self) -> Result<i64> {
        let repo = SessionRepository::new(self.context.pool());
        repo.count(true)
            .await
            .context("Failed to count archived sessions")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_service() -> SessionService {
        use crate::db::Database;

        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let pool = db.pool().clone();

        let context = ServiceContext::new(pool);
        SessionService::new(context)
    }

    #[tokio::test]
    async fn test_create_session() {
        let service = create_test_service().await;
        let session = service
            .create_session(Some("Test Session".to_string()))
            .await
            .unwrap();

        assert_eq!(session.title, Some("Test Session".to_string()));
        assert_eq!(session.token_count, 0);
        assert_eq!(session.total_cost, 0.0);
        assert!(session.archived_at.is_none());
    }

    #[tokio::test]
    async fn test_get_session() {
        let service = create_test_service().await;
        let created = service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        let found = service.get_session(created.id).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn test_get_session_required() {
        let service = create_test_service().await;
        let created = service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        let found = service.get_session_required(created.id).await.unwrap();
        assert_eq!(found.id, created.id);

        // Test non-existent session
        let result = service.get_session_required(Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_session_title() {
        let service = create_test_service().await;
        let session = service
            .create_session(Some("Original".to_string()))
            .await
            .unwrap();

        service
            .update_session_title(session.id, Some("Updated".to_string()))
            .await
            .unwrap();

        let updated = service.get_session_required(session.id).await.unwrap();
        assert_eq!(updated.title, Some("Updated".to_string()));
    }

    #[tokio::test]
    async fn test_update_session_usage() {
        let service = create_test_service().await;
        let session = service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        service
            .update_session_usage(session.id, 100, 0.05)
            .await
            .unwrap();
        service
            .update_session_usage(session.id, 50, 0.025)
            .await
            .unwrap();

        let updated = service.get_session_required(session.id).await.unwrap();
        assert_eq!(updated.token_count, 150);
        assert!((updated.total_cost - 0.075).abs() < 0.0001);
    }

    #[tokio::test]
    async fn test_archive_unarchive_session() {
        let service = create_test_service().await;
        let session = service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        // Archive
        service.archive_session(session.id).await.unwrap();
        let archived = service.get_session_required(session.id).await.unwrap();
        assert!(archived.archived_at.is_some());

        // Unarchive
        service.unarchive_session(session.id).await.unwrap();
        let unarchived = service.get_session_required(session.id).await.unwrap();
        assert!(unarchived.archived_at.is_none());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let service = create_test_service().await;
        let session = service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        service.delete_session(session.id).await.unwrap();

        let result = service.get_session(session.id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let service = create_test_service().await;

        // Create multiple sessions
        service
            .create_session(Some("Session 1".to_string()))
            .await
            .unwrap();
        service
            .create_session(Some("Session 2".to_string()))
            .await
            .unwrap();
        service
            .create_session(Some("Session 3".to_string()))
            .await
            .unwrap();

        let options = SessionListOptions {
            include_archived: false,
            limit: None,
            offset: 0,
        };

        let sessions = service.list_sessions(options).await.unwrap();
        assert_eq!(sessions.len(), 3);
    }

    #[tokio::test]
    async fn test_get_most_recent_session() {
        let service = create_test_service().await;

        let _session1 = service
            .create_session(Some("Session 1".to_string()))
            .await
            .unwrap();
        // Sleep for 1 second to ensure different Unix timestamps (which have second resolution)
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        let session2 = service
            .create_session(Some("Session 2".to_string()))
            .await
            .unwrap();

        let recent = service.get_most_recent_session().await.unwrap();
        assert!(recent.is_some());
        assert_eq!(recent.unwrap().id, session2.id);
    }

    #[tokio::test]
    async fn test_count_sessions() {
        let service = create_test_service().await;

        service
            .create_session(Some("Session 1".to_string()))
            .await
            .unwrap();
        let session2 = service
            .create_session(Some("Session 2".to_string()))
            .await
            .unwrap();
        service
            .create_session(Some("Session 3".to_string()))
            .await
            .unwrap();

        // Archive one session
        service.archive_session(session2.id).await.unwrap();

        let active_count = service.count_sessions().await.unwrap();
        let archived_count = service.count_archived_sessions().await.unwrap();

        assert_eq!(active_count, 2);
        assert_eq!(archived_count, 1);
    }
}
