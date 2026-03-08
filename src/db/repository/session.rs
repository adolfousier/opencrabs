//! Session Repository
//!
//! Database operations for sessions.

use crate::db::Pool;
use crate::db::database::interact_err;
use crate::db::models::Session;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::params;
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
    pool: Pool,
}

impl SessionRepository {
    /// Create a new session repository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Find session by ID
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Session>> {
        let id_str = id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.prepare_cached("SELECT * FROM sessions WHERE id = ?1")?
                    .query_row(params![id_str], Session::from_row)
                    .optional()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to find session")
    }

    /// Create a new session
    pub async fn create(&self, session: &Session) -> Result<()> {
        let s = session.clone();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO sessions (id, title, model, provider_name, created_at, updated_at,
                                          archived_at, token_count, total_cost, working_directory)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        s.id.to_string(),
                        s.title,
                        s.model,
                        s.provider_name,
                        s.created_at.timestamp(),
                        s.updated_at.timestamp(),
                        s.archived_at.map(|dt| dt.timestamp()),
                        s.token_count,
                        s.total_cost,
                        s.working_directory,
                    ],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to create session")?;

        tracing::debug!("Created session: {}", session.id);
        Ok(())
    }

    /// Update an existing session
    pub async fn update(&self, session: &Session) -> Result<()> {
        let s = session.clone();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE sessions
                     SET title = ?1, model = ?2, provider_name = ?3, updated_at = ?4,
                         archived_at = ?5, token_count = ?6, total_cost = ?7, working_directory = ?8
                     WHERE id = ?9",
                    params![
                        s.title,
                        s.model,
                        s.provider_name,
                        s.updated_at.timestamp(),
                        s.archived_at.map(|dt| dt.timestamp()),
                        s.token_count,
                        s.total_cost,
                        s.working_directory,
                        s.id.to_string(),
                    ],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to update session")?;

        tracing::debug!("Updated session: {}", session.id);
        Ok(())
    }

    /// Delete a session
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute("DELETE FROM sessions WHERE id = ?1", params![id_str])
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete session")?;

        tracing::debug!("Deleted session: {}", id);
        Ok(())
    }

    /// List all sessions (most recent first)
    pub async fn list(&self, options: SessionListOptions) -> Result<Vec<Session>> {
        let include_archived = options.include_archived;
        let limit = options.limit;
        let offset = options.offset;

        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) =
                    match (include_archived, limit) {
                        (true, Some(lim)) => (
                            "SELECT * FROM sessions ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2",
                            vec![Box::new(lim as i64), Box::new(offset as i64)],
                        ),
                        (false, Some(lim)) => (
                            "SELECT * FROM sessions WHERE archived_at IS NULL ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2",
                            vec![Box::new(lim as i64), Box::new(offset as i64)],
                        ),
                        (true, None) => (
                            "SELECT * FROM sessions ORDER BY updated_at DESC",
                            vec![],
                        ),
                        (false, None) => (
                            "SELECT * FROM sessions WHERE archived_at IS NULL ORDER BY updated_at DESC",
                            vec![],
                        ),
                    };

                let mut stmt = conn.prepare_cached(sql)?;
                let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params_vec.iter().map(|p| p.as_ref()).collect();
                let rows = stmt.query_map(params_refs.as_slice(), Session::from_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list sessions")
    }

    /// List non-archived sessions
    pub async fn list_active(&self) -> Result<Vec<Session>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT * FROM sessions WHERE archived_at IS NULL ORDER BY updated_at DESC",
                )?;
                let rows = stmt.query_map([], Session::from_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list active sessions")
    }

    /// List archived sessions
    pub async fn list_archived(&self) -> Result<Vec<Session>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT * FROM sessions WHERE archived_at IS NOT NULL ORDER BY updated_at DESC",
                )?;
                let rows = stmt.query_map([], Session::from_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list archived sessions")
    }

    /// Archive a session
    pub async fn archive(&self, id: Uuid) -> Result<()> {
        let now = Utc::now();
        let id_str = id.to_string();

        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE sessions SET archived_at = ?1, updated_at = ?2 WHERE id = ?3",
                    params![now.timestamp(), now.timestamp(), id_str],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to archive session")?;

        tracing::debug!("Archived session: {}", id);
        Ok(())
    }

    /// Unarchive a session
    pub async fn unarchive(&self, id: Uuid) -> Result<()> {
        let now = Utc::now();
        let id_str = id.to_string();

        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE sessions SET archived_at = NULL, updated_at = ?1 WHERE id = ?2",
                    params![now.timestamp(), id_str],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to unarchive session")?;

        tracing::debug!("Unarchived session: {}", id);
        Ok(())
    }

    /// Update session statistics
    pub async fn update_stats(&self, id: Uuid, token_delta: i32, cost_delta: f64) -> Result<()> {
        let updated_at = Utc::now();
        let id_str = id.to_string();

        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE sessions
                     SET token_count = token_count + ?1,
                         total_cost = total_cost + ?2,
                         updated_at = ?3
                     WHERE id = ?4",
                    params![token_delta, cost_delta, updated_at.timestamp(), id_str],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to update session stats")?;

        Ok(())
    }

    /// Count sessions
    pub async fn count(&self, archived_only: bool) -> Result<i64> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let sql = if archived_only {
                    "SELECT COUNT(*) FROM sessions WHERE archived_at IS NOT NULL"
                } else {
                    "SELECT COUNT(*) FROM sessions WHERE archived_at IS NULL"
                };
                conn.query_row(sql, [], |row| row.get(0))
            })
            .await
            .map_err(interact_err)?
            .context("Failed to count sessions")
    }
}

/// Extension trait for rusqlite to add `.optional()` to query results
trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
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
