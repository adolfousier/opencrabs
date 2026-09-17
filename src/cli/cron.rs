//! Cron CLI subcommands — add, list, remove, enable, disable.

use super::args::CronCommands;
use crate::db::models::CronJob;
use crate::db::CronJobRepository;
use anyhow::Result;

/// Cron job management CLI handler
pub(crate) async fn cmd_cron(
    config: &crate::config::Config,
    operation: CronCommands,
) -> Result<()> {
    use crate::db::Database;

    let db = Database::connect(&config.database.path).await?;
    db.run_migrations().await?;
    let repo = CronJobRepository::new(db.pool().clone());

    match operation {
        CronCommands::Add {
            name,
            cron,
            tz,
            prompt,
            provider,
            model,
            thinking,
            auto_approve,
            deliver_to,
            trigger_cmd,
            trigger_on,
            set_goal,
            goal_template,
        } => {
            cmd_add(
                &repo,
                name,
                cron,
                tz,
                prompt,
                provider,
                model,
                thinking,
                auto_approve,
                deliver_to,
                trigger_cmd,
                trigger_on,
                set_goal,
                goal_template,
            )
            .await
        }
        CronCommands::List => cmd_list(&repo).await,
        CronCommands::Remove { id } => cmd_remove(&repo, &id).await,
        CronCommands::Enable { id } => cmd_toggle(&repo, &id, true).await,
        CronCommands::Disable { id } => cmd_toggle(&repo, &id, false).await,
        CronCommands::Test { id } => cmd_test(&repo, &id).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn cmd_add(
    repo: &CronJobRepository,
    name: String,
    cron: String,
    tz: String,
    prompt: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    thinking: String,
    auto_approve: bool,
    deliver_to: Option<String>,
    trigger_cmd: Option<String>,
    trigger_on: Option<String>,
    set_goal: bool,
    goal_template: Option<String>,
) -> Result<()> {
    // Prompt or trigger_cmd must be present
    let prompt_str = prompt.unwrap_or_default();
    if prompt_str.trim().is_empty() && trigger_cmd.as_deref().unwrap_or("").trim().is_empty() {
        anyhow::bail!("Either --prompt or --trigger-cmd must be provided");
    }

    // Validate cron expression (cron crate needs seconds prepended)
    let cron_with_secs = format!("0 {cron}");
    if let Err(e) = cron_with_secs.parse::<cron::Schedule>() {
        anyhow::bail!(
            "Invalid cron expression '{cron}': {e}. 5-field 'min hour dom mon dow'; \
             day-of-week is 1-7 = Sun-Sat (0 invalid) — prefer names like Mon-Fri."
        );
    }
    // Validate the timezone — it's honored by the scheduler (jobs run in this
    // zone, DST-aware), so reject an unknown zone rather than silently UTC.
    let parsed_tz = match crate::cron::parse_timezone(&tz) {
        Some(t) => t,
        None => anyhow::bail!(
            "Unknown timezone '{tz}'. Use an IANA name like 'America/New_York', 'Europe/London', or 'UTC'."
        ),
    };

    // Validate set_goal requires session delivery target
    if set_goal {
        let target_is_session = deliver_to
            .as_deref()
            .map(crate::channels::target_resolver::is_session_target)
            .unwrap_or(false);
        if !target_is_session {
            anyhow::bail!(
                "Cannot create job: --set-goal requires oc://session/<uuid> delivery target. \
                 Channel deliveries (telegram, discord, etc.) are passive outputs and cannot set session goals."
            );
        }
    }

    if (repo.find_by_name(&name).await?).is_some() {
        anyhow::bail!("A cron job named '{name}' already exists");
    }

    let deliver_to = if let Some(ref d) = deliver_to {
        if crate::channels::target_resolver::is_session_target(d) {
            let id_str =
                crate::channels::target_resolver::extract_session_target(d).unwrap_or(d.as_str());
            Some(format!("session:{id_str}"))
        } else {
            deliver_to
        }
    } else {
        deliver_to
    };

    let mut job = CronJob::new_with_trigger(
        name.clone(),
        cron.clone(),
        tz.clone(),
        prompt_str,
        provider,
        model,
        thinking,
        auto_approve,
        deliver_to.clone(),
        None, // CLI doesn't support deliver_api_key yet
        trigger_cmd.clone(),
        trigger_on.clone(),
        set_goal,
        goal_template.clone(),
    );
    job.next_run_at = crate::cron::next_run_utc(&cron, parsed_tz, chrono::Utc::now());

    let id = job.id.to_string();
    repo.insert(&job).await?;

    println!("✅ Cron job created:");
    println!("   ID: {id}");
    println!("   Name: {name}");
    println!("   Schedule: {cron} ({tz})");
    if let Some(ref d) = deliver_to {
        println!("   Deliver to: {d}");
    }
    if let Some(ref cmd) = trigger_cmd {
        let cond = trigger_on.as_deref().unwrap_or("non_empty");
        println!("   Trigger: `{cmd}` ({cond})");
    }
    if set_goal {
        println!("   Set goal: true");
    }
    println!("   Next runs:");
    println!(
        "{}",
        crate::cron::format_upcoming(&cron, parsed_tz, 3, chrono::Utc::now())
    );
    println!("\n💡 Verify the next runs above match what you intended.");
    Ok(())
}

async fn cmd_list(repo: &CronJobRepository) -> Result<()> {
    let jobs = repo.list_all().await?;
    if jobs.is_empty() {
        println!("No cron jobs configured.");
        println!(
            "\n💡 Add one: opencrabs cron add --name \"My Job\" --cron \"0 9 * * *\" --prompt \"Do something\""
        );
        return Ok(());
    }

    println!("⏰ Cron Jobs ({}):\n", jobs.len());
    for job in &jobs {
        let status = if job.enabled { "✅" } else { "⏸️ " };
        let last = job
            .last_run_at
            .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "never".to_string());
        let next = job
            .next_run_at
            .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "none scheduled".to_string());
        let deliver = job.deliver_to.as_deref().unwrap_or("none");
        let prompt_preview = if job.prompt.len() > 60 {
            format!("{}...", job.prompt.chars().take(60).collect::<String>())
        } else {
            job.prompt.clone()
        };

        println!("{status} {} ({})", job.name, job.id);
        println!("   Schedule: {} ({})", job.cron_expr, job.timezone);
        if let Some(ref cmd) = job.trigger_cmd {
            let cond = job.trigger_on.as_deref().unwrap_or("non_empty");
            println!("   Trigger: `{cmd}` ({cond})");
        }
        if job.set_goal {
            println!("   Set goal: true");
        }
        println!("   Deliver: {deliver}");
        println!("   Last run: {last}");
        println!("   Next run: {next}");
        if !job.prompt.is_empty() {
            println!("   Prompt: {prompt_preview}");
        }
        println!();
    }
    Ok(())
}

