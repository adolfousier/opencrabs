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
    let stripped = model.split('/').next_back().unwrap_or(model);

    // Claude normalization
    let stripped = stripped.strip_prefix("claude-").unwrap_or(stripped);
    match stripped {
        "opus" | "opus-4-6" => return "opus-4-6".to_string(),
        "sonnet" | "sonnet-4-6" => return "sonnet-4-6".to_string(),
        "haiku" | "haiku-4-5" => return "haiku-4-5".to_string(),
        _ => {}
    }

    // Qwen 3.6 Plus normalization: all variants (free, thinking, :free suffix) → qwen3.6-plus
    // Strip `:free` suffix and `-free`/`-thinking` suffixes before matching
    let qwen_stripped = stripped
        .strip_suffix(":free")
        .or_else(|| stripped.strip_suffix("-free"))
        .or_else(|| stripped.strip_suffix("-thinking"))
        .unwrap_or(stripped);
    match qwen_stripped {
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
                // Two-pass normalization via nested CASE:
                // 1. Strip provider prefix (openrouter/, opencode/, qwen/)
                // 2. Match known model families to canonical names
                let mut stmt = conn.prepare_cached(
                    "WITH stripped AS ( \
                       SELECT *, \
                         CASE WHEN model LIKE '%/%' \
                           THEN SUBSTR(model, INSTR(model, '/') + 1) \
                           ELSE model \
                         END AS m1 \
                       FROM usage_ledger WHERE model != '' \
                     ) \
                     SELECT \
                       CASE \
                         WHEN m1 IN ('opus', 'claude-opus-4-6') THEN 'opus-4-6' \
                         WHEN m1 IN ('sonnet', 'claude-sonnet-4-6') THEN 'sonnet-4-6' \
                         WHEN m1 IN ('haiku', 'claude-haiku-4-5-20251001') THEN 'haiku-4-5' \
                         WHEN m1 LIKE 'claude-%' THEN REPLACE(m1, 'claude-', '') \
                         WHEN m1 IN ('coder-model', 'qwen3.6-plus', 'qwen-3.6-plus', \
                              'qwen3.6-plus:free', 'qwen3.6-plus-free', 'qwen-3.6-plus-free', \
                              'qwen-3.6-plus-thinking', 'qwen3.6-plus-thinking') THEN 'qwen3.6-plus' \
                         WHEN m1 IN ('qwen3.5-plus', 'qwen-3.5-plus') THEN 'qwen3.5-plus' \
                         ELSE m1 \
                       END AS normalized_model, \
                       COALESCE(SUM(token_count), 0), \
                       COALESCE(SUM(cost), 0.0), \
                       COUNT(*) \
                     FROM stripped \
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
