//! Session Repository
//!
//! Database operations for sessions.

use crate::db::models::Session;
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Options for listing sessions
#[derive(Debug, Clone, Default)]
pub struct SessionListOptions {
    /// Include archived sessions
    pub include_archived: bool,
    /// Maximum number of sessions to return
    pub limit: Option<usize>,
    /// Number of sessions to skip
    pub offset: usize,
}

/// Repository for session operations
#[derive(Clone)]
pub struct SessionRepository {
    pool: SqlitePool,
}

impl SessionRepository {
    /// Create a new session repository
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Find session by ID
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Session>> {
        let session = sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .context("Failed to find session")?;

        Ok(session)
    }

    /// Create a new session
    pub async fn create(&self, session: &Session) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO sessions (id, title, model, provider_name, created_at, updated_at,
                                 archived_at, token_count, total_cost)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(session.id.to_string())
        .bind(&session.title)
        .bind(&session.model)
        .bind(&session.provider_name)
        .bind(session.created_at.timestamp())
        .bind(session.updated_at.timestamp())
        .bind(session.archived_at.map(|dt| dt.timestamp()))
        .bind(session.token_count)
        .bind(session.total_cost)
        .execute(&self.pool)
        .await
        .context("Failed to create session")?;

        tracing::debug!("Created session: {}", session.id);
        Ok(())
    }

    /// Update an existing session
    pub async fn update(&self, session: &Session) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE sessions
            SET title = ?, model = ?, provider_name = ?, updated_at = ?,
                archived_at = ?, token_count = ?, total_cost = ?
            WHERE id = ?
            "#,
        )
        .bind(&session.title)
        .bind(&session.model)
        .bind(&session.provider_name)
        .bind(session.updated_at.timestamp())
        .bind(session.archived_at.map(|dt| dt.timestamp()))
        .bind(session.token_count)
        .bind(session.total_cost)
        .bind(session.id.to_string())
        .execute(&self.pool)
        .await
        .context("Failed to update session")?;

        tracing::debug!("Updated session: {}", session.id);
        Ok(())
    }

    /// Delete a session
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to delete session")?;

        tracing::debug!("Deleted session: {}", id);
        Ok(())
    }

    /// List all sessions (most recent first)
    pub async fn list(&self, options: SessionListOptions) -> Result<Vec<Session>> {
        // Use parameterized queries to prevent SQL injection
        let sessions = if let Some(limit) = options.limit {
            if options.include_archived {
                sqlx::query_as::<_, Session>(
                    "SELECT * FROM sessions ORDER BY updated_at DESC LIMIT ? OFFSET ?",
                )
                .bind(limit as i64)
                .bind(options.offset as i64)
                .fetch_all(&self.pool)
                .await
            } else {
                sqlx::query_as::<_, Session>(
                    "SELECT * FROM sessions WHERE archived_at IS NULL ORDER BY updated_at DESC LIMIT ? OFFSET ?",
                )
                .bind(limit as i64)
                .bind(options.offset as i64)
                .fetch_all(&self.pool)
                .await
            }
        } else if options.include_archived {
            sqlx::query_as::<_, Session>(
                "SELECT * FROM sessions ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, Session>(
                "SELECT * FROM sessions WHERE archived_at IS NULL ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool)
            .await
        }
        .context("Failed to list sessions")?;

        Ok(sessions)
    }

    /// List non-archived sessions
    pub async fn list_active(&self) -> Result<Vec<Session>> {
        let sessions = sqlx::query_as::<_, Session>(
            "SELECT * FROM sessions WHERE archived_at IS NULL ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed to list active sessions")?;

        Ok(sessions)
    }

    /// List archived sessions
    pub async fn list_archived(&self) -> Result<Vec<Session>> {
        let sessions = sqlx::query_as::<_, Session>(
            "SELECT * FROM sessions WHERE archived_at IS NOT NULL ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed to list archived sessions")?;

        Ok(sessions)
    }

    /// Archive a session
    pub async fn archive(&self, id: Uuid) -> Result<()> {
        let now = Utc::now();

        sqlx::query("UPDATE sessions SET archived_at = ?, updated_at = ? WHERE id = ?")
            .bind(now.timestamp())
            .bind(now.timestamp())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to archive session")?;

        tracing::debug!("Archived session: {}", id);
        Ok(())
    }

    /// Unarchive a session
    pub async fn unarchive(&self, id: Uuid) -> Result<()> {
        let now = Utc::now();

        sqlx::query("UPDATE sessions SET archived_at = NULL, updated_at = ? WHERE id = ?")
            .bind(now.timestamp())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to unarchive session")?;

        tracing::debug!("Unarchived session: {}", id);
        Ok(())
    }

    /// Update session statistics
    pub async fn update_stats(&self, id: Uuid, token_delta: i32, cost_delta: f64) -> Result<()> {
        let updated_at = Utc::now();

        sqlx::query(
            r#"
            UPDATE sessions
            SET token_count = token_count + ?,
                total_cost = total_cost + ?,
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(token_delta)
        .bind(cost_delta)
        .bind(updated_at.timestamp())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .context("Failed to update session stats")?;

        Ok(())
    }

    /// Count sessions
    pub async fn count(&self, archived_only: bool) -> Result<i64> {
        let query = if archived_only {
            "SELECT COUNT(*) as count FROM sessions WHERE archived_at IS NOT NULL"
        } else {
            "SELECT COUNT(*) as count FROM sessions WHERE archived_at IS NULL"
        };

        let result: (i64,) = sqlx::query_as(query)
            .fetch_one(&self.pool)
            .await
            .context("Failed to count sessions")?;

        Ok(result.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[tokio::test]
    async fn test_session_crud() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = SessionRepository::new(db.pool().clone());

        // Create
        let session = Session::new(
            Some("Test Session".to_string()),
            Some("claude-sonnet-4-5".to_string()),
            Some("anthropic".to_string()),
        );
        repo.create(&session)
            .await
            .expect("Failed to create session");

        // Read
        let found = repo
            .find_by_id(session.id)
            .await
            .expect("Failed to find session");
        assert!(found.is_some());
        assert_eq!(
            found.as_ref().unwrap().title,
            Some("Test Session".to_string())
        );

        // Update
        let mut updated_session = session.clone();
        updated_session.title = Some("Updated Title".to_string());
        repo.update(&updated_session)
            .await
            .expect("Failed to update session");

        let found = repo
            .find_by_id(session.id)
            .await
            .expect("Failed to find session");
        assert_eq!(found.unwrap().title, Some("Updated Title".to_string()));

        // Delete
        repo.delete(session.id)
            .await
            .expect("Failed to delete session");
        let found = repo
            .find_by_id(session.id)
            .await
            .expect("Failed to find session");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_session_archive() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = SessionRepository::new(db.pool().clone());

        let session = Session::new(Some("Test".to_string()), Some("model".to_string()), None);
        repo.create(&session)
            .await
            .expect("Failed to create session");

        // Archive
        repo.archive(session.id).await.expect("Failed to archive");
        let found = repo
            .find_by_id(session.id)
            .await
            .expect("Failed to find")
            .unwrap();
        assert!(found.is_archived());

        // Unarchive
        repo.unarchive(session.id)
            .await
            .expect("Failed to unarchive");
        let found = repo
            .find_by_id(session.id)
            .await
            .expect("Failed to find")
            .unwrap();
        assert!(!found.is_archived());
    }
}
