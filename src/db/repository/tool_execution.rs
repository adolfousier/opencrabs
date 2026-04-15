//! Tool Execution Repository
//!
//! Tracks every tool call for usage analytics (Core Tools card in /usage dashboard).
//! Entries are append-only.

use crate::db::Pool;
use crate::db::database::interact_err;
use anyhow::{Context, Result};
use rusqlite::params;

/// Aggregated tool usage stats
#[derive(Debug, Clone)]
pub struct ToolUsageStats {
    pub tool_name: String,
    pub call_count: i64,
}

/// Repository for tool execution tracking
#[derive(Clone)]
pub struct ToolExecutionRepository {
    pool: Pool,
}

impl ToolExecutionRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Record a tool execution
    pub async fn record(
        &self,
        id: &str,
        message_id: &str,
        session_id: &str,
        tool_name: &str,
        status: &str,
    ) -> Result<()> {
        let id = id.to_string();
        let message_id = message_id.to_string();
        let session_id = session_id.to_string();
        let tool_name = tool_name.to_string();
        let status = status.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO tool_executions (id, message_id, session_id, tool_name, status) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, message_id, session_id, tool_name, status],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to record tool execution")?;
        Ok(())
    }

    /// Get tool usage stats grouped by tool_name, optionally filtered by time period
    pub async fn stats_by_tool(&self, since_epoch: Option<i64>) -> Result<Vec<ToolUsageStats>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let (query, param): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                    if let Some(since) = since_epoch {
                        (
                            "SELECT tool_name, COUNT(*) as cnt \
                             FROM tool_executions \
                             WHERE created_at >= ?1 \
                             GROUP BY tool_name \
                             ORDER BY cnt DESC"
                                .to_string(),
                            vec![Box::new(since)],
                        )
                    } else {
                        (
                            "SELECT tool_name, COUNT(*) as cnt \
                             FROM tool_executions \
                             GROUP BY tool_name \
                             ORDER BY cnt DESC"
                                .to_string(),
                            vec![],
                        )
                    };
                let mut stmt = conn.prepare_cached(&query)?;
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param.iter().map(|p| p.as_ref()).collect();
                let rows = stmt.query_map(param_refs.as_slice(), |row| {
                    Ok(ToolUsageStats {
                        tool_name: row.get(0)?,
                        call_count: row.get(1)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to query tool usage stats")
    }
}
