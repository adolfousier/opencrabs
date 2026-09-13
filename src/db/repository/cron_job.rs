use crate::db::Pool;
use crate::db::database::interact_err;
use crate::db::models::CronJob;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::params;

/// Extension trait for rusqlite to add `.optional()` to query results
trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Owned SQL bind value for the dynamic UPDATE built by
/// [`CronJobRepository::update_fields`].
enum SqlVal {
    Text(String),
    Int(i32),
    Null,
}

impl rusqlite::ToSql for SqlVal {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(match self {
            SqlVal::Text(s) => rusqlite::types::ToSqlOutput::Borrowed(
                rusqlite::types::ValueRef::Text(s.as_bytes()),
            ),
            SqlVal::Int(i) => {
                rusqlite::types::ToSqlOutput::Owned(rusqlite::types::Value::Integer(i64::from(*i)))
            }
            SqlVal::Null => rusqlite::types::ToSqlOutput::Owned(rusqlite::types::Value::Null),
        })
    }
}

/// Patch for [`CronJobRepository::update_fields`].
///
/// Every field is tri-state: `None` = keep the current value, `Some(x)` = set.
/// For the optional string overrides, `Some(None)` clears the column back to
/// NULL and `Some(Some(v))` sets it. The row id is never touched: an update
/// patches the existing row in place.
#[derive(Debug, Clone, Default)]
pub struct CronJobPatch {
    pub name: Option<String>,
    pub cron_expr: Option<String>,
    pub timezone: Option<String>,
    pub prompt: Option<String>,
    pub provider: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub thinking: Option<String>,
    pub auto_approve: Option<bool>,
    pub deliver_to: Option<Option<String>>,
    pub deliver_api_key: Option<Option<String>>,
    pub enabled: Option<bool>,
    /// Explicit next_run_at update: Some(Some(dt)) sets it, Some(None) clears to NULL.
    pub next_run_at: Option<Option<DateTime<Utc>>>,
    /// When true, `next_run_at` is reset to NULL so the scheduler recomputes
    /// the next fire time from the (possibly changed) schedule on the next
    /// tick. Set this whenever `cron_expr` or `timezone` changes.
    pub reset_next_run: bool,
}

impl CronJobPatch {
    /// True when no field was provided (nothing to write).
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.cron_expr.is_none()
            && self.timezone.is_none()
            && self.prompt.is_none()
            && self.provider.is_none()
            && self.model.is_none()
            && self.thinking.is_none()
            && self.auto_approve.is_none()
            && self.deliver_to.is_none()
            && self.deliver_api_key.is_none()
            && self.enabled.is_none()
            && self.next_run_at.is_none()
            && !self.reset_next_run
    }
}

#[derive(Clone)]
pub struct CronJobRepository {
    pool: Pool,
}

