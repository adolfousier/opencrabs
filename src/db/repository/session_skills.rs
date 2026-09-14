//! Session Seen-Skills Repository (issue #138)
//!
//! Durable backing store for the in-memory `seen_skills` registry
//! (`src/brain/tools/seen_skills.rs`). The registry is the hot path — every
//! `mark_seen` writes a row here, and daemon boot hydrates the registry from
//! this table so the post-compaction skill inventory stamp (#125/#131)
//! survives restarts and rebuilds.
//!
//! Failure mode is deliberately soft (acceptance 5, #138): callers treat any
//! DB error as a WARN + continue — the in-memory registry keeps functioning,
//! a stamp just loses restart durability. No panic paths.

use crate::db::Pool;
use crate::db::database::interact_err;
use anyhow::{Context, Result};
use rusqlite::params;
use uuid::Uuid;

#[derive(Clone)]
pub struct SessionSkillsRepository {
    pool: Pool,
}

impl SessionSkillsRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Record that `session_id` consumed skill `slug` at compaction `epoch`.
    /// Upsert — the (session_id, slug) pair is the primary key, so repeats
    /// refresh `seen_at` AND `epoch` (issue #150: the skill glob gate compares
    /// the stored epoch against the session's current epoch, so a stale row
    /// must always be updatable to the new epoch). Idempotent with the
    /// in-memory registry's semantics.
    pub async fn record(&self, session_id: Uuid, slug: &str, epoch: u64) -> Result<()> {
        let sid = session_id.to_string();
        let slug = slug.to_string();
        let epoch = epoch as i64;
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO session_seen_skills (session_id, slug, epoch) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(session_id, slug) DO UPDATE \
                     SET seen_at = strftime('%s', 'now'), epoch = excluded.epoch",
                    params![sid, slug, epoch],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to record seen skill")?;
        Ok(())
    }

    /// All (session_id, slug, epoch) rows — the boot-hydrate feed. Bounded by
    /// the stamp's own cleanup: rows for sessions deleted by normal session
    /// pruning are removed by [`Self::prune_missing_sessions`].
    pub async fn all(&self) -> Result<Vec<(Uuid, String, Option<i64>)>> {
        let rows = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT session_id, slug, epoch FROM session_seen_skills")?;
                let mapped = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                    ))
                })?;
                // Pin the error parameter explicitly: `anyhow::Result` is in
                // scope and rusqlite::Error has several `From` impls
                // (rusqlite_migration, HookError), so a bare `Ok(vec)` leaves
                // E uninferable (E0282/E0283).
                mapped.collect::<rusqlite::Result<Vec<(String, String, Option<i64>)>>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to read seen skills")?;
        Ok(rows
            .into_iter()
            .filter_map(|(sid, slug, epoch)| Uuid::parse_str(&sid).ok().map(|id| (id, slug, epoch)))
            .collect())
    }

    /// Drop rows whose session no longer exists (normal session pruning).
    /// Called at boot alongside the hydrate; a failure is soft (WARN at the
    /// call site) — pruning is hygiene, not correctness.
    pub async fn prune_missing_sessions(&self) -> Result<u64> {
        let n = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM session_seen_skills \
                     WHERE session_id NOT IN (SELECT id FROM sessions)",
                    [],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to prune seen skills")?;
        Ok(n as u64)
    }
}
