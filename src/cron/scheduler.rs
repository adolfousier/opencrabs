//! Cron Scheduler
//!
//! Background task that checks the `cron_jobs` table every 60 seconds,
//! executes due jobs in a dedicated "Cron" session, and delivers results
//! to the configured channel. Cron jobs are fully isolated from the TUI —
//! they never share or mutate the user's active session.

use crate::channels::ChannelFactory;
use crate::config::Config;
use crate::db::CronJobRepository;
use crate::db::CronJobRunRepository;
use crate::db::models::{CronJob, CronJobRun};
use crate::services::{ServiceContext, SessionService};
use chrono::Utc;
use cron::Schedule;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Name used for the shared cron session.
const CRON_SESSION_NAME: &str = "Cron";

/// Background cron scheduler that polls the database and executes due jobs.
pub struct CronScheduler {
    repo: CronJobRepository,
    run_repo: CronJobRunRepository,
    factory: Arc<ChannelFactory>,
    service_context: ServiceContext,
    /// Dedicated session for all cron jobs — isolated from TUI sessions.
    cron_session_id: Option<Uuid>,
    /// Kept for API compatibility but no longer used for session resolution.
    #[allow(dead_code)]
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
}

impl CronScheduler {
    pub fn new(
        repo: CronJobRepository,
        run_repo: CronJobRunRepository,
        factory: Arc<ChannelFactory>,
        service_context: ServiceContext,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
    ) -> Self {
        Self {
            repo,
            run_repo,
            factory,
            service_context,
            cron_session_id: None,
            shared_session_id,
        }
    }

    /// Spawn the scheduler as a background tokio task.
    /// Polls every 60 seconds for due jobs.
    pub fn spawn(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Find or create the dedicated cron session
            match self.resolve_or_create_cron_session().await {
                Ok(id) => {
                    self.cron_session_id = Some(id);
                    tracing::info!(
                        "Cron scheduler started — polling every 60s, cron session: {}",
                        id
                    );
                }
                Err(e) => {
                    tracing::error!("Cron scheduler failed to create session: {e}");
                }
            }
            loop {
                if let Err(e) = self.tick().await {
                    tracing::error!("Cron scheduler tick error: {e}");
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        })
    }

    /// Find an existing "Cron" session or create one.
    async fn resolve_or_create_cron_session(&self) -> anyhow::Result<Uuid> {
        use crate::db::repository::SessionListOptions;
        let session_svc = SessionService::new(self.service_context.clone());
        // Look for an existing session named "Cron"
        let sessions = session_svc
            .list_sessions(SessionListOptions {
                include_archived: false,
                limit: None,
                offset: 0,
            })
            .await?;
        if let Some(existing) = sessions
            .iter()
            .find(|s| s.title.as_deref().is_some_and(|n| n == CRON_SESSION_NAME))
        {
            return Ok(existing.id);
        }
        // Create a new dedicated cron session
        let config = Config::load()?;
        let provider = config.cron.default_provider.clone();
        let model = config.cron.default_model.clone();
        let session = session_svc
            .create_session_with_provider(Some(CRON_SESSION_NAME.to_string()), provider, model)
            .await?;
        Ok(session.id)
    }

    /// One scheduler tick: check all enabled jobs and execute any that are due.
    async fn tick(&self) -> anyhow::Result<()> {
        let jobs = self.repo.list_enabled().await?;
        let now = Utc::now();

        for job in &jobs {
            if self.is_due(job, now) {
                tracing::info!("Cron job '{}' ({}) is due — executing", job.name, job.id);

                // Calculate next run time before executing (so we don't re-trigger)
                let next_run = self.next_run_after(job, now);
                let next_run_str = next_run.map(|dt| dt.to_rfc3339());
                self.repo
                    .update_last_run(&job.id.to_string(), next_run_str.as_deref())
                    .await?;

                // Execute in background so we don't block other jobs
                let Some(cron_sid) = self.cron_session_id else {
                    tracing::error!(
                        "Cron job '{}' — no cron session available, skipping",
                        job.name
                    );
                    continue;
                };
                let job = job.clone();
                let factory = self.factory.clone();
                let ctx = self.service_context.clone();
                let run_repo = self.run_repo.clone();
                tokio::spawn(async move {
                    if let Err(e) = execute_job(&job, &factory, &ctx, cron_sid, &run_repo).await {
                        tracing::error!("Cron job '{}' failed: {e}", job.name);
                    }
                });
            }
        }

        Ok(())
    }

