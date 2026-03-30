//! Pending Request Repository
//!
//! Tracks in-flight agent requests so they can be replayed after a restart.
//! Rows only exist while a request is PROCESSING — they are deleted on
//! completion (success or failure). Any rows left in the table on startup
//! indicate the process crashed mid-request and should be replayed.

use crate::db::Pool;
use crate::db::database::interact_err;
use anyhow::{Context, Result};
use rusqlite::params;
use uuid::Uuid;

/// A pending request row
#[derive(Debug, Clone)]
pub struct PendingRequest {
    pub id: String,
    pub session_id: String,
    pub user_message: String,
    pub channel: String,
    pub channel_chat_id: Option<String>,
}

/// Repository for pending request operations
#[derive(Clone)]
pub struct PendingRequestRepository {
    pool: Pool,
}

impl PendingRequestRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Insert a new in-flight request
    pub async fn insert(
        &self,
        id: Uuid,
        session_id: Uuid,
        user_message: &str,
        channel: &str,
        channel_chat_id: Option<&str>,
    ) -> Result<()> {
        let id_s = id.to_string();
        let sid = session_id.to_string();
        let msg = user_message.to_string();
        let ch = channel.to_string();
        let cid = channel_chat_id.map(|s| s.to_string());
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO pending_requests (id, session_id, user_message, channel, channel_chat_id, status) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'PROCESSING')",
                    params![id_s, sid, msg, ch, cid],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to insert pending request")?;
        Ok(())
    }

    /// Delete a request (called when it finishes, regardless of outcome)
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let id_s = id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute("DELETE FROM pending_requests WHERE id = ?1", params![id_s])
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete pending request")?;
        Ok(())
    }

    /// Get all surviving rows (process crashed while these were in-flight)
    pub async fn get_interrupted(&self) -> Result<Vec<PendingRequest>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, user_message, channel, channel_chat_id \
                     FROM pending_requests ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(PendingRequest {
                        id: row.get("id")?,
                        session_id: row.get("session_id")?,
                        user_message: row.get("user_message")?,
                        channel: row.get("channel")?,
                        channel_chat_id: row.get("channel_chat_id")?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to get interrupted requests")
    }

    /// Get interrupted requests for a specific channel
    pub async fn get_interrupted_for_channel(&self, channel: &str) -> Result<Vec<PendingRequest>> {
        let ch = channel.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, user_message, channel, channel_chat_id \
                     FROM pending_requests WHERE channel = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![ch], |row| {
                    Ok(PendingRequest {
                        id: row.get("id")?,
                        session_id: row.get("session_id")?,
                        user_message: row.get("user_message")?,
                        channel: row.get("channel")?,
                        channel_chat_id: row.get("channel_chat_id")?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to get interrupted requests for channel")
    }

    /// Delete specific requests by ID
    pub async fn delete_ids(&self, ids: Vec<String>) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                for id in &ids {
                    conn.execute("DELETE FROM pending_requests WHERE id = ?1", params![id])?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete pending requests")?;
        Ok(())
    }

    /// Delete all rows (called on startup after reading interrupted requests)
    pub async fn clear_all(&self) -> Result<()> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| conn.execute("DELETE FROM pending_requests", []))
            .await
            .map_err(interact_err)?
            .context("Failed to clear pending requests")?;
        Ok(())
    }
}
