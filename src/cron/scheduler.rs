//! Cron Scheduler
//!
//! Background task that checks the `cron_jobs` table every 60 seconds,
//! executes due jobs in a shared "Cron" session, and delivers results
//! to the configured channel. Each run inserts a compaction marker after
//! completion so the next run starts with empty context (no cross-job
//! history contamination). Cron jobs are fully isolated from the TUI —
//! they never share or mutate the user's active session.

use crate::channels::ChannelFactory;
use crate::config::Config;
use crate::db::CronJobRepository;
use crate::db::CronJobRunRepository;
use crate::db::models::{CronJob, CronJobRun};
use crate::db::repository::CronJobPatch;
use crate::services::{ServiceContext, SessionService};
use chrono::Utc;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

/// Whether `job_profile` is the active process profile (so the cheap, already
/// wired factory agent can run it) rather than a foreign profile that needs its
/// own config + brain materialized. `None` = legacy pre-stamping row, treated as
/// the active profile. The base profile is stored as the literal "default".
fn is_active_profile(job_profile: Option<&str>, active: Option<&str>) -> bool {
    match job_profile {
        None => true,
        Some(stamped) => stamped == active.unwrap_or("default"),
    }
}

/// Reserved cron-job name for a one-shot background `/rebuild`. The scheduler
/// special-cases this name: instead of running an agent prompt it builds from
/// source and exec-restarts into the new binary, then the job removes itself.
/// The originating session id is carried in `prompt` so the restart resumes
/// the user's session.
pub const REBUILD_JOB_NAME: &str = "__opencrabs_rebuild__";

/// Reserved recurring job: weekly cross-file brain dedup scan (#765). Unlike
/// the one-shot rebuild job, this one is NOT self-deleting — it fires every
/// week as a safety net that catches cross-file drift the event-based trigger
/// (brain writes) and the manual `/dedup` command can miss (template sync,
/// manual edits, preamble changes). Report-only: files proposals, never applies.
pub const DEDUP_SCAN_JOB_NAME: &str = "__opencrabs_dedup_scan__";

/// Weekly dedup scan: Sunday 04:00 UTC (quiet window).
///
/// Weekday is 1 = Sunday in the `cron` crate, NOT the Unix 0 (see
/// `cron_schedule_util_test::numeric_dow_is_sunday_first`). This job shipped
/// as `0 4 * * 0`, which the crate rejects outright, so it never ran once and
/// logged a parse failure every minute for its whole lifetime — the cross-file
/// dedup scan that catches the same rule written into two brain files was dead
/// the entire time (#1024).
///
/// Do NOT "fix" a Unix-style 0 by translating it to 7: 7 is Saturday under this
/// numbering, so the job would run on the wrong day, silently — worse than not
/// running. Rejecting 0 is deliberate (`cron_schedule_util_test::dow_zero_is_rejected`).
pub(crate) const DEDUP_SCAN_CRON: &str = "0 4 * * 1";

/// The pre-#1024 artifact this job originally shipped as: Unix-style dow 0,
/// which the `cron` crate rejects outright (see [`DEDUP_SCAN_CRON`] docs).
/// Rows still holding EXACTLY this value are repaired at startup (#1163).
/// Any other expression — including other invalid ones — is treated as
/// deliberate and never touched.
pub(crate) const LEGACY_DEDUP_SCAN_CRON: &str = "0 4 * * 0";

/// Warn-once guard for unparseable cron expressions (#1163): without it an
/// invalid row warns on every ~60s tick — 1,440 warns/day for a job that
/// never runs. One warning per process is enough to diagnose.
static INVALID_EXPR_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Schedule a one-shot background rebuild for `session_id`. Returns once the
/// job is queued — the build runs out-of-band on the scheduler's next tick
/// (within ~60s), so the calling session is never blocked. `deliver_to` (if
/// set) receives a status message; the reload resumes `session_id`.
pub async fn schedule_background_rebuild(
    pool: crate::db::Pool,
    session_id: Uuid,
    deliver_to: Option<String>,
) -> anyhow::Result<()> {
    let repo = CronJobRepository::new(pool);
    // Remove any stale rebuild job first so we never stack two builds.
    if let Ok(existing) = repo.list_all().await {
        for j in existing.iter().filter(|j| j.name == REBUILD_JOB_NAME) {
            if let Err(e) = repo.delete(&j.id.to_string()).await {
                tracing::warn!(error = %e, job_id = %j.id, "failed to delete cron job");
            }
        }
    }
    let now = Utc::now();
    let job = CronJob {
        id: Uuid::new_v4(),
        name: REBUILD_JOB_NAME.to_string(),
        // Every minute → the next tick (within 60s) picks it up; the job
        // deletes itself on pickup so it runs exactly once.
        cron_expr: "* * * * *".to_string(),
        timezone: "UTC".to_string(),
        prompt: session_id.to_string(),
        provider: None,
        model: None,
        thinking: "off".to_string(),
        auto_approve: true,
        deliver_to,
        deliver_api_key: None,
        enabled: true,
        last_run_at: None,
        next_run_at: None,
        created_at: now,
        updated_at: now,
        // Stamp the current profile so the guard in `tick()` lets it run here.
        // current_profile_name() honors the task-local profile scope.
        profile_name: Some(crate::config::profile::current_profile_name()),
    };
    repo.insert(&job).await?;
    tracing::info!("Background rebuild queued for session {session_id}");
    Ok(())
}