    /// Check if a job is due to run.
    fn is_due(&self, job: &CronJob, now: chrono::DateTime<Utc>) -> bool {
        match &job.next_run_at {
            // If next_run_at is set and is in the past (or now), it's due
            Some(next) => *next <= now,
            // If next_run_at is None (first run), calculate from cron and check
            None => {
                // For first-time jobs, check if the current minute matches
                let cron_str = format!("0 {}", job.cron_expr);
                if let Ok(schedule) = Schedule::from_str(&cron_str) {
                    // If any upcoming time is within the next 60s, it's due
                    if let Some(next) = schedule.upcoming(Utc).next() {
                        let diff = next - now;
                        diff.num_seconds() <= 60
                    } else {
                        false
                    }
                } else {
                    tracing::warn!(
                        "Invalid cron expression for job '{}': {}",
                        job.name,
                        job.cron_expr
                    );
                    false
                }
            }
        }
    }

    /// Calculate the next run time after a given point.
    fn next_run_after(
        &self,
        job: &CronJob,
        after: chrono::DateTime<Utc>,
    ) -> Option<chrono::DateTime<Utc>> {
        let cron_str = format!("0 {}", job.cron_expr);
        Schedule::from_str(&cron_str)
            .ok()
            .and_then(|s| s.after(&after).next())
    }
}

/// Execute a single cron job in the shared cron session.
/// Isolated from TUI — never touches the user's active session.
/// Results are always stored in the DB; channel delivery is optional.
async fn execute_job(
    job: &CronJob,
    factory: &ChannelFactory,
    _ctx: &ServiceContext,
    cron_session_id: Uuid,
    run_repo: &CronJobRunRepository,
) -> anyhow::Result<()> {
    // Resolve effective provider/model: job override > config default > system default
    let config = Config::load()?;
    let effective_provider = job
        .provider
        .clone()
        .or_else(|| config.cron.default_provider.clone());
    let effective_model = job
        .model
        .clone()
        .or_else(|| config.cron.default_model.clone());

    // Create a run record in the DB (status = "running")
    let run = CronJobRun::new_running(
        job.id,
        job.name.clone(),
        effective_provider.clone(),
        effective_model.clone(),
    );
    let run_id = run.id.to_string();
    if let Err(e) = run_repo.insert(&run).await {
        tracing::error!("Failed to insert cron run record: {e}");
    }

    let session_id = cron_session_id;
    tracing::info!(
        "Cron job '{}' — using cron session {}",
        job.name,
        session_id
    );

    // Spawn agent service (inherits tools, brain, working dir from factory)
    let agent = factory.create_agent_service();

    // Swap to cron-specific provider if configured
    if let Some(ref provider_name) = effective_provider {
        match crate::brain::provider::create_provider_by_name(&config, provider_name) {
            Ok(provider) => {
                tracing::info!(
                    "Cron job '{}' — using provider '{}'",
                    job.name,
                    provider_name
                );
                agent.swap_provider(provider);
            }
            Err(e) => {
                tracing::warn!(
                    "Cron job '{}' — failed to create provider '{}': {e}, using system default",
                    job.name,
                    provider_name
                );
            }
        }
    }

    // Execute with auto-approved tools (no interactive user)
    let result = agent
        .send_message_with_tools_and_callback(
            session_id,
            job.prompt.clone(),
            effective_model,
            None, // no cancel token
            Some(Arc::new(|_| {
                // Auto-approve all tools for cron jobs
                Box::pin(async { Ok((true, false)) })
            })),
            None, // no progress callback
            "cron",
            None,
        )
        .await;

    match result {
        Ok(response) => {
            let clean = crate::utils::sanitize::strip_llm_artifacts(&response.content);

            tracing::info!(
                "Cron job '{}' completed — {} tokens, ${:.6}",
                job.name,
                response.usage.input_tokens + response.usage.output_tokens,
                response.cost
            );

            // Save result to DB
            if let Err(e) = run_repo
                .complete_success(
                    &run_id,
                    &clean,
                    response.usage.input_tokens as i64,
                    response.usage.output_tokens as i64,
                    response.cost,
                )
                .await
            {
                tracing::error!("Failed to save cron run result to DB: {e}");
            }

            // Optionally deliver to configured channels too
            if let Some(ref deliver_to) = job.deliver_to {
                for target in deliver_to
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    deliver_result(target, &job.name, &clean).await;
                }
            }
        }
        Err(e) => {
            tracing::error!("Cron job '{}' agent error: {e}", job.name);

            // Save error to DB
            let error_msg = format!("{e}");
            if let Err(db_err) = run_repo.complete_error(&run_id, &error_msg).await {
                tracing::error!("Failed to save cron run error to DB: {db_err}");
            }

            // Optionally deliver error to configured channels too
            if let Some(ref deliver_to) = job.deliver_to {
                let msg = format!("Cron job '{}' failed: {e}", job.name);
                for target in deliver_to
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    deliver_result(target, &job.name, &msg).await;
                }
            }
        }
    }

    Ok(())
}

