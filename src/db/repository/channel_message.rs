//! Channel Message Repository
//!
//! Database operations for passively captured channel messages.

use crate::db::Pool;
use crate::db::database::interact_err;
use crate::db::models::ChannelMessage;
use anyhow::{Context, Result};
use rusqlite::params;

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
    pool: Pool,
}

impl ChannelMessageRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Insert a single channel message
    pub async fn insert(&self, msg: &ChannelMessage) -> Result<()> {
        let m = msg.clone();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO channel_messages
                        (id, channel, channel_chat_id, channel_chat_name,
                         sender_id, sender_name, content, message_type,
                         platform_message_id, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        m.id.to_string(),
                        m.channel,
                        m.channel_chat_id,
                        m.channel_chat_name,
                        m.sender_id,
                        m.sender_name,
                        m.content,
                        m.message_type,
                        m.platform_message_id,
                        m.created_at.timestamp(),
                    ],
                )
            })
            .await
            .map_err(interact_err)?
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
        let ch = channel.map(|s| s.to_string());
        let cid = chat_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                if let Some(ch) = ch {
                    let mut stmt = conn.prepare_cached(
                        "SELECT * FROM channel_messages \
                         WHERE channel = ?1 AND channel_chat_id = ?2 \
                         ORDER BY created_at DESC LIMIT ?3",
                    )?;
                    let rows = stmt.query_map(params![ch, cid, limit], ChannelMessage::from_row)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                } else {
                    let mut stmt = conn.prepare_cached(
                        "SELECT * FROM channel_messages \
                         WHERE channel_chat_id = ?1 \
                         ORDER BY created_at DESC LIMIT ?2",
                    )?;
                    let rows = stmt.query_map(params![cid, limit], ChannelMessage::from_row)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                }
            })
            .await
            .map_err(interact_err)?
            .context("Failed to fetch recent channel messages")
    }

    /// Search messages by content (LIKE-based)
    pub async fn search(
        &self,
        channel: Option<&str>,
        chat_id: Option<&str>,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ChannelMessage>> {
        let ch = channel.map(|s| s.to_string());
        let cid = chat_id.map(|s| s.to_string());
        let pattern = format!("%{query}%");

        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| match (ch, cid) {
                (Some(ch), Some(cid)) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT * FROM channel_messages \
                             WHERE channel = ?1 AND channel_chat_id = ?2 AND content LIKE ?3 \
                             ORDER BY created_at DESC LIMIT ?4",
                    )?;
                    let rows =
                        stmt.query_map(params![ch, cid, pattern, limit], ChannelMessage::from_row)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                }
                (Some(ch), None) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT * FROM channel_messages \
                             WHERE channel = ?1 AND content LIKE ?2 \
                             ORDER BY created_at DESC LIMIT ?3",
                    )?;
                    let rows =
                        stmt.query_map(params![ch, pattern, limit], ChannelMessage::from_row)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                }
                (None, Some(cid)) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT * FROM channel_messages \
                             WHERE channel_chat_id = ?1 AND content LIKE ?2 \
                             ORDER BY created_at DESC LIMIT ?3",
                    )?;
                    let rows =
                        stmt.query_map(params![cid, pattern, limit], ChannelMessage::from_row)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                }
                (None, None) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT * FROM channel_messages \
                             WHERE content LIKE ?1 \
                             ORDER BY created_at DESC LIMIT ?2",
                    )?;
                    let rows = stmt.query_map(params![pattern, limit], ChannelMessage::from_row)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                }
            })
            .await
            .map_err(interact_err)?
            .context("Failed to search channel messages")
    }

    /// List distinct chats with message count and last message time
    pub async fn list_chats(&self, channel: Option<&str>) -> Result<Vec<ChatSummary>> {
        let ch = channel.map(|s| s.to_string());
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                if let Some(ch) = ch {
                    let mut stmt = conn.prepare_cached(
                        "SELECT channel, channel_chat_id, \
                                MAX(channel_chat_name) as channel_chat_name, \
                                COUNT(*) as message_count, \
                                MAX(created_at) as last_message_at \
                         FROM channel_messages \
                         WHERE channel = ?1 \
                         GROUP BY channel, channel_chat_id \
                         ORDER BY last_message_at DESC",
                    )?;
                    let rows = stmt.query_map(params![ch], |row| {
                        Ok(ChatSummary {
                            channel: row.get(0)?,
                            channel_chat_id: row.get(1)?,
                            channel_chat_name: row.get(2)?,
                            message_count: row.get(3)?,
                            last_message_at: row.get(4)?,
                        })
                    })?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                } else {
                    let mut stmt = conn.prepare_cached(
                        "SELECT channel, channel_chat_id, \
                                MAX(channel_chat_name) as channel_chat_name, \
                                COUNT(*) as message_count, \
                                MAX(created_at) as last_message_at \
                         FROM channel_messages \
                         GROUP BY channel, channel_chat_id \
                         ORDER BY last_message_at DESC",
                    )?;
                    let rows = stmt.query_map([], |row| {
                        Ok(ChatSummary {
                            channel: row.get(0)?,
                            channel_chat_id: row.get(1)?,
                            channel_chat_name: row.get(2)?,
                            message_count: row.get(3)?,
                            last_message_at: row.get(4)?,
                        })
                    })?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                }
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list channel chats")
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
