//! Decision Cache Repository (#1648)
//!
//! Storage half of the L1 exact-decision-reuse ring. Keys come from
//! [`crate::decisions::normalize`] and are the row's whole identity:
//! (tier, policy_version, normalizer_version, canonical input) under one
//! sha256. A miss is just "ask the model" — correctness lives at the write
//! gate: a margin below the tier's floor is refused HERE, so a borderline
//! decision can never be frozen no matter what a caller does (PR2's
//! `decide_cached` relies on this being unskippable).
//!
//! Removal path (release-day evaluation, #1648): `[decisions]` off in
//! config stops every read/write; `DROP TABLE IF EXISTS decision_cache`
//! finishes it. Nothing else references this table.

use crate::db::Pool;
use crate::db::database::interact_err;
use anyhow::{Context, Result};
use rusqlite::params;

/// One cached decision, as stored.
#[derive(Debug, Clone)]
pub struct DecisionCacheRow {
    pub key: String,
    pub tier_id: String,
    pub result_json: String,
    pub p: Option<f64>,
    pub margin: Option<f64>,
    pub policy_version: String,
    pub normalizer_version: String,
    pub created_at: i64,
    pub hits: i64,
    pub last_used_at: Option<i64>,
}

/// A freshly derived decision ready to be written.
#[derive(Debug, Clone)]
pub struct DecisionPut {
    pub key: String,
    pub tier_id: String,
    /// The decision itself, verbatim JSON from the model's tiny schema.
    pub result_json: String,
    /// Model-reported confidence, when it reported one.
    pub p: Option<f64>,
    /// Distance to the runner-up. `None` means not measurable (a
    /// deterministic-shaped decision); measurable-but-borderline is
    /// refused by the floor gate in [`DecisionCacheRepository::put`].
    pub margin: Option<f64>,
    pub policy_version: String,
    pub normalizer_version: String,
}

#[derive(Clone)]
pub struct DecisionCacheRepository {
    pool: Pool,
}