impl CronJobRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, job: &CronJob) -> Result<()> {
        let j = job.clone();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO cron_jobs (id, name, cron_expr, timezone, prompt, provider, model, thinking, auto_approve, deliver_to, deliver_api_key, enabled, next_run_at, created_at, updated_at, profile_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        j.id.to_string(),
                        j.name,
                        j.cron_expr,
                        j.timezone,
                        j.prompt,
                        j.provider,
                        j.model,
                        j.thinking,
                        j.auto_approve as i32,
                        j.deliver_to,
                        j.deliver_api_key,
                        j.enabled as i32,
                        j.next_run_at.map(|d| d.to_rfc3339()),
                        j.created_at.to_rfc3339(),
                        j.updated_at.to_rfc3339(),
                        j.profile_name,
                    ],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to insert cron job")?;
        Ok(())
    }

    pub async fn list_all(&self) -> Result<Vec<CronJob>> {
        // Retry once after 100ms if the query fails (handles brief DB contention
        // from scheduler updates). Timeout after 5s to prevent hangs (#665).
        let query = || async {
            self.pool
                .get()
                .await
                .context("Failed to get connection")?
                .interact(|conn| -> anyhow::Result<Vec<CronJob>> {
                    let mut stmt = conn.prepare_cached("SELECT * FROM cron_jobs ORDER BY name")?;
                    let rows = stmt.query_map([], |row| {
                        // Capture the id BEFORE from_row so a decode failure
                        // points at a specific job rather than the anonymous
                        // "row 0/0" rusqlite produces by default. Useful for
                        // the 2026-05-17 class of bug where one bad row
                        // silently empties an entire list query.
                        let id: String = row.get::<_, String>("id").unwrap_or_default();
                        CronJob::from_row(row).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                format!("cron_jobs row id={id}: {e}").into(),
                            )
                        })
                    })?;
                    let mut out = Vec::new();
                    for row in rows {
                        out.push(row.context("cron_jobs row decode failed")?);
                    }
                    Ok(out)
                })
                .await
                .map_err(interact_err)?
                .context("Failed to list cron jobs")
        };

        match tokio::time::timeout(std::time::Duration::from_secs(5), query()).await {
            Ok(result) => result,
            Err(_) => {
                // Timeout — retry once after 100ms backoff
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                query().await
            }
        }
    }
    pub async fn list_enabled(&self) -> Result<Vec<CronJob>> {
        // Same retry/timeout pattern as list_all (#665)
        let query = || async {
            self.pool
                .get()
                .await
                .context("Failed to get connection")?
                .interact(|conn| {
                    let mut stmt = conn.prepare_cached(
                        "SELECT * FROM cron_jobs WHERE enabled = 1 ORDER BY name",
                    )?;
                    let rows = stmt.query_map([], CronJob::from_row)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                })
                .await
                .map_err(interact_err)?
                .context("Failed to list enabled cron jobs")
        };

        match tokio::time::timeout(std::time::Duration::from_secs(5), query()).await {
            Ok(result) => result,
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                query().await
            }
        }
    }

    pub async fn find_by_id(&self, id: &str) -> Result<Option<CronJob>> {
        let id = id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.prepare_cached("SELECT * FROM cron_jobs WHERE id = ?1")?
                    .query_row(params![id], CronJob::from_row)
                    .optional()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to find cron job")
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<CronJob>> {
        let name = name.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.prepare_cached("SELECT * FROM cron_jobs WHERE name = ?1")?
                    .query_row(params![name], CronJob::from_row)
                    .optional()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to find cron job by name")
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        let rows = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id]))
            .await
            .map_err(interact_err)?
            .context("Failed to delete cron job")?;
        Ok(rows > 0)
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let id = id.to_string();
        let rows = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE cron_jobs SET enabled = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
                    params![enabled as i32, id],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to set cron job enabled")?;
        Ok(rows > 0)
    }

    /// Patch an existing job in place. Only the fields set in `patch` are
    /// written; everything else keeps its current value and the row id is
    /// never touched. Returns false when the job does not exist or the patch
    /// is empty (nothing to write).
    ///
    /// The UPDATE statement only lists the provided columns, so a concurrent
    /// scheduler tick writing `last_run_at`/`next_run_at` is never clobbered
    /// by a stale full-row snapshot (#966).
    pub async fn update_fields(&self, id: &str, patch: CronJobPatch) -> Result<bool> {
        if patch.is_empty() {
            return Ok(false);
        }
        let id = id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| -> anyhow::Result<bool> {
                fn push(sets: &mut Vec<String>, vals: &mut Vec<SqlVal>, col: &str, v: SqlVal) {
                    sets.push(format!("{col} = ?{}", vals.len() + 1));
                    vals.push(v);
                }
                fn push_opt(
                    sets: &mut Vec<String>,
                    vals: &mut Vec<SqlVal>,
                    col: &str,
                    v: Option<Option<String>>,
                ) {
                    match v {
                        Some(Some(s)) => push(sets, vals, col, SqlVal::Text(s)),
                        Some(None) => push(sets, vals, col, SqlVal::Null),
                        None => {}
                    }
                }

                let mut sets: Vec<String> = Vec::new();
                let mut vals: Vec<SqlVal> = Vec::new();

                if let Some(v) = patch.name {
                    push(&mut sets, &mut vals, "name", SqlVal::Text(v));
                }
                if let Some(v) = patch.cron_expr {
                    push(&mut sets, &mut vals, "cron_expr", SqlVal::Text(v));
                }
                if let Some(v) = patch.timezone {
                    push(&mut sets, &mut vals, "timezone", SqlVal::Text(v));
                }
                if let Some(v) = patch.prompt {
                    push(&mut sets, &mut vals, "prompt", SqlVal::Text(v));
                }
                push_opt(&mut sets, &mut vals, "provider", patch.provider);
                push_opt(&mut sets, &mut vals, "model", patch.model);
                if let Some(v) = patch.thinking {
                    push(&mut sets, &mut vals, "thinking", SqlVal::Text(v));
                }
                if let Some(v) = patch.auto_approve {
                    push(
                        &mut sets,
                        &mut vals,
                        "auto_approve",
                        SqlVal::Int(i32::from(v)),
                    );
                }
                push_opt(&mut sets, &mut vals, "deliver_to", patch.deliver_to);
                push_opt(
                    &mut sets,
                    &mut vals,
                    "deliver_api_key",
                    patch.deliver_api_key,
                );
                if let Some(v) = patch.enabled {
                    push(&mut sets, &mut vals, "enabled", SqlVal::Int(i32::from(v)));
                }
                match patch.next_run_at {
                    Some(Some(dt)) => {
                        push(
                            &mut sets,
                            &mut vals,
                            "next_run_at",
                            SqlVal::Text(dt.to_rfc3339()),
                        );
                    }
                    Some(None) => {
                        push(&mut sets, &mut vals, "next_run_at", SqlVal::Null);
                    }
                    None => {
                        if patch.reset_next_run {
                            push(&mut sets, &mut vals, "next_run_at", SqlVal::Null);
                        }
                    }
                }

                // Bump updated_at the same way every other write on this
                // table does.
                sets.push("updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')".to_string());

                let sql = format!(
                    "UPDATE cron_jobs SET {} WHERE id = ?{}",
                    sets.join(", "),
                    vals.len() + 1
                );
                vals.push(SqlVal::Text(id));
                let rows = conn.execute(&sql, rusqlite::params_from_iter(vals))?;
                Ok(rows > 0)
            })
            .await
            .map_err(interact_err)?
            .context("Failed to update cron job")
    }

    /// Set next_run_at to a past timestamp so the scheduler fires it on the next tick.
    /// Also ensures the job is enabled.
    pub async fn trigger_now(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        let rows = self
            .pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE cron_jobs SET next_run_at = '2000-01-01T00:00:00Z', enabled = 1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
                    params![id],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to trigger cron job")?;
        Ok(rows > 0)
    }

    pub async fn update_last_run(&self, id: &str, next_run_at: Option<&str>) -> Result<()> {
        let id = id.to_string();
        let next = next_run_at.map(|s| s.to_string());
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE cron_jobs SET last_run_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), next_run_at = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
                    params![next, id],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to update last run")?;
        Ok(())
    }
}