/// Idempotently seed the reserved weekly brain-dedup scan job (#765). A no-op
/// if a job named [`DEDUP_SCAN_JOB_NAME`] already exists, so repeated scheduler
/// starts never stack duplicate jobs. The job runs the scanner directly (see
/// [`run_dedup_scan_job`]) every Sunday at 04:00 UTC.
pub(crate) async fn ensure_weekly_dedup_scan_job(repo: &CronJobRepository) -> anyhow::Result<()> {
    if let Ok(existing) = repo.list_all().await
        && let Some(job) = existing.iter().find(|j| j.name == DEDUP_SCAN_JOB_NAME)
    {
        // Repair-in-place (#1163): installs that seeded before #1024 hold
        // LEGACY_DEDUP_SCAN_CRON, which this parser rejects outright. The
        // name-idempotent early-return used to make that row immortal:
        // dead job plus one warn per minute for life. Rewrite ONLY rows
        // holding that exact legacy artifact; every other expression
        // (including user-customized schedules) is deliberate and stays.
        if job.cron_expr == LEGACY_DEDUP_SCAN_CRON {
            let patch = CronJobPatch {
                cron_expr: Some(DEDUP_SCAN_CRON.to_string()),
                reset_next_run: true,
                ..Default::default()
            };
            match repo.update_fields(&job.id.to_string(), patch).await {
                Ok(true) => tracing::info!(
                    "Repaired legacy dedup-scan schedule '{}' -> '{}' (#1163)",
                    LEGACY_DEDUP_SCAN_CRON,
                    DEDUP_SCAN_CRON
                ),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "failed to repair legacy dedup-scan schedule")
                }
            }
        }
        return Ok(());
    }
    let job = CronJob::new(
        DEDUP_SCAN_JOB_NAME.to_string(),
        DEDUP_SCAN_CRON.to_string(),
        "UTC".to_string(),
        // Reserved job — the prompt is never run as an agent turn; kept as a
        // human-readable descriptor only.
        "reserved: weekly cross-file brain dedup scan (report-only)".to_string(),
        None,
        None,
        "off".to_string(),
        true,
        // Deliver the interactive approval keyboard to the dev group so pending
        // cross-file duplicates can be approved or rejected in-chat (#765).
        Some("telegram:-1002554690655".to_string()),
        None,
    );
    repo.insert(&job).await?;
    tracing::info!("Seeded weekly brain dedup scan job ({DEDUP_SCAN_JOB_NAME})");
    Ok(())
}

/// Execute the reserved weekly brain-dedup scan (#765): run the cross-file
/// scanner directly against this profile's brain dir (no agent prompt, no LLM
/// cost), filing report-only proposals into the Mission Control inbox. Never
/// applies — merging across files changes enforcement scope and needs human
/// approval. Runs inline (the scan touches a handful of small `.md` files) so
/// it stays inside the per-profile home scope the scheduler's spawned task set,
/// keeping `opencrabs_home()` pointed at the right profile. Delivers a summary
/// to the job's channel only when something is pending, so a clean tree doesn't
/// spam weekly.
async fn run_dedup_scan_job(job: &CronJob) -> anyhow::Result<()> {
    let brain_dir = crate::config::opencrabs_home();
    let store = crate::brain::rsi_proposals::ProposalsStore::new();
    store.prune_handled();
    let filed = crate::brain::dedup_scan::file_dedup_proposals(&brain_dir, &store);
    let pending = store.list_brain_dedup_proposals().len();
    tracing::info!(
        "Weekly brain dedup scan complete: {filed} new proposal(s), {pending} pending total"
    );
    if pending > 0 {
        let msg = format!(
            "🧹 Weekly brain dedup scan: {pending} pending cross-file duplicate proposal(s) \
             ({filed} new this run). Review and approve in the Mission Control inbox."
        );
        // deliver_rebuild_status is the generic per-channel delivery helper
        // (no-op when the job has no deliver_to); the name is rebuild-specific
        // but the body just fans `msg` out to the job's configured channels.
        deliver_rebuild_status(job, &msg).await;
        // Follow the text summary with an interactive approval keyboard so the
        // dev group can apply or reject the pending duplicates in-chat (#765).
        // No-op when the job has no telegram target or the feature is off.
        send_dedup_approval_keyboard(job).await;
    }
    Ok(())
}

/// Send the interactive brain-dedup approval keyboard to the job's Telegram
/// target (#765). Parses the chat id from `deliver_to` (format
/// `telegram:<chat_id>`), builds a bot from the keys.toml token, and posts the
/// pending proposals with inline Approve/Reject buttons. Removals only happen
/// on an explicit button tap; this just surfaces the pending list. Silent no-op
/// when the job has no telegram target, the token is missing, or nothing filed.
#[cfg(feature = "telegram")]
async fn send_dedup_approval_keyboard(job: &CronJob) {
    // Pull the first `telegram:<chat_id>` target out of the (possibly
    // comma-separated) deliver_to list. A thread component (#104) is
    // accepted by the grammar but the approval keyboard itself is
    // chat-level — it goes to the chat's default topic.
    let Some((chat_id, _thread)) = job.deliver_to.as_deref().and_then(|targets| {
        targets
            .split(',')
            .map(str::trim)
            .find_map(|t| t.strip_prefix("telegram:"))
            .and_then(parse_telegram_target)
    }) else {
        return;
    };
    let Some(token) = read_channel_secret("telegram", "token") else {
        tracing::warn!("No Telegram bot token in keys.toml, cannot send dedup approval keyboard");
        return;
    };
    let bot = teloxide::Bot::new(token);
    match crate::channels::telegram::dedup_approval::send_approval_request(
        &bot,
        teloxide::types::ChatId(chat_id),
    )
    .await
    {
        Ok(0) => {}
        Ok(n) => tracing::info!(
            "Sent dedup approval keyboard to Telegram chat {chat_id} ({n} proposal(s))"
        ),
        Err(e) => tracing::warn!("Failed to send dedup approval keyboard to {chat_id}: {e}"),
    }
}

/// Non-telegram builds have no keyboard to send; keep the call site unconditional.
#[cfg(not(feature = "telegram"))]
async fn send_dedup_approval_keyboard(_job: &CronJob) {}

