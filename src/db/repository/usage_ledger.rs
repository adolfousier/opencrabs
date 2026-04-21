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

    // Lowercase everything for consistent matching and display
    let lower = stripped.to_lowercase();

    // Strip common suffixes before matching: :free, -free, -thinking
    let base = lower
        .strip_suffix(":free")
        .or_else(|| lower.strip_suffix("-free"))
        .or_else(|| lower.strip_suffix("-thinking"))
        .unwrap_or(&lower);

    // Strip claude- prefix
    let base = base.strip_prefix("claude-").unwrap_or(base);

    match base {
        // Claude
        "opus" | "opus-4-6" => "opus-4-6".to_string(),
        "sonnet" | "sonnet-4-6" => "sonnet-4-6".to_string(),
        "haiku" | "haiku-4-5" | "haiku-4-5-20251001" => "haiku-4-5".to_string(),
        // Qwen
        "qwen-3.6-max-preview"
        | "qwen3.6-max-preview"
        | "qwen-3-6-max-preview"
        | "qwen3-6-max-preview"
        | "qwen-max-preview" => "qwen3.6-max-preview".to_string(),
        "coder-model" | "qwen-3.6-plus" | "qwen3.6-plus" => "qwen3.6-plus".to_string(),
        "qwen3.5-plus" | "qwen-3.5-plus" => "qwen3.5-plus".to_string(),
        // MiniMax
        "minimax-m2.5" => "minimax-m2.5".to_string(),
        "minimax-m2.7" => "minimax-m2.7".to_string(),
        // Mimo (free variants merge with base)
        "mimo-v2-omni" | "mimo-v2-omni-free" => "mimo-v2-omni".to_string(),
        "mimo-v2-pro" | "mimo-v2-pro-free" => "mimo-v2-pro".to_string(),
        // Kimi
        "kimi-k2.6" | "kimi-k2-6" | "kimik2.6" => "kimi-k2.6".to_string(),
        "kimi-k2.5" | "kimi-k2-5" => "kimi-k2.5".to_string(),
        // GLM / ZhiPu
        "glm-5-turbo" | "zhipu" => "glm-5-turbo".to_string(),
        // No match — return lowercased as-is
        _ => lower.to_string(),
    }
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
                // Normalize model names in SQL to merge duplicates.
                // 1. Strip provider prefix (openrouter/, opencode/, qwen/)
                // 2. Lowercase everything
                // 3. Strip :free/-free/-thinking suffixes
                // 4. Match known families to canonical names
                let mut stmt = conn.prepare_cached(
                    "WITH stripped AS ( \
                       SELECT *, \
                         LOWER(CASE WHEN model LIKE '%/%' \
                           THEN SUBSTR(model, INSTR(model, '/') + 1) \
                           ELSE model \
                         END) AS m1 \
                       FROM usage_ledger WHERE model != '' \
                     ), \
                     cleaned AS ( \
                       SELECT *, \
                         CASE \
                           WHEN m1 LIKE '%:free' THEN SUBSTR(m1, 1, LENGTH(m1) - 5) \
                           WHEN m1 LIKE '%-free' THEN SUBSTR(m1, 1, LENGTH(m1) - 5) \
                           WHEN m1 LIKE '%-thinking' THEN SUBSTR(m1, 1, LENGTH(m1) - 9) \
                           ELSE m1 \
                         END AS m2 \
                       FROM stripped \
                     ), \
                     prefixed AS ( \
                       SELECT *, \
                         CASE WHEN m2 LIKE 'claude-%' THEN SUBSTR(m2, 8) ELSE m2 END AS m3 \
                       FROM cleaned \
                     ) \
                     SELECT \
                       CASE \
                         WHEN m3 IN ('opus', 'opus-4-6') THEN 'opus-4-6' \
                         WHEN m3 IN ('sonnet', 'sonnet-4-6') THEN 'sonnet-4-6' \
                         WHEN m3 IN ('haiku', 'haiku-4-5', 'haiku-4-5-20251001') THEN 'haiku-4-5' \
                         WHEN m3 IN ('qwen-3.6-max-preview', 'qwen3.6-max-preview', 'qwen-3-6-max-preview', 'qwen3-6-max-preview', 'qwen-max-preview') THEN 'qwen3.6-max-preview' \
                         WHEN m3 IN ('coder-model', 'qwen3.6-plus', 'qwen-3.6-plus') THEN 'qwen3.6-plus' \
                         WHEN m3 IN ('qwen3.5-plus', 'qwen-3.5-plus') THEN 'qwen3.5-plus' \
                         WHEN m3 IN ('minimax-m2.5') THEN 'minimax-m2.5' \
                         WHEN m3 IN ('minimax-m2.7') THEN 'minimax-m2.7' \
                         WHEN m3 IN ('mimo-v2-omni', 'mimo-v2-omni-free') THEN 'mimo-v2-omni' \
                         WHEN m3 IN ('mimo-v2-pro', 'mimo-v2-pro-free') THEN 'mimo-v2-pro' \
                         WHEN m3 IN ('kimi-k2.6', 'kimi-k2-6', 'kimik2.6') THEN 'kimi-k2.6' \
                         WHEN m3 IN ('kimi-k2.5', 'kimi-k2-5') THEN 'kimi-k2.5' \
                         WHEN m3 IN ('glm-5-turbo', 'zhipu') THEN 'glm-5-turbo' \
                         ELSE m3 \
                       END AS normalized_model, \
                       COALESCE(SUM(token_count), 0), \
                       COALESCE(SUM(cost), 0.0), \
                       COUNT(*) \
                     FROM prefixed \
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