/// Deliver a cron job result to the specified channel.
/// Format: "telegram:chat_id", "discord:channel_id", "slack:channel_id",
/// or an HTTP(S) URL for generic webhook delivery.
async fn deliver_result(deliver_to: &str, job_name: &str, content: &str) {
    // HTTP(S) URL — generic webhook delivery
    if deliver_to.starts_with("http://") || deliver_to.starts_with("https://") {
        deliver_http(deliver_to, job_name, content).await;
        return;
    }

    let parts: Vec<&str> = deliver_to.splitn(2, ':').collect();
    if parts.len() != 2 {
        tracing::warn!(
            "Invalid deliver_to format '{}' for job '{}' — expected 'channel:id' or HTTP URL",
            deliver_to,
            job_name
        );
        return;
    }

    let (channel, target_id) = (parts[0], parts[1]);

    // Truncate content for delivery (channels have message limits)
    let max_len = 4000;
    let msg = if content.len() > max_len {
        format!(
            "{}...\n\n(truncated — full output in session)",
            &content[..max_len]
        )
    } else {
        content.to_string()
    };

    let delivery_msg = format!("⏰ **Cron: {job_name}**\n\n{msg}");

    match channel {
        "telegram" => {
            #[cfg(feature = "telegram")]
            {
                tracing::info!("Delivering cron result to Telegram chat {target_id}");
                deliver_telegram(target_id, &delivery_msg).await;
            }
            #[cfg(not(feature = "telegram"))]
            {
                tracing::warn!("Telegram feature not enabled — cannot deliver cron result");
            }
        }
        "discord" => {
            tracing::info!("Delivering cron result to Discord channel {target_id}");
            tracing::warn!("Discord cron delivery not yet wired — result logged only");
        }
        "slack" => {
            tracing::info!("Delivering cron result to Slack channel {target_id}");
            tracing::warn!("Slack cron delivery not yet wired — result logged only");
        }
        other => {
            tracing::warn!("Unknown delivery channel '{other}' for job '{job_name}'");
        }
    }
}

/// Deliver cron result via HTTP POST to a generic webhook URL.
async fn deliver_http(url: &str, job_name: &str, content: &str) {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "job_name": job_name,
        "content": content,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    match client.post(url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!("Cron result for '{job_name}' delivered to {url}");
        }
        Ok(resp) => {
            tracing::warn!(
                "HTTP delivery to {url} failed ({}): {:?}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        Err(e) => {
            tracing::error!("HTTP delivery to {url} error: {e}");
        }
    }
}

/// Deliver via Telegram Bot API (direct HTTP POST).
#[cfg(feature = "telegram")]
async fn deliver_telegram(chat_id: &str, message: &str) {
    // We need the bot token — read from config
    let brain_path = crate::brain::BrainLoader::resolve_path();
    let keys_path = brain_path.join("keys.toml");
    let token = if let Ok(content) = std::fs::read_to_string(&keys_path) {
        content.parse::<toml::Table>().ok().and_then(|t| {
            t.get("channels")?
                .as_table()?
                .get("telegram")?
                .as_table()?
                .get("token")?
                .as_str()
                .map(String::from)
        })
    } else {
        None
    };

    let Some(token) = token else {
        tracing::warn!("No Telegram bot token found in keys.toml — cannot deliver cron result");
        return;
    };

    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": message,
        "parse_mode": "Markdown"
    });

    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!("Cron result delivered to Telegram chat {chat_id}");
        }
        Ok(resp) => {
            tracing::warn!(
                "Telegram delivery failed ({}): {:?}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        Err(e) => {
            tracing::error!("Telegram delivery HTTP error: {e}");
        }
    }
}