/// Execute the reserved background-rebuild job: delete it first (one-shot, no
/// retry on the 60s tick), build from source, then exec-restart into the
/// freshly-built binary (replaces the whole process). On failure it reports
/// to `deliver_to` and returns. The originating session id is in `job.prompt`.
async fn run_rebuild_job(
    job: &CronJob,
    ctx: &ServiceContext,
    session_notifier: Option<&SessionNotifier>,
) -> anyhow::Result<()> {
    use crate::brain::SelfUpdater;

    // Delete up front so a long/failed build can't re-trigger next tick.
    let repo = CronJobRepository::new(ctx.pool());
    if let Err(e) = repo.delete(&job.id.to_string()).await {
        tracing::error!("rebuild job: failed to delete self: {e}");
    }

    let session_id = Uuid::parse_str(job.prompt.trim()).unwrap_or_else(|_| Uuid::nil());
    tracing::info!("Background rebuild starting (will resume session {session_id})");

    let updater =
        SelfUpdater::auto_detect().map_err(|e| anyhow::anyhow!("rebuild: auto_detect: {e}"))?;

    match updater
        .build_streaming(|line| tracing::debug!("rebuild: {line}"))
        .await
    {
        Ok(built_path) => {
            tracing::info!(
                "Background rebuild succeeded: {} — reloading",
                built_path.display()
            );
            let handles = deliver_rebuild_status(
                job,
                "✅ Rebuilt from source — reloading into the new binary now.",
            )
            .await;
            // Await all delivery tasks before exec() replaces the process (#1105).
            // Without this, the detached Telegram send is killed mid-flight and
            // the completion message never arrives.
            if !handles.is_empty() {
                tracing::info!(
                    "Awaiting {} delivery handle(s) before exec()",
                    handles.len()
                );
                futures::future::join_all(handles).await;
            }
            // Persist the completion report to the session DB so the agent
            // sees it on the next turn after the exec restart (#1105).
            // Without this, the hot-reload wake-up message is orphaned —
            // the agent responds but has no context about what triggered it.
            if !session_id.is_nil() {
                let msg_svc = crate::services::MessageService::new(ctx.clone());
                let report = format!(
                    "✅ Background rebuild succeeded — binary at {}. Hot-reloading now.",
                    built_path.display()
                );
                match msg_svc
                    .create_message(session_id, "assistant".to_string(), report)
                    .await
                {
                    Ok(_) => tracing::info!(
                        "Persisted rebuild completion report to session {session_id}"
                    ),
                    Err(e) => tracing::error!("Failed to persist rebuild completion report: {e}"),
                }
            }
            // exec() replaces the entire process (this scheduler task too).
            if let Err(e) = SelfUpdater::restart_into(&built_path, session_id) {
                tracing::error!("Background rebuild restart failed: {e}");
                return Err(anyhow::anyhow!("rebuild restart failed: {e}"));
            }
            Ok(()) // unreachable on success
        }
        Err(out) => {
            tracing::error!("Background rebuild failed: {out}");
            let msg = format!("⚠️ Background rebuild failed:\n{out}");
            // TUI (#304): surface the failure in the session that asked. It
            // was told "reloading automatically when ready" and would
            // otherwise wait forever on a log-only error.
            if let Some(notify) = session_notifier {
                notify(session_id, msg.clone());
            }
            let _ = deliver_rebuild_status(job, &msg).await; // detached — no exec follows
            Ok(())
        }
    }
}

/// Deliver a rebuild status line to the job's configured channels (if any).
/// Returns spawn handles so the caller can await delivery before exec() (#1105).
async fn deliver_rebuild_status(job: &CronJob, msg: &str) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    if let Some(ref deliver_to) = job.deliver_to {
        for target in deliver_to
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            // Rebuild status messages aren't worth reply recovery — no pool.
            if let Some(h) =
                deliver_result(target, &job.name, msg, job.deliver_api_key.as_deref(), None).await
            {
                handles.push(h);
            }
        }
    }
    handles
}

/// Callback for surfacing scheduler events into a live session UI (the TUI).
/// Args: originating session id, message text. Daemon callers run without one.
pub type SessionNotifier = Arc<dyn Fn(Uuid, String) + Send + Sync>;

/// Background cron scheduler that polls the database and executes due jobs.
pub struct CronScheduler {
    repo: CronJobRepository,
    run_repo: CronJobRunRepository,
    factory: Arc<ChannelFactory>,
    service_context: ServiceContext,
    /// Surfaces rebuild outcomes into the originating TUI session (#304):
    /// without it a failed background build was visible only in the log
    /// while the user waited for a reload that would never come.
    session_notifier: Option<SessionNotifier>,
}

impl CronScheduler {
    pub fn new(
        repo: CronJobRepository,
        run_repo: CronJobRunRepository,
        factory: Arc<ChannelFactory>,
        service_context: ServiceContext,
    ) -> Self {
        Self {
            repo,
            run_repo,
            factory,
            service_context,
            session_notifier: None,
        }
    }

    /// Wire a live-session notifier (TUI mode). Daemon callers skip this.
    pub fn with_session_notifier(mut self, notifier: SessionNotifier) -> Self {
        self.session_notifier = Some(notifier);
        self
    }

