//! Cron Scheduler
//!
//! Background task that checks the `cron_jobs` table every 60 seconds,
//! executes due jobs in the user's active session, and delivers results
//! to the configured channel. Never spawns new sessions — follows the
//! user's current session, falls back to the initial session at startup.

use crate::channels::ChannelFactory;
use crate::db::CronJobRepository;
use crate::db::models::CronJob;
use crate::services::{ServiceContext, SessionService};
use chrono::Utc;
use cron::Schedule;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Background cron scheduler that polls the database and executes due jobs.
pub struct CronScheduler {
    repo: CronJobRepository,
    factory: Arc<ChannelFactory>,
    service_context: ServiceContext,
    /// Shared reference to the user's currently active session in the TUI.
    /// For repeating crons, we follow the user to their current session.
    /// Falls back to `initial_session_id` if the user has no active session.
    shared_session_id: Arc<Mutex<Option<Uuid>>>,
    /// The session that was active when the scheduler was spawned.
    initial_session_id: Option<Uuid>,
}

impl CronScheduler {
    pub fn new(
        repo: CronJobRepository,
        factory: Arc<ChannelFactory>,
        service_context: ServiceContext,
        shared_session_id: Arc<Mutex<Option<Uuid>>>,
    ) -> Self {
        Self {
            repo,
            factory,
            service_context,
            shared_session_id,
            initial_session_id: None,
        }
    }

    /// Spawn the scheduler as a background tokio task.
    /// Polls every 60 seconds for due jobs.
    pub fn spawn(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Capture the session that was active when the scheduler started
            self.initial_session_id = *self.shared_session_id.lock().await;
            tracing::info!(
                "Cron scheduler started — polling every 60s, initial session: {:?}",
                self.initial_session_id
            );
            loop {
                if let Err(e) = self.tick().await {
                    tracing::error!("Cron scheduler tick error: {e}");
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        })
    }

    /// Resolve which session cron jobs should run in.
    /// Priority: user's current active session > initial session at scheduler start.
    async fn resolve_session_id(&self) -> Option<Uuid> {
        let current = *self.shared_session_id.lock().await;
        current.or(self.initial_session_id)
    }

    /// One scheduler tick: check all enabled jobs and execute any that are due.
    async fn tick(&self) -> anyhow::Result<()> {
        let jobs = self.repo.list_enabled().await?;
        let now = Utc::now();

        // Resolve session once per tick — all jobs in this tick share it
        let session_id = self.resolve_session_id().await;

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
                let job = job.clone();
                let factory = self.factory.clone();
                let ctx = self.service_context.clone();
                tokio::spawn(async move {
                    if let Err(e) = execute_job(&job, &factory, &ctx, session_id).await {
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

/// Execute a single cron job in the user's current session.
/// Falls back to creating a session only if no active session exists.
async fn execute_job(
    job: &CronJob,
    factory: &ChannelFactory,
    ctx: &ServiceContext,
    target_session_id: Option<Uuid>,
) -> anyhow::Result<()> {
    let session_svc = SessionService::new(ctx.clone());

    let session_id = if let Some(id) = target_session_id {
        // Verify session still exists
        if session_svc.get_session(id).await?.is_some() {
            tracing::info!("Cron job '{}' — using active session {}", job.name, id);
            id
        } else {
            // Session was deleted — fall back to most recent
            let fallback = session_svc.get_most_recent_session().await?;
            match fallback {
                Some(s) => {
                    tracing::info!(
                        "Cron job '{}' — target session gone, using most recent {}",
                        job.name,
                        s.id
                    );
                    s.id
                }
                None => {
                    // No sessions at all — create one as last resort
                    let s = session_svc
                        .create_session_with_provider(
                            Some(format!("Cron: {}", job.name)),
                            job.provider.clone(),
                            job.model.clone(),
                        )
                        .await?;
                    tracing::warn!(
                        "Cron job '{}' — no sessions found, created fallback {}",
                        job.name,
                        s.id
                    );
                    s.id
                }
            }
        }
    } else {
        // No shared session yet (app just started?) — try most recent
        let fallback = session_svc.get_most_recent_session().await?;
        match fallback {
            Some(s) => {
                tracing::info!(
                    "Cron job '{}' — no active session, using most recent {}",
                    job.name,
                    s.id
                );
                s.id
            }
            None => {
                let s = session_svc
                    .create_session_with_provider(
                        Some(format!("Cron: {}", job.name)),
                        job.provider.clone(),
                        job.model.clone(),
                    )
                    .await?;
                tracing::warn!(
                    "Cron job '{}' — no sessions found, created fallback {}",
                    job.name,
                    s.id
                );
                s.id
            }
        }
    };

    // Spawn agent service (inherits tools, brain, working dir from factory)
    let agent = factory.create_agent_service();

    // Execute with auto-approved tools (no interactive user)
    let result = agent
        .send_message_with_tools_and_callback(
            session_id,
            job.prompt.clone(),
            job.model.clone(),
            None, // no cancel token
            Some(Arc::new(|_| {
                // Auto-approve all tools for cron jobs
                Box::pin(async { Ok((true, false)) })
            })),
            None, // no progress callback
        )
        .await;

    match result {
        Ok(response) => {
            tracing::info!(
                "Cron job '{}' completed — {} tokens, ${:.6}",
                job.name,
                response.usage.input_tokens + response.usage.output_tokens,
                response.cost
            );

            // Deliver results to channel if configured
            if let Some(ref deliver_to) = job.deliver_to {
                deliver_result(deliver_to, &job.name, &response.content).await;
            }
        }
        Err(e) => {
            tracing::error!("Cron job '{}' agent error: {e}", job.name);
            // Deliver error to channel if configured
            if let Some(ref deliver_to) = job.deliver_to {
                deliver_result(
                    deliver_to,
                    &job.name,
                    &format!("Cron job '{}' failed: {e}", job.name),
                )
                .await;
            }
        }
    }

    Ok(())
}

/// Deliver a cron job result to the specified channel.
/// Format: "telegram:chat_id", "discord:channel_id", "slack:channel_id"
async fn deliver_result(deliver_to: &str, job_name: &str, content: &str) {
    let parts: Vec<&str> = deliver_to.splitn(2, ':').collect();
    if parts.len() != 2 {
        tracing::warn!(
            "Invalid deliver_to format '{}' for job '{}' — expected 'channel:id'",
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
                // Use the Telegram Bot API directly for delivery
                // The bot instance is shared via TelegramState, but for cron we use a simple HTTP call
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
            // Discord delivery requires the bot's HTTP client from DiscordState
            // For now, log — will be wired when Discord state is accessible
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
