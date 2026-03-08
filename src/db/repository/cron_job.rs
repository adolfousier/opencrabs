use crate::db::Pool;
use crate::db::database::interact_err;
use crate::db::models::CronJob;
use anyhow::{Context, Result};
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
                    "INSERT INTO cron_jobs (id, name, cron_expr, timezone, prompt, provider, model, thinking, auto_approve, deliver_to, enabled, next_run_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
                        j.enabled as i32,
                        j.next_run_at.map(|d| d.to_rfc3339()),
                        j.created_at.to_rfc3339(),
                        j.updated_at.to_rfc3339(),
                    ],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to insert cron job")?;
        Ok(())
    }

    pub async fn list_all(&self) -> Result<Vec<CronJob>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                let mut stmt = conn.prepare_cached("SELECT * FROM cron_jobs ORDER BY name")?;
                let rows = stmt.query_map([], CronJob::from_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list cron jobs")
    }

    pub async fn list_enabled(&self) -> Result<Vec<CronJob>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(|conn| {
                let mut stmt =
                    conn.prepare_cached("SELECT * FROM cron_jobs WHERE enabled = 1 ORDER BY name")?;
                let rows = stmt.query_map([], CronJob::from_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list enabled cron jobs")
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
