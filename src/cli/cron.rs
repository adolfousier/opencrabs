//! Cron CLI subcommands — add, list, remove, enable, disable.

use super::CronCommands;
use crate::db::CronJobRepository;
use crate::db::models::CronJob;
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
    prompt: String,
    provider: Option<String>,
    model: Option<String>,
    thinking: String,
    auto_approve: bool,
    deliver_to: Option<String>,
) -> Result<()> {
    // Validate cron expression (cron crate needs seconds prepended)
    let cron_with_secs = format!("0 {cron}");
    if let Err(e) = cron_with_secs.parse::<cron::Schedule>() {
        anyhow::bail!("Invalid cron expression '{cron}': {e}");
    }

    if (repo.find_by_name(&name).await?).is_some() {
        anyhow::bail!("A cron job named '{name}' already exists");
    }

    let job = CronJob::new(
        name.clone(),
        cron.clone(),
        tz.clone(),
        prompt,
        provider,
        model,
        thinking,
        auto_approve,
        deliver_to.clone(),
    );

    let id = job.id.to_string();
    repo.insert(&job).await?;

    println!("✅ Cron job created:");
    println!("   ID: {id}");
    println!("   Name: {name}");
    println!("   Schedule: {cron} ({tz})");
    if let Some(ref d) = deliver_to {
        println!("   Deliver to: {d}");
    }
    println!("\n💡 Job will run on next scheduler tick (within 60s)");
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
        let deliver = job.deliver_to.as_deref().unwrap_or("none");
        let prompt_preview = if job.prompt.len() > 60 {
            format!("{}...", &job.prompt[..60])
        } else {
            job.prompt.clone()
        };

        println!("{status} {} ({})", job.name, job.id);
        println!("   Schedule: {} ({})", job.cron_expr, job.timezone);
        println!("   Deliver: {deliver}");
        println!("   Last run: {last}");
        println!("   Prompt: {prompt_preview}");
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
