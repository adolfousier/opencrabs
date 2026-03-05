use crate::db::models::CronJob;
use anyhow::Result;
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct CronJobRepository {
    pool: SqlitePool,
}

impl CronJobRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, job: &CronJob) -> Result<()> {
        sqlx::query(
            "INSERT INTO cron_jobs (id, name, cron_expr, timezone, prompt, provider, model, thinking, auto_approve, deliver_to, enabled, next_run_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(job.id.to_string())
        .bind(&job.name)
        .bind(&job.cron_expr)
        .bind(&job.timezone)
        .bind(&job.prompt)
        .bind(&job.provider)
        .bind(&job.model)
        .bind(&job.thinking)
        .bind(job.auto_approve as i32)
        .bind(&job.deliver_to)
        .bind(job.enabled as i32)
        .bind(job.next_run_at.map(|d| d.to_rfc3339()))
        .bind(job.created_at.to_rfc3339())
        .bind(job.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_all(&self) -> Result<Vec<CronJob>> {
        let jobs = sqlx::query_as::<_, CronJob>("SELECT * FROM cron_jobs ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        Ok(jobs)
    }

    pub async fn list_enabled(&self) -> Result<Vec<CronJob>> {
        let jobs =
            sqlx::query_as::<_, CronJob>("SELECT * FROM cron_jobs WHERE enabled = 1 ORDER BY name")
                .fetch_all(&self.pool)
                .await?;
        Ok(jobs)
    }

    pub async fn find_by_id(&self, id: &str) -> Result<Option<CronJob>> {
        let job = sqlx::query_as::<_, CronJob>("SELECT * FROM cron_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(job)
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<CronJob>> {
        let job = sqlx::query_as::<_, CronJob>("SELECT * FROM cron_jobs WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(job)
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM cron_jobs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE cron_jobs SET enabled = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?",
        )
        .bind(enabled as i32)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update_last_run(&self, id: &str, next_run_at: Option<&str>) -> Result<()> {
        sqlx::query(
            "UPDATE cron_jobs SET last_run_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), next_run_at = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?",
        )
        .bind(next_run_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