impl DecisionCacheRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Exact-key lookup. `None` = miss = ask the model; there is no fuzzy
    /// path here (similarity reuse is a separate, opt-in later thing and
    /// never goes through this method).
    pub async fn get(&self, key: &str) -> Result<Option<DecisionCacheRow>> {
        let key = key.to_string();
        let row = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT key, tier_id, result_json, p, margin, policy_version, \
                     normalizer_version, created_at, hits, last_used_at \
                     FROM decision_cache WHERE key = ?1",
                )?;
                let mapped = stmt
                    .query_map(params![key], |row| {
                        Ok(DecisionCacheRow {
                            key: row.get(0)?,
                            tier_id: row.get(1)?,
                            result_json: row.get(2)?,
                            p: row.get(3)?,
                            margin: row.get(4)?,
                            policy_version: row.get(5)?,
                            normalizer_version: row.get(6)?,
                            created_at: row.get(7)?,
                            hits: row.get(8)?,
                            last_used_at: row.get(9)?,
                        })
                    })?
                    .next()
                    .transpose()?;
                Ok::<_, rusqlite::Error>(mapped)
            })
            .await
            .map_err(interact_err)?
            .context("Failed to read decision_cache row")?;
        Ok(row)
    }

    /// Write a freshly derived decision.
    ///
    /// `margin_floor` is the tier's gate: a measurable margin below it is
    /// refused (`Ok(false)`) and the caller must treat the decision as
    /// live-only, re-asking next time. Refusal is the only loss this table
    /// can hand out, and it is always the cheap one.
    ///
    /// Re-put of an existing key refreshes the stored result in place: one
    /// row per key, hit counter and last_used preserved.
    pub async fn put(&self, input: DecisionPut, margin_floor: f64) -> Result<bool> {
        if let Some(m) = input.margin
            && m < margin_floor
        {
            tracing::debug!(
                "decision_cache: refused write for tier {} (margin {m} < floor {margin_floor})",
                input.tier_id
            );
            return Ok(false);
        }
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO decision_cache \
                     (key, tier_id, result_json, p, margin, policy_version, normalizer_version, created_at, hits) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s','now'), 0) \
                     ON CONFLICT(key) DO UPDATE SET \
                       result_json = excluded.result_json, \
                       p = excluded.p, \
                       margin = excluded.margin, \
                       created_at = excluded.created_at",
                    params![
                        input.key,
                        input.tier_id,
                        input.result_json,
                        input.p,
                        input.margin,
                        input.policy_version,
                        input.normalizer_version,
                    ],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to write decision_cache row")?;
        Ok(true)
    }

    /// Record one use: the /usage accounting counter and the TTL signal.
    /// A missing key is not an error (the row may have been swept between
    /// get and bump); a missed bump under-counts, never corrupts.
    pub async fn bump_hits(&self, key: &str) -> Result<()> {
        let key = key.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE decision_cache \
                     SET hits = hits + 1, last_used_at = strftime('%s','now') \
                     WHERE key = ?1",
                    params![key],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to bump decision_cache hits")?;
        Ok(())
    }

    /// Delete one (tier, policy_version) slice: the stale-policy sweeper and
    /// the kill-switch cleanup. Returns how many rows went.
    pub async fn delete_by_tier_version(
        &self,
        tier_id: &str,
        policy_version: &str,
    ) -> Result<usize> {
        let (tier_id, policy_version) = (tier_id.to_string(), policy_version.to_string());
        let n = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM decision_cache WHERE tier_id = ?1 AND policy_version = ?2",
                    params![tier_id, policy_version],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete decision_cache slice")?;
        Ok(n)
    }

    /// Row count per tier, for the /usage decisions block (#1648 PR3):
    /// the cached-answer distribution alongside the call counters.
    pub async fn count_by_tier(&self) -> Result<Vec<(String, i64)>> {
        let rows = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                let mut stmt =
                    conn.prepare("SELECT tier_id, COUNT(*) FROM decision_cache GROUP BY tier_id")?;
                let mapped = stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok::<_, rusqlite::Error>(mapped)
            })
            .await
            .map_err(interact_err)?
            .context("Failed to count decision_cache rows by tier")?;
        Ok(rows)
    }

    /// TTL pruning for one tier (#1648 PR3): rows untouched for longer than
    /// `ttl_hours` go. Recency is `last_used_at` when a hit has refreshed
    /// it, else `created_at`; a row is only ever kept by being useful.
    /// Only called for tiers with `ttl_hours` set; `None` in config means
    /// "bounded by policy_version only", which is the sweeper's job.
    pub async fn prune_expired(&self, tier_id: &str, ttl_hours: i64) -> Result<usize> {
        let tier_id = tier_id.to_string();
        let n = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM decision_cache \
                     WHERE tier_id = ?1 \
                       AND COALESCE(last_used_at, created_at) \
                           < strftime('%s','now') - (?2 * 3600)",
                    params![tier_id, ttl_hours],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to prune expired decision_cache rows")?;
        Ok(n)
    }

    /// Delete rows for tiers with no `[decisions.tiers]` entry (#1648 PR3).
    /// An unconfigured tier's rows can never be read: lookup starts from
    /// config, so these are garbage the kill switch left behind. Passing an
    /// empty list (feature fully off) therefore clears the table, which is
    /// exactly the removal cheapness the kill rule promises.
    pub async fn delete_unknown_tiers(&self, known_tiers: &[String]) -> Result<usize> {
        let known = known_tiers.to_vec();
        let n = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                if known.is_empty() {
                    return conn.execute("DELETE FROM decision_cache", []);
                }
                let placeholders = known.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql =
                    format!("DELETE FROM decision_cache WHERE tier_id NOT IN ({placeholders})");
                let mut stmt = conn.prepare(&sql)?;
                stmt.execute(rusqlite::params_from_iter(known.iter()))
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete orphaned decision_cache rows")?;
        Ok(n)
    }
}
