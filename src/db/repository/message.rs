//! Message Repository
//!
//! Database operations for messages.

use crate::db::models::Message;
use anyhow::{Context, Result};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Repository for message operations
#[derive(Clone)]
pub struct MessageRepository {
    pool: SqlitePool,
}

impl MessageRepository {
    /// Create a new message repository
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Find message by ID
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Message>> {
        let message = sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .context("Failed to find message")?;

        Ok(message)
    }

    /// Find all messages for a session
    pub async fn find_by_session(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let messages = sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE session_id = ? ORDER BY sequence ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await
        .context("Failed to find messages by session")?;

        Ok(messages)
    }

    /// Create a new message
    pub async fn create(&self, message: &Message) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO messages (id, session_id, role, content, sequence,
                                 created_at, token_count, cost)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(message.id.to_string())
        .bind(message.session_id.to_string())
        .bind(&message.role)
        .bind(&message.content)
        .bind(message.sequence)
        .bind(message.created_at.timestamp())
        .bind(message.token_count)
        .bind(message.cost)
        .execute(&self.pool)
        .await
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
        sqlx::query(
            r#"
            UPDATE messages
            SET content = ?, token_count = ?, cost = ?
            WHERE id = ?
            "#,
        )
        .bind(&message.content)
        .bind(message.token_count)
        .bind(message.cost)
        .bind(message.id.to_string())
        .execute(&self.pool)
        .await
        .context("Failed to update message")?;

        tracing::debug!("Updated message: {}", message.id);
        Ok(())
    }

    /// Append content to an existing message (for real-time history persistence)
    pub async fn append_content(&self, id: Uuid, content_to_append: &str) -> Result<()> {
        sqlx::query("UPDATE messages SET content = content || ? WHERE id = ?")
            .bind(content_to_append)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to append to message")?;

        tracing::debug!("Appended content to message: {}", id);
        Ok(())
    }

    /// Delete a message
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
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
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE session_id = ?")
            .bind(session_id.to_string())
            .fetch_one(&self.pool)
            .await
            .context("Failed to count messages")?;

        Ok(result.0)
    }

    /// Get the last message in a session
    pub async fn get_last_message(&self, session_id: Uuid) -> Result<Option<Message>> {
        let message = sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE session_id = ? ORDER BY sequence DESC LIMIT 1",
        )
        .bind(session_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .context("Failed to get last message")?;

        Ok(message)
    }

    /// Delete all messages in a session
    pub async fn delete_by_session(&self, session_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to delete session messages")?;

        tracing::debug!("Deleted all messages for session: {}", session_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::Session;
    use crate::db::repository::SessionRepository;
    use crate::db::Database;

    #[tokio::test]
    async fn test_message_crud() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let session_repo = SessionRepository::new(db.pool().clone());
        let message_repo = MessageRepository::new(db.pool().clone());

        // Create session first
        let session = Session::new(Some("Test".to_string()), Some("model".to_string()));
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

        let session = Session::new(Some("Test".to_string()), Some("model".to_string()));
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
