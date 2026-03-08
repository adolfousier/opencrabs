//! Usage Ledger Repository
//!
//! Cumulative usage tracking that persists across session deletes and compaction.
//! Entries are append-only — never deleted.

use crate::db::Pool;
use crate::db::database::interact_err;
use anyhow::{Context, Result};
use rusqlite::params;

/// Aggregated usage stats grouped by model
#[derive(Debug, Clone)]
pub struct ModelUsageStats {
    pub model: String,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub entry_count: i64,
}

/// Repository for usage ledger operations
#[derive(Clone)]
pub struct UsageLedgerRepository {
    pool: Pool,
}

impl UsageLedgerRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Record a usage event (append-only, never deleted)
    pub async fn record(
        &self,
        session_id: &str,
        model: &str,
        token_count: i32,
        cost: f64,
    ) -> Result<()> {
        let sid = session_id.to_string();
        let mdl = model.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO usage_ledger (session_id, model, token_count, cost) VALUES (?1, ?2, ?3, ?4)",
                    params![sid, mdl, token_count, cost],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to record usage")?;

        Ok(())
    }

    /// Get all-time totals (tokens + cost)
    pub async fn totals(&self) -> Result<(i64, f64)> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                conn.query_row(
                    "SELECT COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0) FROM usage_ledger",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to query usage totals")
    }

    /// Get usage stats grouped by model
    pub async fn stats_by_model(&self) -> Result<Vec<ModelUsageStats>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT model, COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0), COUNT(*) \
                     FROM usage_ledger WHERE model != '' GROUP BY model ORDER BY SUM(cost) DESC",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(ModelUsageStats {
                        model: row.get(0)?,
                        total_tokens: row.get(1)?,
                        total_cost: row.get(2)?,
                        entry_count: row.get(3)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to query usage by model")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[tokio::test]
    async fn test_record_and_totals() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = UsageLedgerRepository::new(db.pool().clone());

        repo.record("s1", "claude-sonnet-4-5", 100, 0.05)
            .await
            .unwrap();
        repo.record("s1", "claude-sonnet-4-5", 200, 0.10)
            .await
            .unwrap();
        repo.record("s2", "claude-opus-4", 500, 0.50).await.unwrap();

        let (tokens, cost) = repo.totals().await.unwrap();
        assert_eq!(tokens, 800);
        assert!((cost - 0.65).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_stats_by_model() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = UsageLedgerRepository::new(db.pool().clone());

        repo.record("s1", "sonnet", 100, 0.05).await.unwrap();
        repo.record("s2", "opus", 500, 0.50).await.unwrap();
        repo.record("s3", "sonnet", 200, 0.10).await.unwrap();

        let stats = repo.stats_by_model().await.unwrap();
        assert_eq!(stats.len(), 2);
        // opus should be first (higher cost)
        assert_eq!(stats[0].model, "opus");
        assert_eq!(stats[0].total_tokens, 500);
        assert_eq!(stats[1].model, "sonnet");
        assert_eq!(stats[1].total_tokens, 300);
    }
}
