//! Channel Message Repository
//!
//! Database operations for passively captured channel messages.

use crate::db::models::ChannelMessage;
use anyhow::{Context, Result};
use sqlx::SqlitePool;

/// Summary of a known chat
pub struct ChatSummary {
    pub channel: String,
    pub channel_chat_id: String,
    pub channel_chat_name: Option<String>,
    pub message_count: i64,
    pub last_message_at: i64,
}

/// Repository for channel message operations
#[derive(Clone)]
pub struct ChannelMessageRepository {
    pool: SqlitePool,
}

impl ChannelMessageRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert a single channel message
    pub async fn insert(&self, msg: &ChannelMessage) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO channel_messages
                (id, channel, channel_chat_id, channel_chat_name,
                 sender_id, sender_name, content, message_type,
                 platform_message_id, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(msg.id.to_string())
        .bind(&msg.channel)
        .bind(&msg.channel_chat_id)
        .bind(&msg.channel_chat_name)
        .bind(&msg.sender_id)
        .bind(&msg.sender_name)
        .bind(&msg.content)
        .bind(&msg.message_type)
        .bind(&msg.platform_message_id)
        .bind(msg.created_at.timestamp())
        .execute(&self.pool)
        .await
        .context("Failed to insert channel message")?;

        Ok(())
    }

    /// Get recent messages for a specific chat
    pub async fn recent(
        &self,
        channel: Option<&str>,
        chat_id: &str,
        limit: i64,
    ) -> Result<Vec<ChannelMessage>> {
        let messages = if let Some(ch) = channel {
            sqlx::query_as::<_, ChannelMessage>(
                "SELECT * FROM channel_messages \
                 WHERE channel = ? AND channel_chat_id = ? \
                 ORDER BY created_at DESC LIMIT ?",
            )
            .bind(ch)
            .bind(chat_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, ChannelMessage>(
                "SELECT * FROM channel_messages \
                 WHERE channel_chat_id = ? \
                 ORDER BY created_at DESC LIMIT ?",
            )
            .bind(chat_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
        }
        .context("Failed to fetch recent channel messages")?;

        Ok(messages)
    }

    /// Search messages by content (LIKE-based)
    pub async fn search(
        &self,
        channel: Option<&str>,
        chat_id: Option<&str>,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ChannelMessage>> {
        let pattern = format!("%{query}%");
        let messages = match (channel, chat_id) {
            (Some(ch), Some(cid)) => {
                sqlx::query_as::<_, ChannelMessage>(
                    "SELECT * FROM channel_messages \
                     WHERE channel = ? AND channel_chat_id = ? AND content LIKE ? \
                     ORDER BY created_at DESC LIMIT ?",
                )
                .bind(ch)
                .bind(cid)
                .bind(&pattern)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (Some(ch), None) => {
                sqlx::query_as::<_, ChannelMessage>(
                    "SELECT * FROM channel_messages \
                     WHERE channel = ? AND content LIKE ? \
                     ORDER BY created_at DESC LIMIT ?",
                )
                .bind(ch)
                .bind(&pattern)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(cid)) => {
                sqlx::query_as::<_, ChannelMessage>(
                    "SELECT * FROM channel_messages \
                     WHERE channel_chat_id = ? AND content LIKE ? \
                     ORDER BY created_at DESC LIMIT ?",
                )
                .bind(cid)
                .bind(&pattern)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                sqlx::query_as::<_, ChannelMessage>(
                    "SELECT * FROM channel_messages \
                     WHERE content LIKE ? \
                     ORDER BY created_at DESC LIMIT ?",
                )
                .bind(&pattern)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
        }
        .context("Failed to search channel messages")?;

        Ok(messages)
    }

    /// List distinct chats with message count and last message time
    pub async fn list_chats(&self, channel: Option<&str>) -> Result<Vec<ChatSummary>> {
        let rows: Vec<(String, String, Option<String>, i64, i64)> = if let Some(ch) = channel {
            sqlx::query_as(
                "SELECT channel, channel_chat_id, \
                        MAX(channel_chat_name) as channel_chat_name, \
                        COUNT(*) as message_count, \
                        MAX(created_at) as last_message_at \
                 FROM channel_messages \
                 WHERE channel = ? \
                 GROUP BY channel, channel_chat_id \
                 ORDER BY last_message_at DESC",
            )
            .bind(ch)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as(
                "SELECT channel, channel_chat_id, \
                        MAX(channel_chat_name) as channel_chat_name, \
                        COUNT(*) as message_count, \
                        MAX(created_at) as last_message_at \
                 FROM channel_messages \
                 GROUP BY channel, channel_chat_id \
                 ORDER BY last_message_at DESC",
            )
            .fetch_all(&self.pool)
            .await
        }
        .context("Failed to list channel chats")?;

        Ok(rows
            .into_iter()
            .map(
                |(channel, channel_chat_id, channel_chat_name, message_count, last_message_at)| {
                    ChatSummary {
                        channel,
                        channel_chat_id,
                        channel_chat_name,
                        message_count,
                        last_message_at,
                    }
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::models::ChannelMessage;

    #[tokio::test]
    async fn test_channel_message_crud() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = ChannelMessageRepository::new(db.pool().clone());

        let msg = ChannelMessage::new(
            "telegram".into(),
            "-100123456".into(),
            Some("Test Group".into()),
            "42".into(),
            "Alice".into(),
            "Hello world".into(),
            "text".into(),
            Some("101".into()),
        );

        repo.insert(&msg).await.expect("Failed to insert");

        let recent = repo
            .recent(Some("telegram"), "-100123456", 10)
            .await
            .expect("Failed to fetch recent");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].content, "Hello world");

        let results = repo
            .search(Some("telegram"), Some("-100123456"), "Hello", 10)
            .await
            .expect("Failed to search");
        assert_eq!(results.len(), 1);

        let chats = repo
            .list_chats(Some("telegram"))
            .await
            .expect("Failed to list chats");
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].message_count, 1);
    }
}