    /// Spawn the scheduler as a background tokio task.
    /// Polls every 60 seconds for due jobs.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.run())
    }

    /// Run the polling loop in the CURRENT task (no internal spawn). The
    /// multi-profile daemon drives this directly inside a
    /// `with_profile_home_async(profile, ...)` scope so the scheduler's own
    /// setup (cron session, config reads) resolves to that profile's home.
    /// `spawn()` is the thin wrapper for callers that just want it backgrounded.
    pub async fn run(self) {
        tracing::info!(
            "Cron scheduler started — polling every 60s (shared Cron session, compaction-isolated)"
        );
        // Seed the reserved weekly brain-dedup safety-net job once per scheduler
        // start (#765). Idempotent: a no-op if the job already exists. Runs in
        // the per-profile home scope (daemon case), so the job is stamped with
        // the correct profile.
        if let Err(e) = ensure_weekly_dedup_scan_job(&self.repo).await {
            tracing::warn!("Failed to seed weekly brain dedup scan job: {e}");
        }

        loop {
            if let Err(e) = self.tick().await {
                tracing::error!("Cron scheduler tick error: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    }

    /// One scheduler tick: check all enabled jobs and execute any that are due.
    async fn tick(&self) -> anyhow::Result<()> {
        let jobs = self.repo.list_enabled().await?;
        let now = Utc::now();

        for job in &jobs {
            if self.is_due(job, now) {
                tracing::info!("Cron job '{}' ({}) is due — executing", job.name, job.id);

                // Calculate next run time before executing (so we don't re-trigger).
                // Use the scheduled boundary as anchor, not `now`: for a
                // first-run job (next_run_at = None), `now` may be 16s before
                // the boundary (e.g. 10:59:44 vs 11:00:00). Passing `now`
                // resolves to the SAME boundary, causing a double-fire next tick.
                // (#224)
                let next_run = match job.next_run_at {
                    Some(_) => self.next_run_after(job, now),
                    None => match super::next_run_utc(&job.cron_expr, job_tz(job), now) {
                        Some(boundary) => self.next_run_after(job, boundary),
                        None => None,
                    },
                };
                let next_run_str = next_run.map(|dt| dt.to_rfc3339());
                self.repo
                    .update_last_run(&job.id.to_string(), next_run_str.as_deref())
                    .await?;

                // Execute in background so we don't block other jobs
                let job = job.clone();
                let factory = self.factory.clone();
                let ctx = self.service_context.clone();
                let run_repo = self.run_repo.clone();
                let notifier = self.session_notifier.clone();
                let job_name = job.name.clone();
                let job_id = job.id;
                tokio::spawn(
                    async move {
                        // For foreign-profile jobs, wrap the ENTIRE execution in a
                        // task-local profile home scope. This means every tool call
                        // the agent makes (memory writes, config reads, file ops,
                        // brain reads) resolves to the job's profile home, not the
                        // process profile. The scope lives until the task ends, so
                        // it persists across every .await inside the agent loop.
                        //
                        // This spawned task does NOT inherit the scheduler's own
                        // task-local home (tokio::spawn drops it), so it defaults to
                        // the process global. We therefore scope whenever the job's
                        // profile differs from the process global, which is exactly
                        // the multi-profile daemon case: a per-profile scheduler's
                        // jobs are stamped with a non-global profile and get scoped
                        // here.
                        let profile = job.profile_name.as_deref();
                        let active = crate::config::profile::active_profile().unwrap_or("default");
                        let needs_scope = profile.is_some() && profile != Some(active);

                        let result = if needs_scope {
                            crate::config::profile::with_profile_home_async(profile, async {
                                tracing::info!(
                                    "Cron job '{}' — task-local profile home set to {:?}",
                                    job.name,
                                    crate::config::opencrabs_home()
                                );
                                match resolve_or_create_cron_session(&ctx).await {
                                    Ok(cron_sid) => {
                                        execute_job(
                                            &job,
                                            &factory,
                                            &ctx,
                                            cron_sid,
                                            &run_repo,
                                            notifier.as_ref(),
                                        )
                                        .await
                                    }
                                    Err(e) => Err(e),
                                }
                            })
                            .await
                        } else {
                            match resolve_or_create_cron_session(&ctx).await {
                                Ok(cron_sid) => {
                                    execute_job(
                                        &job,
                                        &factory,
                                        &ctx,
                                        cron_sid,
                                        &run_repo,
                                        notifier.as_ref(),
                                    )
                                    .await
                                }
                                Err(e) => Err(e),
                            }
                        };

                        if let Err(e) = result {
                            tracing::error!("Cron job '{}' failed: {e}", job.name);
                        }
                    }
                    .instrument(tracing::info_span!("job", name = %job_name, id = %job_id)),
                );
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
                // Interpret the schedule in the job's timezone (DST-aware),
                // then compare the resulting UTC instant. If any upcoming run
                // is within the next 60s (one tick), it's due.
                match super::next_run_utc(&job.cron_expr, job_tz(job), now) {
                    Some(next) => (next - now).num_seconds() <= 60,
                    None => {
                        // Warn once per process (#1163): this arm fires on
                        // every ~60s tick otherwise, so one bad row produced
                        // 1,440 identical warns/day.
                        if !INVALID_EXPR_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                            tracing::warn!(
                                "Invalid cron expression for job '{}': {} (suppressing further warnings until restart)",
                                job.name,
                                job.cron_expr
                            );
                        }
                        false
                    }
                }
            }
        }
    }

    /// Calculate the next run time after a given point, in the job's timezone.
    fn next_run_after(
        &self,
        job: &CronJob,
        after: chrono::DateTime<Utc>,
    ) -> Option<chrono::DateTime<Utc>> {
        super::next_run_utc(&job.cron_expr, job_tz(job), after)
    }
}

/// Find an existing "Cron" session or create one. All cron jobs share this
/// session for logging/debugging, but each run inserts a compaction marker
/// after completion so the next run starts with empty context (no history
/// contamination between jobs).
async fn resolve_or_create_cron_session(ctx: &ServiceContext) -> anyhow::Result<Uuid> {
    const CRON_SESSION_NAME: &str = "Cron";
    use crate::db::repository::SessionListOptions;
    let session_svc = SessionService::new(ctx.clone());
    let sessions = session_svc
        .list_sessions(SessionListOptions {
            include_archived: false,
            limit: None,
            offset: 0,
            query: None,
            include_subagents: false,
        })
        .await?;
    if let Some(existing) = sessions
        .iter()
        .find(|s| s.title.as_deref().is_some_and(|n| n == CRON_SESSION_NAME))
    {
        return Ok(existing.id);
    }
    let config = Config::load()?;
    let provider = config.cron.default_provider.clone();
    let model = config.cron.default_model.clone();
    let session = session_svc
        .create_session_with_provider(Some(CRON_SESSION_NAME.to_string()), provider, model, None)
        .await?;
    Ok(session.id)
}

/// Resolve a job's stored timezone string to a `Tz`, falling back to UTC for
/// an unknown zone (the tool/CLI reject unknown zones at creation, so this is
/// just a safety net for hand-edited rows).
fn job_tz(job: &CronJob) -> chrono_tz::Tz {
    super::parse_timezone(&job.timezone).unwrap_or(chrono_tz::UTC)
}

/// Resolve the `(Config, AgentService)` a job should run with.
///
/// Jobs created in the active profile (and legacy unstamped jobs) use the
/// already-wired factory agent. A job stamped with a DIFFERENT profile
/// (shared-DB case, #182) gets its own config + brain + provider built from
/// that profile's home. The shared DB pool is reused since that's exactly why
/// the foreign job is visible to this scheduler at all.
async fn resolve_job_agent(
    job: &CronJob,
    factory: &ChannelFactory,
    ctx: &ServiceContext,
) -> anyhow::Result<(Config, Arc<crate::brain::agent::AgentService>)> {
    // Use the task-local profile (set by the per-job home scope above) rather
    // than the process global, so a job running under its own profile scope
    // is recognized as "local" and reuses the in-scope factory/config instead
    // of needlessly re-materializing. Falls back to the global when unscoped.
    let current = crate::config::profile::current_profile_name();
    if is_active_profile(job.profile_name.as_deref(), Some(&current)) {
        return Ok((Config::load()?, factory.create_agent_service().await));
    }

    let profile = job.profile_name.as_deref();
    tracing::info!(
        "Cron job '{}' belongs to profile {:?} (current profile {:?}); \
         running under its own profile context",
        job.name,
        profile,
        current
    );

    // Materialize config + brain from the foreign profile's home.
    // with_profile_home sets a sync scope just for these two loads.
    let (config, brain, home) = crate::config::profile::with_profile_home(profile, || {
        let config = Config::load()?;
        let home = crate::config::opencrabs_home();
        let brain =
            crate::brain::prompt_builder::BrainLoader::new(home.clone()).build_core_brain(None);
        anyhow::Ok((config, brain, home))
    })?;

    // Provider is built from the foreign profile's keys.
    let provider = crate::brain::provider::create_provider(&config).await?;
    let mut builder = crate::brain::agent::AgentService::new(provider, ctx.clone(), &config)
        .await
        .with_system_brain(brain)
        .with_working_directory(home.clone())
        .with_brain_path(home);
    if let Some(registry) = factory.tool_registry() {
        builder = builder.with_tool_registry(registry);
    }
    // #129: cron execution is headless — backstop flag on the agent so
    // interactive-only tools hard-error even if re-registered.
    builder = builder.with_headless(true);
    Ok((config, Arc::new(builder)))
}

/// Execute a single cron job in its own isolated session.
/// Isolated from TUI — never touches the user's active session.
/// Results are always stored in the DB; channel delivery is optional.
async fn execute_job(
    job: &CronJob,
    factory: &ChannelFactory,
    ctx: &ServiceContext,
    cron_session_id: Uuid,
    run_repo: &CronJobRunRepository,
    session_notifier: Option<&SessionNotifier>,
) -> anyhow::Result<()> {
    // Reserved one-shot background rebuild — build + exec-restart, never an
    // agent prompt.
    if job.name == REBUILD_JOB_NAME {
        return run_rebuild_job(job, ctx, session_notifier).await;
    }

    // Reserved weekly cross-file brain dedup scan (#765) — runs the scanner
    // directly (no agent prompt, no LLM cost) and files report-only proposals.
    if job.name == DEDUP_SCAN_JOB_NAME {
        return run_dedup_scan_job(job).await;
    }

    // Resolve the config + agent for this job's profile. A job created in a
    // non-active profile (shared-DB case, #182) runs under its own profile's
    // config + brain, not the process profile's.
    let (config, agent) = resolve_job_agent(job, factory, ctx).await?;
    let effective_provider = job
        .provider
        .clone()
        .or_else(|| config.cron.default_provider.clone());
    let effective_model = job
        .model
        .clone()
        .or_else(|| config.cron.default_model.clone());

    // Pre-validate the {provider, model} pair before spawning the agent.
    // A reversed cron config (e.g. model="zhipu", provider="glm-5.1") or a
    // typo would otherwise reach the tool loop and produce confusing RSI
    // entries like "dialagram/zhipu" where a provider name leaked into the
    // model slot. Catch it early, log loudly, skip the job.
    if let Some(ref provider_name) = effective_provider
        && let Some(ref model) = effective_model
    {
        match crate::brain::provider::create_provider_by_name(&config, provider_name).await {
            Ok(provider) => {
                let supported = provider.supported_models();
                if !supported.is_empty() && !supported.iter().any(|m| m == model) {
                    tracing::error!(
                        "Cron job '{}' — model '{}' is NOT supported by provider '{}' \
                         (supported: {}). SKIPPING job — fix cron config. \
                         Either set a valid model or remove the model override to use \
                         the provider's default ('{}').",
                        job.name,
                        model,
                        provider_name,
                        supported.join(", "),
                        provider.default_model(),
                    );
                    // Record the failure so RSI surfaces it
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
                    let err_msg = format!(
                        "model '{}' not supported by provider '{}' — cron config invalid",
                        model, provider_name
                    );
                    if let Err(db_err) = run_repo.complete_error(&run_id, &err_msg).await {
                        tracing::error!("Failed to save cron run error to DB: {db_err}");
                    }
                    return Ok(());
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Cron job '{}' — cannot pre-validate model (provider '{}' creation \
                     failed: {e}) — proceeding with default validation",
                    job.name,
                    provider_name
                );
            }
        }
    }

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

    // Swap to cron-specific provider if configured
    if let Some(ref provider_name) = effective_provider {
        match crate::brain::provider::create_provider_by_name(&config, provider_name).await {
            Ok(provider) => {
                tracing::info!(
                    "Cron job '{}' — using provider '{}'",
                    job.name,
                    provider_name
                );
                agent.swap_provider_for_session(
                    cron_session_id,
                    provider.clone(),
                    provider.default_model().to_string(),
                );
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

    // Permitted destinations for this turn, taken from the job's own
    // configuration (#148: authority-aware across telegram, discord, slack,
    // whatsapp). A job with no `deliver_to` may send to none: its output
    // lives in its session and the scheduler is the only thing that speaks.
    // Scoped across the whole turn so it holds inside every tool call, and
    // task-local so it never reaches a sibling job on the scheduler.
    let permitted_targets: Option<Vec<crate::cron::send_scope::PermittedTarget>> = job
        .deliver_to
        .as_deref()
        .map(|targets| {
            targets
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .filter_map(|t| {
                    if let Some(rest) = t.strip_prefix("telegram:") {
                        parse_telegram_target(rest)
                            .map(|(chat_id, _)| crate::cron::send_scope::PermittedTarget {
                                channel: "telegram",
                                target_id: chat_id.to_string(),
                            })
                    } else if let Some(rest) = t.strip_prefix("discord:") {
                        Some(crate::cron::send_scope::PermittedTarget {
                            channel: "discord",
                            target_id: rest.to_string(),
                        })
                    } else if let Some(rest) = t.strip_prefix("slack:") {
                        Some(crate::cron::send_scope::PermittedTarget {
                            channel: "slack",
                            target_id: rest.to_string(),
                        })
                    } else {
                        t.strip_prefix("whatsapp:").map(|rest| {
                            crate::cron::send_scope::PermittedTarget {
                                channel: "whatsapp",
                                target_id: rest.to_string(),
                            }
                        })
                    }
                })
                .collect()
        });

    // Execute with auto-approved tools (no interactive user)
    let result = crate::cron::send_scope::with_permitted_targets(
        permitted_targets,
        agent.send_message_with_tools_and_callback(
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
            None,
        ),
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
                    let _ = deliver_result(
                        target,
                        &job.name,
                        &clean,
                        job.deliver_api_key.as_deref(),
                        Some(ctx.pool()),
                    )
                    .await;
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
                    let _ = deliver_result(
                        target,
                        &job.name,
                        &msg,
                        job.deliver_api_key.as_deref(),
                        Some(ctx.pool()),
                    )
                    .await;
                }
            }
        }
    }

    // Insert a compaction marker so the next cron run starts with empty
    // context. Without this, every job would see the full conversation
    // history of all previous jobs (the contamination vector).
    let message_svc = crate::services::MessageService::new(ctx.clone());
    if let Err(e) = message_svc
        .create_message(
            session_id,
            "user".to_string(),
            "[CONTEXT COMPACTION — Cron job execution boundary]".to_string(),
        )
        .await
    {
        tracing::warn!("Failed to insert cron compaction marker: {e}");
    }

    Ok(())
}

/// Parse the Telegram target out of a `deliver_to` entry (fork #104).
/// Grammar: `telegram:<chat_id>` → `(chat_id, None)` — the chat's default
/// topic, the behavior every existing job keeps; `telegram:<chat_id>:<thread_id>`
/// → `(chat_id, Some(thread_id))` — opt-in delivery into that forum topic.
/// Anything else (non-numeric components, an extra `:` segment) → `None`;
/// the caller owns the loud failure. Thread delivery is opt-in only: a job
/// configured without a thread component never gets retargeted (#1085
/// scope discipline).
pub(crate) fn parse_telegram_target(target: &str) -> Option<(i64, Option<i64>)> {
    let mut parts = target.split(':');
    let chat_id = parts.next()?.trim().parse::<i64>().ok()?;
    match parts.next() {
        None => Some((chat_id, None)),
        Some(thread) => {
            let thread_id = thread.trim().parse::<i64>().ok()?;
            // A third component means the target is malformed, not deeply nested.
            if parts.next().is_some() {
                return None;
            }
            Some((chat_id, Some(thread_id)))
        }
    }
}

/// Parse a session target out of a `deliver_to` entry (fork #144).
/// Grammar: `session:<uuid-or-8+-char-prefix>` → the target session id.
/// Full UUIDs pass through untouched; anything else is matched as a
/// case-insensitive prefix against the session DB via the shared resolver
/// (`crate::cli::session_resolve`) so operators can paste the short id that
/// `session list` prints. Anything else → `None`; the caller owns the loud
/// failure.
pub(crate) async fn parse_session_target(target: &str) -> Option<Uuid> {
    // Full UUID fast path — no DB needed (resolver passthrough parity).
    if let Ok(uuid) = Uuid::parse_str(target) {
        return Some(uuid);
    }
    let config = crate::config::Config::load().ok()?;
    let db = crate::db::Database::connect(&config.database.path)
        .await
        .ok()?;
    let sessions = crate::db::repository::SessionRepository::new(db.pool().clone())
        .list(crate::db::repository::SessionListOptions::default())
        .await
        .ok()?;
    resolve_session_target(&sessions, target)
}

/// Pure resolution over a session set — the testable core. Full UUIDs are
/// already consumed by the fast path in [`parse_session_target`], so
/// everything reaching here is a prefix: the shared resolver's rules apply
/// verbatim (0 matches and ambiguity are both `None`; the caller logs loudly).
pub(crate) fn resolve_session_target(
    sessions: &[crate::db::models::Session],
    target: &str,
) -> Option<Uuid> {
    crate::cli::session_resolve::resolve_session_id(sessions, target).ok()
}

/// Deliver a cron job result to the specified channel.
/// Format: "telegram:chat_id", "telegram:chat_id:thread_id" (opt-in forum
/// topic), "discord:channel_id", "slack:channel_id", "session:<id|prefix>"
/// (fork #144: into a session's notify queue through the shared
/// `notify_policy` path — default mode `turn-end`, cron results are turn
/// outputs), or an HTTP(S) URL for generic webhook delivery.
async fn deliver_result(
    deliver_to: &str,
    job_name: &str,
    content: &str,
    api_key: Option<&str>,
    pool: Option<crate::db::Pool>,
) -> Option<tokio::task::JoinHandle<()>> {
    // Only the Telegram delivery arm uses the pool (to record the message for
    // reply recovery); other targets ignore it.
    #[cfg(not(feature = "telegram"))]
    let _ = &pool;
    // HTTP(S) URL — generic webhook delivery
    if deliver_to.starts_with("http://") || deliver_to.starts_with("https://") {
        deliver_http(deliver_to, job_name, content, api_key).await;
        return None;
    }

    // Leaked `oc://` target URL at fire time (#148 loud failure pin):
    // cron targets must be baked at create/update time. A leaked URL here
    // means the bake step was bypassed — refuse loudly.
    if crate::channels::target_resolver::is_target_url(deliver_to) {
        tracing::error!(
            "Unbaked target URL '{deliver_to}' reached delivery for job '{job_name}' — oc:// targets must be baked at create/update time (#148); refusing fire-time resolution"
        );
        return None;
    }

    let parts: Vec<&str> = deliver_to.splitn(2, ':').collect();
    if parts.len() != 2 {
        tracing::warn!(
            "Invalid deliver_to format '{}' for job '{}' — expected 'channel:id' or HTTP URL",
            deliver_to,
            job_name
        );
        return None;
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
        "session" => {
            // Fork #144: deliver into a session's notify queue. Cron results
            // are turn outputs, so the default mode is `turn-end` (never
            // derail a mid-turn session — the target drains at its next
            // boundary); `quiet` rides the same policy when configured.
            let Some(session_id) = parse_session_target(target_id).await else {
                tracing::error!(
                    "Invalid session deliver_to target '{target_id}' for job '{job_name}' \
                     — no session matches; not delivering"
                );
                return None;
            };
            tracing::info!("Delivering cron result to session {session_id} (mode turn-end)");
            let queued = crate::brain::agent::service::QueuedUserMessage {
                context_text: delivery_msg.clone(),
                display_text: delivery_msg,
                origin: crate::brain::agent::PushOrigin::SessionNotify,
                bg_meta: None,
            };
            let delivery = crate::brain::agent::service::session_routes::deliver_to_session(
                session_id, queued, false,
            );
            tracing::info!("Cron '{job_name}' session delivery verdict: {delivery:?}");
            return None;
        }
        "telegram" => {
            #[cfg(feature = "telegram")]
            {
                match parse_telegram_target(target_id) {
                    Some((cid, thread_id)) => {
                        tracing::info!(
                            "Delivering cron result to Telegram chat {cid}{}",
                            thread_id
                                .map(|t| format!(" thread {t}"))
                                .unwrap_or_default()
                        );
                        return deliver_telegram(
                            cid,
                            thread_id,
                            job_name,
                            &delivery_msg,
                            pool.clone(),
                        )
                        .await;
                    }
                    None => {
                        tracing::error!(
                            "Invalid Telegram deliver_to target '{target_id}' for job \
                             '{job_name}' — expected 'telegram:<chat_id>' or \
                             'telegram:<chat_id>:<thread_id>'; not delivering"
                        );
                    }
                }
            }
            #[cfg(not(feature = "telegram"))]
            {
                tracing::warn!("Telegram feature not enabled — cannot deliver cron result");
            }
        }
        "discord" => {
            #[cfg(feature = "discord")]
            {
                tracing::info!("Delivering cron result to Discord channel {target_id}");
                deliver_discord(target_id, &delivery_msg).await;
            }
            #[cfg(not(feature = "discord"))]
            {
                tracing::warn!("Discord feature not enabled — cannot deliver cron result");
            }
        }
        "slack" => {
            #[cfg(feature = "slack")]
            {
                tracing::info!("Delivering cron result to Slack channel {target_id}");
                deliver_slack(target_id, &delivery_msg).await;
            }
            #[cfg(not(feature = "slack"))]
            {
                tracing::warn!("Slack feature not enabled — cannot deliver cron result");
            }
        }
        other => {
            tracing::warn!("Unknown delivery channel '{other}' for job '{job_name}'");
        }
    }
    None
}

/// Deliver cron result via HTTP POST to a generic webhook URL.
async fn deliver_http(url: &str, job_name: &str, content: &str, api_key: Option<&str>) {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "job_name": job_name,
        "content": content,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    let mut request = client.post(url).json(&body);

    // Attach Bearer token if the job has one configured.
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {key}"));
    }

    match request.send().await {
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

/// Read `channels.<channel>.<field>` (e.g. a bot token) from the active
/// workspace's `keys.toml`. Cron delivery runs outside any channel's live
/// connection, so it reads the credential straight off disk.
#[cfg(any(feature = "telegram", feature = "discord", feature = "slack"))]
fn read_channel_secret(channel: &str, field: &str) -> Option<String> {
    let keys_path = crate::brain::BrainLoader::resolve_path().join("keys.toml");
    let content = std::fs::read_to_string(&keys_path).ok()?;
    content.parse::<toml::Table>().ok().and_then(|t| {
        t.get("channels")?
            .as_table()?
            .get(channel)?
            .as_table()?
            .get(field)?
            .as_str()
            .map(String::from)
    })
}

/// Split `text` into `<= max_len` byte chunks, breaking on a newline near the
/// limit when possible and never inside a multi-byte char. Used for Discord's
/// 2000-char and Slack's message limits. (Telegram reuses its own chunker so
/// HTML stays valid across splits.)
#[cfg(any(feature = "discord", feature = "slack"))]
fn split_for_delivery(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max_len).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end -= 1;
        }
        let break_at = if end < text.len() {
            text[start..end]
                .rfind('\n')
                .filter(|&pos| pos > end - start - 200)
                .map(|pos| start + pos + 1)
                .unwrap_or(end)
        } else {
            end
        };
        chunks.push(&text[start..break_at]);
        start = break_at;
    }
    chunks
}