async fn cmd_remove(repo: &CronJobRepository, id: &str) -> Result<()> {
    let job_id = resolve_job_id(repo, id).await?;
    if repo.delete(&job_id).await? {
        println!("✅ Cron job removed: {job_id}");
    } else {
        println!("❌ No cron job found: {id}");
    }
    Ok(())
}

async fn cmd_toggle(repo: &CronJobRepository, id: &str, enabled: bool) -> Result<()> {
    let job_id = resolve_job_id(repo, id).await?;
    if repo.set_enabled(&job_id, enabled).await? {
        let state = if enabled { "enabled" } else { "disabled" };
        let icon = if enabled { "✅" } else { "⏸️ " };
        println!("{icon} Cron job {state}: {job_id}");
    } else {
        println!("❌ No cron job found: {id}");
    }
    Ok(())
}

async fn cmd_test(repo: &CronJobRepository, id: &str) -> Result<()> {
    let job_id = resolve_job_id(repo, id).await?;
    if repo.trigger_now(&job_id).await? {
        println!("🚀 Cron job triggered: {job_id}");
        println!("   It will execute on the next scheduler tick (within 60 seconds).");
    } else {
        println!("❌ Failed to trigger cron job: {id}");
    }
    Ok(())
}

/// Resolve a job identifier — accepts UUID or name.
async fn resolve_job_id(repo: &CronJobRepository, id: &str) -> Result<String> {
    if let Ok(Some(job)) = repo.find_by_id(id).await {
        return Ok(job.id.to_string());
    }
    if let Ok(Some(job)) = repo.find_by_name(id).await {
        return Ok(job.id.to_string());
    }
    anyhow::bail!("No cron job found with ID or name '{id}'")
}
