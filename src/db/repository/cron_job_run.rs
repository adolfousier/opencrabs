use crate::db::Pool;
use crate::db::database::interact_err;
use crate::db::models::CronJobRun;
use anyhow::{Context, Result};
use rusqlite::params;

#[derive(Clone)]
pub struct CronJobRunRepository {
    pool: Pool,
}

impl CronJobRunRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Insert a new run record (status = "running").
    pub async fn insert(&self, run: &CronJobRun) -> Result<()> {
        let r = run.clone();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO cron_job_runs (id, job_id, job_name, status, content, error, input_tokens, output_tokens, cost, provider, model, started_at, completed_at, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        r.id.to_string(),
                        r.job_id.to_string(),
                        r.job_name,
                        r.status,
                        r.content,
                        r.error,
                        r.input_tokens,
                        r.output_tokens,
                        r.cost,
                        r.provider,
                        r.model,
                        r.started_at.to_rfc3339(),
                        r.completed_at.map(|d: chrono::DateTime<chrono::Utc>| d.to_rfc3339()),
                        r.created_at.to_rfc3339(),
                    ],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to insert cron job run")?;
        Ok(())
    }

    /// Mark a run as completed with success.
    pub async fn complete_success(
        &self,
        run_id: &str,
        content: &str,
        input_tokens: i64,
        output_tokens: i64,
        cost: f64,
    ) -> Result<()> {
        let id = run_id.to_string();
        let content = content.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE cron_job_runs SET status = 'success', content = ?1, input_tokens = ?2, output_tokens = ?3, cost = ?4, completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?5",
                    params![content, input_tokens, output_tokens, cost, id],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to update cron job run")?;
        Ok(())
    }

    /// Mark a run as failed with error.
    pub async fn complete_error(&self, run_id: &str, error: &str) -> Result<()> {
        let id = run_id.to_string();
        let error = error.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "UPDATE cron_job_runs SET status = 'error', error = ?1, completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
                    params![error, id],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to update cron job run error")?;
        Ok(())
    }

    /// List recent runs for a specific job (most recent first).
    pub async fn list_by_job(&self, job_id: &str, limit: i64) -> Result<Vec<CronJobRun>> {
        let job_id = job_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| -> rusqlite::Result<Vec<CronJobRun>> {
                let mut stmt = conn.prepare_cached(
                    "SELECT * FROM cron_job_runs WHERE job_id = ?1 ORDER BY started_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![job_id, limit], CronJobRun::from_row)?;
                rows.collect()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list cron job runs")
    }

    /// List all recent runs across all jobs.
    pub async fn list_recent(&self, limit: i64) -> Result<Vec<CronJobRun>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| -> rusqlite::Result<Vec<CronJobRun>> {
                let mut stmt = conn.prepare_cached(
                    "SELECT * FROM cron_job_runs ORDER BY started_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit], CronJobRun::from_row)?;
                rows.collect()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list recent cron job runs")
    }
}