/// Deliver via the shared Telegram outbox ladder (#1085 P1b R2). The
/// hand-rolled raw `reqwest` POST + message_id JSON scrape is gone: retry
/// (#297), 4096 chunking, plain-text fallback (Q4) and correlation
/// telemetry come from `send_markdown_outbox`, and the delivery is
/// persisted through the same `record_outgoing` the tool path uses.
#[cfg(feature = "telegram")]
async fn deliver_telegram(
    chat_id: i64,
    thread_id: Option<i64>,
    job_name: &str,
    message: &str,
    pool: Option<crate::db::Pool>,
) -> Option<tokio::task::JoinHandle<()>> {
    let Some(token) = read_channel_secret("telegram", "token") else {
        tracing::warn!("No Telegram bot token found in keys.toml — cannot deliver cron result");
        return None;
    };
    let bot = teloxide::Bot::new(token);
    use teloxide::prelude::Requester;
    // Opt-in thread delivery (fork #104): `telegram:<chat_id>:<thread_id>`
    // routes into that forum topic; a bare `telegram:<chat_id>` stays on the
    // chat's default topic — existing jobs are never silently retargeted
    // (#1085 scope discipline).
    let thread = thread_id.map(|t| teloxide::types::ThreadId(teloxide::types::MessageId(t as i32)));
    // Fire-time validation for the opt-in path: the chat must be a forum
    // with topics enabled. A thread pointed at a non-forum chat is a
    // configuration error — fail LOUDLY here rather than drop the message
    // into General (that fallback is the exact silent-retargeting behavior
    // the deliver_to grammar must never perform). Topic *existence* has no
    // Bot API read; it is proven by the send itself, and a dead topic id
    // surfaces as a loud send error below.
    if let Some(tid) = thread_id {
        match bot.get_chat(teloxide::types::ChatId(chat_id)).await {
            Ok(chat) if is_forum_chat(&chat) => {}
            Ok(_) => {
                tracing::error!(
                    "Cron job '{job_name}': deliver_to thread {tid} rejected — chat {chat_id} \
                     is not a forum (topics disabled); refusing delivery instead of falling \
                     back to the default topic"
                );
                return None;
            }
            Err(e) => {
                tracing::error!(
                    "Cron job '{job_name}': cannot validate chat {chat_id} for thread \
                     {tid} delivery: {e} — refusing delivery"
                );
                return None;
            }
        }
    }
    // Delivery runs detached (review F18): the shared outbox ladder can
    // legally wait out 429 windows (up to ~90s total per send). Awaiting
    // that inline would stall the whole scheduler tick — one flood-banned
    // chat must not delay every other job. The outbox telemetry carries
    // the outcome either way.
    //
    // Returns the spawn handle so rebuild jobs can await delivery before
    // exec() replaces the process (#1105). Normal cron jobs discard the
    // handle — their delivery survives the tick regardless.
    let message = message.to_string();
    let job_name = job_name.to_string();
    Some(tokio::spawn(async move {
        match crate::channels::telegram::send::send_markdown_outbox(
            &bot,
            teloxide::types::ChatId(chat_id),
            thread,
            &message,
            "cron",
            &job_name,
            None,
        )
        .await
        {
            Ok(sent) => {
                tracing::info!(
                    "Cron result for '{job_name}' delivered to Telegram chat {chat_id}{} ({} part(s))",
                    thread_id
                        .map(|t| format!(" thread {t}"))
                        .unwrap_or_default(),
                    sent.len()
                );
                // Persist keyed by message id so a reply to the cron post
                // resolves to this exact content (#234).
                crate::channels::telegram::send::record_outgoing(pool, chat_id, thread, &sent)
                    .await;
            }
            Err(e) => {
                if let Some(t) = thread_id {
                    tracing::error!(
                        "Cron delivery for '{job_name}' to chat {chat_id} thread {t} failed: {e} — \
                         if the error is 'message thread not found', topic {t} does not exist \
                         in chat {chat_id}; fix the job's deliver_to (there is no fallback to the \
                         default topic)"
                    );
                } else {
                    tracing::error!("Cron delivery for '{job_name}' to chat {chat_id} failed: {e}");
                }
            }
        }
    }))
}

