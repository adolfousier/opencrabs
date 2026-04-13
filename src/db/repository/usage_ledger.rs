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

/// Normalize model names for consistent ledger tracking.
/// - Claude: "claude-opus-4-6" → "opus-4-6", bare "opus" → "opus-4-6"
/// - Qwen: "coder-model", "qwen3.6-plus", "qwen-3.6-plus", "qwen/qwen3.6-plus" → "qwen3.6-plus"
/// - OpenRouter/OpenCode prefixed: "openrouter/X", "opencode/X" → "X"
pub(crate) fn normalize_model_name(model: &str) -> String {
    // Strip provider prefixes (openrouter/model, opencode/model, qwen/model, etc.)
    let stripped = model
        .split('/')
        .next_back()
        .unwrap_or(model);

    // Claude normalization
    let stripped = stripped.strip_prefix("claude-").unwrap_or(stripped);
    match stripped {
        "opus" | "opus-4-6" => return "opus-4-6".to_string(),
        "sonnet" | "sonnet-4-6" => return "sonnet-4-6".to_string(),
        "haiku" | "haiku-4-5" => return "haiku-4-5".to_string(),
        _ => {}
    }

    // Qwen normalization: coder-model, qwen3.6-plus, qwen-3.6-plus → qwen3.6-plus
    match stripped {
        "coder-model" | "qwen-3.6-plus" | "qwen3.6-plus" => {
            return "qwen3.6-plus".to_string();
        }
        "qwen3.5-plus" | "qwen-3.5-plus" => return "qwen3.5-plus".to_string(),
        _ => {}
    }

    stripped.to_string()
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
        let mdl = normalize_model_name(model);
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

    /// Get usage stats grouped by model (normalizes "claude-X" → "X" to merge duplicates)
    pub async fn stats_by_model(&self) -> Result<Vec<ModelUsageStats>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                // Normalize model names in SQL to merge duplicates:
                // "claude-opus-4-6" → "opus-4-6", bare "opus" → "opus-4-6", etc.
                let mut stmt = conn.prepare_cached(
                    "SELECT \
                       CASE \
                         WHEN model IN ('opus', 'claude-opus-4-6') THEN 'opus-4-6' \
                         WHEN model IN ('sonnet', 'claude-sonnet-4-6') THEN 'sonnet-4-6' \
                         WHEN model IN ('haiku', 'claude-haiku-4-5-20251001') THEN 'haiku-4-5' \
                         WHEN model LIKE 'claude-%' THEN REPLACE(model, 'claude-', '') \
                         WHEN model IN ('coder-model', 'qwen-3.6-plus', 'qwen/qwen3.6-plus') THEN 'qwen3.6-plus' \
                         WHEN model IN ('qwen-3.5-plus', 'qwen/qwen3.5-plus') THEN 'qwen3.5-plus' \
                         WHEN model LIKE '%/%' THEN SUBSTR(model, INSTR(model, '/') + 1) \
                         ELSE model \
                       END AS normalized_model, \
                       COALESCE(SUM(token_count), 0), \
                       COALESCE(SUM(cost), 0.0), \
                       COUNT(*) \
                     FROM usage_ledger WHERE model != '' \
                     GROUP BY normalized_model \
                     ORDER BY SUM(cost) DESC",
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
