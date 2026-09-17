//! Notify Queue Repository (#111)
//!
//! Durable parking for pushes that could not be delivered: session_notify
//! announcements and background-task completions park in memory when a
//! session has no live route (or is mid-turn and refuses an in-flight
//! injection). Memory does not survive a restart — and the sender has
//! already been told "queued" — so the push would silently vanish.
//!
//! A row exists only while the push is still undelivered. It is recorded
//! when a push parks, consumed the moment delivery succeeds, and re-offered
//! at boot ([`crate::brain::agent::service::restart_recovery`]) so a push
//! parked at kill time still reaches its session on the next start.

use crate::brain::agent::{BgTaskMeta, PushOrigin};
use crate::db::Pool;
use crate::db::database::interact_err;
use anyhow::{Context, Result};
use rusqlite::params;
use uuid::Uuid;

/// A push that has not reached its session yet.
#[derive(Debug, Clone)]
pub struct NotifyQueueRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub context_text: String,
    pub display_text: String,
    pub origin: PushOrigin,
    pub bg_meta: Option<BgTaskMeta>,
    pub created_at: i64,
}

/// Persisted form of [`PushOrigin`]. Unknown or legacy values map to
/// [`PushOrigin::Other`] (the safe default — never renders an echo bubble).
fn origin_as_db_str(origin: PushOrigin) -> &'static str {
    match origin {
        PushOrigin::BackgroundTask => "background_task",
        PushOrigin::SessionNotify => "session_notify",
        PushOrigin::SubAgent => "sub_agent",
        PushOrigin::Recovery => "recovery",
        PushOrigin::Ingress => "ingress",
        PushOrigin::Other => "other",
    }
}

fn origin_from_db_str(value: &str) -> PushOrigin {
    match value {
        "background_task" => PushOrigin::BackgroundTask,
        "session_notify" => PushOrigin::SessionNotify,
        "sub_agent" => PushOrigin::SubAgent,
        "recovery" => PushOrigin::Recovery,
        "ingress" => PushOrigin::Ingress,
        _ => PushOrigin::Other,
    }
}

#[derive(Clone)]
pub struct NotifyQueueRepository {
    pool: Pool,
}

