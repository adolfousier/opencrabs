//! Cron Scheduler
//!
//! Background service that polls the `cron_jobs` table every 60 seconds and
//! executes due jobs in the user's active session. Never spawns new sessions —
//! follows the user, falls back to initial session. Results are optionally
//! delivered to a configured channel (Telegram, Discord, Slack).

mod scheduler;

pub use scheduler::CronScheduler;
