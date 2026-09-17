use crate::brain::goal::GoalManager;
use crate::channels::target_resolver::extract_session_target;
use crate::cron::scheduler::resolve_session_target;
use crate::cron::trigger::{TriggerCondition, TriggerResult, TriggerRunner};
use crate::db::models::{CronJob, CronJobRun};
use crate::db::repository::CronJobRunRepository;
use crate::services::ServiceContext;
use uuid::Uuid;

/// Outcome of evaluating a cron trigger.
#[derive(Debug)]
pub enum TriggerOutcome {
    /// No trigger configured on the job.
    NoTrigger,
    /// Trigger ran and fired with the returned result.
    Fired(TriggerResult),
    /// Trigger ran and condition was NOT met (short-circuit).
    Skipped(TriggerResult),
    /// Trigger execution encountered an error.
    Error(String),
}

/// Helper to interpolate `{output}`, `{stdout}`, `{stderr}`, `{exit_code}` into templates.
pub fn interpolate_template(template: &str, result: &TriggerResult) -> String {
    template
        .replace("{output}", &result.combined_output())
        .replace("{stdout}", &result.stdout)
        .replace("{stderr}", &result.stderr)
        .replace("{exit_code}", &result.exit_code.to_string())
}

/// Handles trigger pre-flight evaluation, 0-token pass-through, and goal dispatch.
pub struct PipelineExecutor;

impl PipelineExecutor {
    /// Evaluate the trigger command for a job if present.
    pub async fn evaluate_trigger(job: &CronJob) -> TriggerOutcome {
        let Some(ref cmd) = job.trigger_cmd else {
            return TriggerOutcome::NoTrigger;
        };

        if cmd.trim().is_empty() {
            return TriggerOutcome::NoTrigger;
        }

        let condition = TriggerCondition::parse(job.trigger_on.as_deref());
        let runner = TriggerRunner::default();

        match runner.run(cmd).await {
            Ok(result) => {
                if condition.should_fire(&result) {
                    TriggerOutcome::Fired(result)
                } else {
                    TriggerOutcome::Skipped(result)
                }
            }
            Err(e) => TriggerOutcome::Error(e),
        }
    }

    /// Record a skipped trigger run in the database (0 tokens, status="skipped").
    pub async fn record_skipped_run(
        job: &CronJob,
        result: &TriggerResult,
        run_repo: &CronJobRunRepository,
    ) -> anyhow::Result<()> {
        let run = CronJobRun::new_running(
            job.id,
            job.name.clone(),
            job.provider.clone(),
            job.model.clone(),
        );
        let run_id = run.id.to_string();
        run_repo.insert(&run).await?;

        let skipped_msg = format!(
            "Trigger condition not met (exit_code={}, output_bytes={}). Execution skipped.",
            result.exit_code,
            result.combined_output().len()
        );
        run_repo
            .complete_success(&run_id, &skipped_msg, 0, 0, 0.0)
            .await?;
        tracing::info!(
            "Cron job '{}' trigger skipped — recorded 0-token run",
            job.name
        );
        Ok(())
    }

    /// Dispatch active goal to target session if `set_goal` is true and `deliver_to` resolves to a session.
    pub async fn maybe_dispatch_goal(
        job: &CronJob,
        ctx: &ServiceContext,
        content: &str,
    ) -> anyhow::Result<Option<Uuid>> {
        if !job.set_goal {
            return Ok(None);
        }

        let Some(ref deliver_to) = job.deliver_to else {
            return Ok(None);
        };

        let Some(sid_raw) = extract_session_target(deliver_to) else {
            return Ok(None);
        };

        let session_repo = crate::db::repository::SessionRepository::new(ctx.pool());
        let sessions = session_repo
            .list(crate::db::repository::SessionListOptions {
                include_archived: true,
                include_subagents: true,
                ..Default::default()
            })
            .await
            .unwrap_or_default();
        let target_uuid = resolve_session_target(&sessions, sid_raw);

        if let Some(uuid) = target_uuid {
            let goal_text = if let Some(ref tmpl) = job.goal_template {
                tmpl.replace("{output}", content)
            } else {
                content.to_string()
            };

            let goal_mgr = GoalManager::new(ctx.clone());
            match goal_mgr.set_goal(uuid, goal_text, None, None, None).await {
                Ok(state) => {
                    tracing::info!(
                        "Dispatched goal to session {} for cron job '{}'",
                        uuid,
                        job.name
                    );
                    return Ok(Some(state.session_id));
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to dispatch goal to session {} for cron job '{}': {e}",
                        uuid,
                        job.name
                    );
                }
            }
        }

        Ok(None)
    }
}