/// Whether a `get_chat` result describes a forum (topics-enabled) group.
/// Only supergroups carry the `is_forum` flag; channels/groups/private
/// chats are all non-forum, so opt-in thread delivery rejects them.
#[cfg(feature = "telegram")]
fn is_forum_chat(chat: &teloxide::types::ChatFullInfo) -> bool {
    matches!(
        &chat.kind,
        teloxide::types::ChatFullInfoKind::Public(public)
            if matches!(
                &public.kind,
                teloxide::types::ChatFullInfoPublicKind::Supergroup(supergroup)
                    if supergroup.is_forum
            )
    )
}

/// Deliver via Discord Bot API (direct HTTP POST to the channel-messages
/// endpoint). Discord renders its own markdown natively, so the content is
/// sent as-is, chunked to the 2000-char message limit.
#[cfg(feature = "discord")]
async fn deliver_discord(channel_id: &str, message: &str) {
    let Some(token) = read_channel_secret("discord", "token") else {
        tracing::warn!("No Discord bot token found in keys.toml — cannot deliver cron result");
        return;
    };

    let url = format!("https://discord.com/api/v10/channels/{channel_id}/messages");
    let client = reqwest::Client::new();
    let mut delivered = 0usize;
    for chunk in split_for_delivery(message, 2000) {
        let body = serde_json::json!({ "content": chunk });
        match client
            .post(&url)
            .header("Authorization", format!("Bot {token}"))
            .json(&body)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => delivered += 1,
            Ok(resp) => {
                tracing::warn!(
                    "Discord delivery to {channel_id} failed ({}): {:?}",
                    resp.status(),
                    resp.text().await.unwrap_or_default()
                );
            }
            Err(e) => {
                tracing::error!("Discord delivery to {channel_id} HTTP error: {e}");
            }
        }
    }
    if delivered > 0 {
        tracing::info!(
            "Cron result delivered to Discord channel {channel_id} ({delivered} part(s))"
        );
    }
}

