//! Message Repository
//!
//! Database operations for messages.

use crate::db::Pool;
use crate::db::database::interact_err;
use crate::db::models::Message;
use anyhow::{Context, Result};
use rusqlite::params;
use uuid::Uuid;

/// Repository for message operations
#[derive(Clone)]
pub struct MessageRepository {
    pool: Pool,
}

impl MessageRepository {
    /// Create a new message repository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Find message by ID
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Message>> {
        let id_str = id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.prepare_cached("SELECT * FROM messages WHERE id = ?1")?
                    .query_row(params![id_str], Message::from_row)
                    .optional()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to find message")
    }

    /// Find all messages for a session
    pub async fn find_by_session(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let sid = session_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT * FROM messages WHERE session_id = ?1 ORDER BY sequence ASC",
                )?;
                let rows = stmt.query_map(params![sid], Message::from_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to find messages by session")
    }

    /// Create a new message
    pub async fn create(&self, message: &Message) -> Result<()> {
        let m = message.clone();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, sequence,
                                         created_at, token_count, cost)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        m.id.to_string(),
                        m.session_id.to_string(),
                        m.role,
                        m.content,
                        m.sequence,
                        m.created_at.timestamp(),
                        m.token_count,
                        m.cost,
                    ],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to create message")?;

        tracing::debug!(
            "Created message: {} in session: {}",
            message.id,
            message.session_id
        );
        Ok(())
    }

    /// Update an existing message
    pub async fn update(&self, message: &Message) -> Result<()> {
        let m = message.clone();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE messages
                     SET content = ?1, token_count = ?2, cost = ?3
                     WHERE id = ?4",
                    params![m.content, m.token_count, m.cost, m.id.to_string()],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to update message")?;

        tracing::debug!("Updated message: {}", message.id);
        Ok(())
    }

    /// Append content to an existing message (for real-time history persistence)
    pub async fn append_content(&self, id: Uuid, content_to_append: &str) -> Result<()> {
        let id_str = id.to_string();
        let content = content_to_append.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE messages SET content = content || ?1 WHERE id = ?2",
                    params![content, id_str],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to append to message")?;

        tracing::debug!("Appended content to message: {}", id);
        Ok(())
    }

    /// Delete a message
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute("DELETE FROM messages WHERE id = ?1", params![id_str])
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete message")?;

        tracing::debug!("Deleted message: {}", id);
        Ok(())
    }

    /// List all messages for a session
    pub async fn list_by_session(&self, session_id: Uuid) -> Result<Vec<Message>> {
        self.find_by_session(session_id).await
    }

    /// Count messages in a session
    pub async fn count_by_session(&self, session_id: Uuid) -> Result<i64> {
        let sid = session_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
                    params![sid],
                    |row| row.get(0),
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to count messages")
    }

    /// Get the last message in a session
    pub async fn get_last_message(&self, session_id: Uuid) -> Result<Option<Message>> {
        let sid = session_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.prepare_cached(
                    "SELECT * FROM messages WHERE session_id = ?1 ORDER BY sequence DESC LIMIT 1",
                )?
                .query_row(params![sid], Message::from_row)
                .optional()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to get last message")
    }

    /// Delete all messages in a session
    pub async fn delete_by_session(&self, session_id: Uuid) -> Result<()> {
        let sid = session_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute("DELETE FROM messages WHERE session_id = ?1", params![sid])
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete session messages")?;

        tracing::debug!("Deleted all messages for session: {}", session_id);
        Ok(())
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
    use crate::db::models::Session;
    use crate::db::repository::SessionRepository;

    #[tokio::test]
    async fn test_message_crud() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        // Create session first
        let session = Session::new(Some("Test".to_string()), Some("model".to_string()), None);
        session_repo
            .create(&session)
            .await
            .expect("Failed to create session");

        // Create message
        let message = Message::new(session.id, "user".to_string(), "Hello!".to_string(), 1);
        message_repo
            .create(&message)
            .await
            .expect("Failed to create message");

        // Read
        let found = message_repo
            .find_by_id(message.id)
            .await
            .expect("Failed to find");
        assert!(found.is_some());
        assert_eq!(found.unwrap().content, "Hello!");

        // Update
        let mut updated = message.clone();
        updated.content = "Updated content".to_string();
        message_repo
            .update(&updated)
            .await
            .expect("Failed to update");

        let found = message_repo
            .find_by_id(message.id)
            .await
            .expect("Failed to find");
        assert_eq!(found.unwrap().content, "Updated content");

        // Delete
        message_repo
            .delete(message.id)
            .await
            .expect("Failed to delete");
        let found = message_repo
            .find_by_id(message.id)
            .await
            .expect("Failed to find");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_message_list_by_session() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        let session = Session::new(Some("Test".to_string()), Some("model".to_string()), None);
        session_repo
            .create(&session)
            .await
            .expect("Failed to create session");

        // Create multiple messages
        for i in 0..3 {
            let msg = Message::new(
                session.id,
                "user".to_string(),
                format!("Message {}", i),
                i + 1,
            );
            message_repo
                .create(&msg)
                .await
                .expect("Failed to create message");
        }

        let messages = message_repo
            .list_by_session(session.id)
            .await
            .expect("Failed to list");
        assert_eq!(messages.len(), 3);

        let count = message_repo
            .count_by_session(session.id)
            .await
            .expect("Failed to count");
        assert_eq!(count, 3);
    }
}
