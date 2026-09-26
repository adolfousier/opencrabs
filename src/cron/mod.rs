//! Cron Scheduler
//!
//! Background service that polls the `cron_jobs` table every 60 seconds and
//! executes due jobs in the user's active session. Never spawns new sessions —
//! follows the user, falls back to initial session. Results are optionally
//! delivered to a configured channel (Telegram, Discord, Slack).

pub mod pipeline;
mod schedule_util;
pub(crate) mod scheduler;
pub mod send_scope;
pub mod trigger;

pub use pipeline::{PipelineExecutor, TriggerOutcome, interpolate_template};
pub use schedule_util::{format_upcoming, next_run_utc, parse_timezone, upcoming_in_tz};
pub use scheduler::CronScheduler;
pub use trigger::{TriggerCondition, TriggerResult, TriggerRunner};
