//! Session Binding Repository
//!
//! Persists which channel (and chat / forum topic) owns each session so a
//! restart can re-register delivery routes at channel-connect time instead
//! of waiting for the next inbound message (#1224). A session idle at boot
//! has no way to reach ingress: without this record every background-task
//! completion and sub-agent result parks until a human pokes the topic.

use crate::db::{Pool, database::interact_err};
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

/// One persisted binding: "session S lives on channel C, chat X, topic T".
#[derive(Debug, Clone)]
pub struct SessionBinding {
    pub session_id: String,
    pub channel: String,
    pub chat_id: String,
    pub thread_id: Option<i32>,
}

/// Reads and writes [`SessionBinding`] rows.
#[derive(Clone)]
pub struct SessionBindingRepository {
    pool: Pool,
}

impl SessionBindingRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Record or refresh where a session lives. Idempotent per session:
    /// re-binding replaces the previous channel/chat/thread, which is what a
    /// session moving between surfaces needs.
    pub async fn upsert(
        &self,
        session_id: String,
        channel: &str,
        chat_id: &str,
        thread_id: Option<i32>,
    ) -> Result<()> {
        let ch = channel.to_string();
        let cid = chat_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO session_bindings (session_id, channel, chat_id, thread_id) \
                     VALUES (?1, ?2, ?3, ?4) \
                     ON CONFLICT(session_id) DO UPDATE SET \
                       channel = excluded.channel, \
                       chat_id = excluded.chat_id, \
                       thread_id = excluded.thread_id, \
                       updated_at = strftime('%s', 'now')",
                    params![session_id, ch, cid, thread_id],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to upsert session binding")?;
        Ok(())
    }

    /// Every binding recorded for one channel, least-recently-changed first.
    /// INNER JOIN against sessions drops bindings whose session was deleted,
    /// so connect-time re-registration never revives dead routes (#1224).
    /// Called once per channel connect to re-register routes (#1224).
    pub async fn all_for_channel(&self, channel: &str) -> Result<Vec<SessionBinding>> {
        let ch = channel.to_string();
        let mapped = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT b.session_id, b.channel, b.chat_id, b.thread_id \
                     FROM session_bindings b \
                     JOIN sessions s ON s.id = b.session_id \
                     WHERE b.channel = ?1 \
                     ORDER BY b.updated_at ASC",
                )?
                .query_map(params![ch], |row| {
                    Ok(SessionBinding {
                        session_id: row.get("session_id")?,
                        channel: row.get("channel")?,
                        chat_id: row.get("chat_id")?,
                        thread_id: row.get("thread_id")?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list session bindings")?;
        Ok(mapped)
    }

    /// Bindings for one channel whose `updated_at` is at least `since_epoch`
    /// (unix seconds), least-recently-changed first.
    ///
    /// The boot-time wake pass (#1227) uses this to find sessions that were
    /// active around a restart, so it can ping their topics that the platform
    /// survived — without waking every historical session. The INNER JOIN
    /// against `sessions` drops bindings whose session was deleted, matching
    /// [`Self::all_for_channel`] (#1224).
    pub async fn recent_for_channel(
        &self,
        channel: &str,
        since_epoch: i64,
    ) -> Result<Vec<SessionBinding>> {
        let ch = channel.to_string();
        let mapped = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT b.session_id, b.channel, b.chat_id, b.thread_id \
                     FROM session_bindings b \
                     JOIN sessions s ON s.id = b.session_id \
                     WHERE b.channel = ?1 AND b.updated_at >= ?2 \
                     ORDER BY b.updated_at ASC",
                )?
                .query_map(params![ch, since_epoch], |row| {
                    Ok(SessionBinding {
                        session_id: row.get("session_id")?,
                        channel: row.get("channel")?,
                        chat_id: row.get("chat_id")?,
                        thread_id: row.get("thread_id")?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list recent session bindings")?;
        Ok(mapped)
    }

    /// The binding recorded for ONE session, or `None` when that session has no
    /// binding row (or its `sessions` row is gone).
    ///
    /// The push-target resolver reads a session's OWN binding through this
    /// accessor: a push has to reach the session's binding rather than the
    /// topic that spoke last. The INNER JOIN against `sessions` matches
    /// [`Self::all_for_channel`] (#1224), so a binding whose session was
    /// deleted never revives a dead route. `session_id` is the table's primary
    /// key, so at most one row can match.
    pub async fn by_session(&self, session_id: &str) -> Result<Option<SessionBinding>> {
        let sid = session_id.to_string();
        let mapped = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT b.session_id, b.channel, b.chat_id, b.thread_id \
                     FROM session_bindings b \
                     JOIN sessions s ON s.id = b.session_id \
                     WHERE b.session_id = ?1",
                )?
                .query_row(params![sid], |row| {
                    Ok(SessionBinding {
                        session_id: row.get("session_id")?,
                        channel: row.get("channel")?,
                        chat_id: row.get("chat_id")?,
                        thread_id: row.get("thread_id")?,
                    })
                })
                .optional()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to read session binding")?;
        Ok(mapped)
    }
}
