//! Usage Ledger Repository
//!
//! Cumulative usage tracking that persists across session deletes and compaction.
//! Entries are append-only — never deleted.

use anyhow::{Context, Result};
use sqlx::SqlitePool;

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
    pool: SqlitePool,
}

impl UsageLedgerRepository {
    pub fn new(pool: SqlitePool) -> Self {
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
        sqlx::query(
            "INSERT INTO usage_ledger (session_id, model, token_count, cost) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(model)
        .bind(token_count)
        .bind(cost)
        .execute(&self.pool)
        .await
        .context("Failed to record usage")?;

        Ok(())
    }

    /// Get all-time totals (tokens + cost)
    pub async fn totals(&self) -> Result<(i64, f64)> {
        let row: (i64, f64) = sqlx::query_as(
            "SELECT COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0) FROM usage_ledger",
        )
        .fetch_one(&self.pool)
        .await
        .context("Failed to query usage totals")?;

        Ok(row)
    }

    /// Get usage stats grouped by model
    pub async fn stats_by_model(&self) -> Result<Vec<ModelUsageStats>> {
        let rows: Vec<(String, i64, f64, i64)> = sqlx::query_as(
            "SELECT model, COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0), COUNT(*) \
             FROM usage_ledger WHERE model != '' GROUP BY model ORDER BY SUM(cost) DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed to query usage by model")?;

        Ok(rows
            .into_iter()
            .map(
                |(model, total_tokens, total_cost, entry_count)| ModelUsageStats {
                    model,
                    total_tokens,
                    total_cost,
                    entry_count,
                },
            )
            .collect())
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
