//! Decision Stats Repository (#1648, PR2)
//!
//! Per-tier counters for the L1 decision-reuse ring. These are the
//! release-day evidence: `calls` and `would_hit` accumulate under
//! mode=shadow (zero behavior change), `live_hit` only under mode=live.
//! The keep-or-cut evaluation reads this table; if it is empty on
//! release day, the feature is removed, not extended (Adolfo directive
//! 2026-09-21).
//!
//! Failure direction: a lost bump under-counts evidence and can only
//! push the decision toward removal, which is the safe side of the
//! error. Nothing in the agent loop reads this table to make a live
//! decision, so an error here must never block a decision call —
//! callers should log-and-continue.
//!
//! Removal path: `DROP TABLE IF EXISTS decision_stats`.

use crate::db::Pool;
use crate::db::database::interact_err;
use anyhow::{Context, Result};
use rusqlite::params;

/// One tier's counters, as stored.
#[derive(Debug, Clone)]
pub struct DecisionStats {
    pub tier_id: String,
    pub calls: i64,
    pub would_hit: i64,
    pub live_hit: i64,
    pub last_seen_at: i64,
}

#[derive(Clone)]
pub struct DecisionStatsRepository {
    pool: Pool,
}

impl DecisionStatsRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Upsert one observation. `would_hit`/`live_hit` are 0 or 1 per call;
    /// the deltas keep this a single atomic statement — concurrent sessions
    /// bumping the same tier cannot lose counts to a read-modify-write race.
    pub async fn bump(&self, tier_id: &str, would_hit: bool, live_hit: bool) -> Result<()> {
        let tier_id = tier_id.to_string();
        let (wh, lh) = (i64::from(would_hit), i64::from(live_hit));
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO decision_stats (tier_id, calls, would_hit, live_hit, last_seen_at) \
                     VALUES (?1, 1, ?2, ?3, strftime('%s','now')) \
                     ON CONFLICT(tier_id) DO UPDATE SET \
                       calls = calls + 1, \
                       would_hit = would_hit + excluded.would_hit, \
                       live_hit = live_hit + excluded.live_hit, \
                       last_seen_at = excluded.last_seen_at",
                    params![tier_id, wh, lh],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to bump decision_stats")?;
        Ok(())
    }

    pub async fn get(&self, tier_id: &str) -> Result<Option<DecisionStats>> {
        let tier_id = tier_id.to_string();
        let row = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT tier_id, calls, would_hit, live_hit, last_seen_at \
                     FROM decision_stats WHERE tier_id = ?1",
                )?;
                let mapped = stmt
                    .query_map(params![tier_id], Self::map_row)?
                    .next()
                    .transpose()?;
                Ok::<_, rusqlite::Error>(mapped)
            })
            .await
            .map_err(interact_err)?
            .context("Failed to read decision_stats row")?;
        Ok(row)
    }

    /// All tiers, ordered for the /usage block (PR3): hottest last.
    pub async fn all(&self) -> Result<Vec<DecisionStats>> {
        let rows = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT tier_id, calls, would_hit, live_hit, last_seen_at \
                     FROM decision_stats ORDER BY calls DESC, tier_id ASC",
                )?;
                let mapped = stmt
                    .query_map([], Self::map_row)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok::<_, rusqlite::Error>(mapped)
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list decision_stats")?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DecisionStats> {
        Ok(DecisionStats {
            tier_id: row.get(0)?,
            calls: row.get(1)?,
            would_hit: row.get(2)?,
            live_hit: row.get(3)?,
            last_seen_at: row.get(4)?,
        })
    }
}