/// Deliver via Slack Web API (`chat.postMessage`). The `text` field renders
/// Slack mrkdwn. Slack returns HTTP 200 even on a logical failure
/// (`{"ok":false,"error":...}`), so we inspect the body, not just the status.
#[cfg(feature = "slack")]
async fn deliver_slack(channel_id: &str, message: &str) {
    let Some(token) = read_channel_secret("slack", "token") else {
        tracing::warn!("No Slack bot token found in keys.toml — cannot deliver cron result");
        return;
    };

    let url = "https://slack.com/api/chat.postMessage";
    let client = reqwest::Client::new();
    let mut delivered = 0usize;
    for chunk in split_for_delivery(message, 3500) {
        let body = serde_json::json!({ "channel": channel_id, "text": chunk });
        match client
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                let parsed: serde_json::Value = resp.json().await.unwrap_or_default();
                if parsed.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
                    delivered += 1;
                } else {
                    tracing::warn!(
                        "Slack delivery to {channel_id} failed: {}",
                        parsed
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("unknown error")
                    );
                }
            }
            Err(e) => {
                tracing::error!("Slack delivery to {channel_id} HTTP error: {e}");
            }
        }
    }
    if delivered > 0 {
        tracing::info!("Cron result delivered to Slack channel {channel_id} ({delivered} part(s))");
    }
}