impl NotifyQueueRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Persist an undelivered push so a restart cannot lose it.
    pub async fn record(
        &self,
        id: Uuid,
        session_id: Uuid,
        context_text: &str,
        display_text: &str,
        origin: PushOrigin,
        bg_meta: Option<&BgTaskMeta>,
    ) -> Result<()> {
        let (id, session_id) = (id.to_string(), session_id.to_string());
        let (context_text, display_text) = (context_text.to_string(), display_text.to_string());
        let origin = origin_as_db_str(origin).to_string();
        let bg_meta = bg_meta
            .map(|m| serde_json::to_string(m).context("encode notify bg_meta"))
            .transpose()?;
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO notify_queue \
                     (id, session_id, context_text, display_text, origin, bg_meta, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%s','now'))",
                    params![id, session_id, context_text, display_text, origin, bg_meta],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to record notify queue row")?;
        Ok(())
    }

    /// Every surviving row, oldest first.
    pub async fn all(&self) -> Result<Vec<NotifyQueueRow>> {
        let rows = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, context_text, display_text, origin, bg_meta, created_at \
                     FROM notify_queue ORDER BY created_at ASC",
                )?;
                let mapped = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, i64>(6)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok::<_, rusqlite::Error>(mapped)
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list notify queue rows")?;

        Ok(rows
            .into_iter()
            .filter_map(
                |(id, session_id, context_text, display_text, origin, bg_meta, created_at)| {
                    // A row whose ids no longer parse, or whose origin /
                    // bg_meta JSON is corrupt, is skipped rather than fatal —
                    // one unusable record must not fail boot redelivery.
                    let bg_meta = bg_meta.and_then(|json| match serde_json::from_str(&json) {
                        Ok(m) => Some(m),
                        Err(e) => {
                            tracing::warn!("notify_queue: skipping corrupt bg_meta: {}", e);
                            None
                        }
                    });
                    Some(NotifyQueueRow {
                        id: Uuid::parse_str(&id).ok()?,
                        session_id: Uuid::parse_str(&session_id).ok()?,
                        context_text,
                        display_text,
                        origin: origin_from_db_str(&origin),
                        bg_meta,
                        created_at,
                    })
                },
            )
            .collect())
    }

    /// Drop the row for a push that has been delivered.
    pub async fn clear(&self, id: Uuid) -> Result<()> {
        let id = id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute("DELETE FROM notify_queue WHERE id = ?1", params![id])
            })
            .await
            .map_err(interact_err)?
            .context("Failed to clear notify queue row")?;
        Ok(())
    }

    /// Drop every undelivered push for a session.
    ///
    /// Called when that session's route claims (or a flush delivers) the
    /// in-memory copies: the durable copies are redundant from that moment.
    pub async fn clear_for_session(&self, session_id: Uuid) -> Result<()> {
        let session_id = session_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM notify_queue WHERE session_id = ?1",
                    params![session_id],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to clear notify queue for session")?;
        Ok(())
    }

    /// Drop the rows matching one message's content fingerprint.
    ///
    /// Used when the in-memory twin of a persisted row was just delivered:
    /// an exact-content match deletes only that push, leaving any OTHER
    /// undelivered pushes for the same session intact (a blanket
    /// clear-for-session here could eat a second push that has not been
    /// delivered yet). Failure direction is a stale row surviving the next
    /// boot redelivery — a duplicate, never a lost message (#111).
    pub async fn clear_matching(
        &self,
        session_id: Uuid,
        context_text: &str,
        display_text: &str,
    ) -> Result<()> {
        let (session_id, context_text, display_text) = (
            session_id.to_string(),
            context_text.to_string(),
            display_text.to_string(),
        );
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM notify_queue \
                     WHERE session_id = ?1 AND context_text = ?2 AND display_text = ?3",
                    params![session_id, context_text, display_text],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to clear matching notify queue rows")?;
        Ok(())
    }

    /// Reap rows whose owning session no longer exists (#111 follow-up, Part B).
    ///
    /// A row for a session that is gone can never be claimed: no channel will
    /// ever register a route for it, so no consume site will ever clear it.
    /// Left alone it survives every boot forever, and boot redelivery keeps
    /// re-offering it to a target that cannot exist. Returns how many rows
    /// were dropped.
    ///
    /// Failure direction matches every other clear here: the push was already
    /// undeliverable (its session is gone), so dropping it loses nothing a
    /// route could have carried.
    pub async fn clear_dead_sessions(&self) -> Result<usize> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM notify_queue \
                     WHERE session_id NOT IN (SELECT id FROM sessions)",
                    [],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to reap notify-queue rows for dead sessions")
    }

    /// Reap undelivered rows older than `cutoff_created_at` in an atomic transaction (#182).
    ///
    /// Sessions that still exist in `sessions` but have no active channel route
    /// (e.g. headless/cron sessions, archived worker topics, surfaceless sessions)
    /// park pushes forever. After a ceiling window (e.g. 72h), this reaps them
    /// so they do not leak table space or re-park into memory on every boot.
    /// Returns the deleted rows so caller can log/audit what was dropped.
    pub async fn reap_stale_unclaimed(
        &self,
        cutoff_created_at: i64,
    ) -> Result<Vec<NotifyQueueRow>> {
        let raw_rows = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let tx = conn.transaction()?;
                let mapped = {
                    let mut stmt = tx.prepare(
                        "SELECT id, session_id, context_text, display_text, origin, bg_meta, created_at \
                         FROM notify_queue WHERE created_at < ?1 ORDER BY created_at ASC",
                    )?;
                    stmt.query_map(params![cutoff_created_at], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, i64>(6)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?
                };
                tx.execute(
                    "DELETE FROM notify_queue WHERE created_at < ?1",
                    params![cutoff_created_at],
                )?;
                tx.commit()?;
                Ok::<_, rusqlite::Error>(mapped)
            })
            .await
            .map_err(interact_err)?
            .context("Failed to reap stale unclaimed notify-queue rows")?;

        Ok(raw_rows
            .into_iter()
            .filter_map(
                |(id, session_id, context_text, display_text, origin, bg_meta, created_at)| {
                    let bg_meta = bg_meta.and_then(|json| match serde_json::from_str(&json) {
                        Ok(m) => Some(m),
                        Err(e) => {
                            tracing::warn!(
                                "notify_queue: skipping corrupt bg_meta in reaped row: {}",
                                e
                            );
                            None
                        }
                    });
                    // The row is already DELETED by the time we get here, so a
                    // parse failure would erase it from the audit log too — the
                    // one place in this module where a push is genuinely lost.
                    // The bg_meta branch above already logs; these must too.
                    let parsed_id = match Uuid::parse_str(&id) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(
                                "notify_queue: reaped row with unparseable id {id:?}, \
                                 dropped without audit detail: {e}"
                            );
                            return None;
                        }
                    };
                    let parsed_session = match Uuid::parse_str(&session_id) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(
                                "notify_queue: reaped row {parsed_id} carries an \
                                 unparseable session_id {session_id:?}: {e}"
                            );
                            return None;
                        }
                    };
                    Some(NotifyQueueRow {
                        id: parsed_id,
                        session_id: parsed_session,
                        context_text,
                        display_text,
                        origin: origin_from_db_str(&origin),
                        bg_meta,
                        created_at,
                    })
                },
            )
            .collect())
    }
}
