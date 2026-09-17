use crate::brain::provider::Provider;
use crate::brain::tools::cron_manage::CronManageTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use crate::channels::ChannelFactory;
use crate::config::Config;
use crate::cron::scheduler::CronScheduler;
use crate::db::models::CronJob;
use crate::db::repository::cron_job_run::CronJobRunRepository;
use crate::db::{CronJobRepository, Database};
use crate::services::ServiceContext;
use crate::tests::agent_service_mocks::MockProvider;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

async fn setup() -> (Database, CronJobRepository, CronManageTool) {
    let db = Database::connect_in_memory()
        .await
        .expect("Failed to create database");
    db.run_migrations().await.expect("Failed to run migrations");
    let repo = CronJobRepository::new(db.pool().clone());
    let tool = CronManageTool::new(repo.clone());
    (db, repo, tool)
}

fn ctx() -> ToolExecutionContext {
    ToolExecutionContext::new(uuid::Uuid::new_v4())
}

fn create_test_factory(service_ctx: ServiceContext) -> Arc<ChannelFactory> {
    let (_, config_rx) = watch::channel(Config::default());
    Arc::new(ChannelFactory::new(
        Arc::new(MockProvider) as Arc<dyn Provider>,
        service_ctx,
        "test brain".to_string(),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp/oc_test_brain"),
        Arc::new(Mutex::new(None)),
        config_rx,
    ))
}

#[tokio::test]
async fn test_create_standalone_bash_job() {
    let (_db, repo, tool) = setup().await;
    let input = serde_json::json!({
        "action": "create",
        "name": "Standalone Bash Job",
        "cron": "0 9 * * *",
        "tz": "UTC",
        "trigger_cmd": "echo 'running maintenance'",
        "trigger_on": "exit_zero"
    });
    let res = tool.execute(input, &ctx()).await.unwrap();
    assert!(res.success, "Tool execution failed: {}", res.output);

    let job = repo
        .find_by_name("Standalone Bash Job")
        .await
        .unwrap()
        .expect("Job not found");
    assert_eq!(job.prompt, "");
    assert_eq!(
        job.trigger_cmd.as_deref(),
        Some("echo 'running maintenance'")
    );
    assert_eq!(job.trigger_on.as_deref(), Some("exit_zero"));
}

#[tokio::test]
async fn test_create_seeds_next_run_at() {
    let (_db, repo, tool) = setup().await;
    let input = serde_json::json!({
        "action": "create",
        "name": "Seeded Job",
        "cron": "0 9 * * *",
        "tz": "UTC",
        "prompt": "check status"
    });
    let res = tool.execute(input, &ctx()).await.unwrap();
    assert!(res.success, "Tool execution failed: {}", res.output);

    let job = repo
        .find_by_name("Seeded Job")
        .await
        .unwrap()
        .expect("Job not found");
    assert!(
        job.next_run_at.is_some(),
        "next_run_at should be seeded on creation"
    );
    let next = job.next_run_at.unwrap();
    assert!(
        next > chrono::Utc::now(),
        "next_run_at should be in the future"
    );
}

#[tokio::test]
async fn test_update_schedule_recomputes_next_run_at() {
    let (_db, repo, tool) = setup().await;
    let input = serde_json::json!({
        "action": "create",
        "name": "Update Schedule Job",
        "cron": "0 9 * * *",
        "tz": "UTC",
        "prompt": "run test"
    });
    tool.execute(input, &ctx()).await.unwrap();
    let job_before = repo
        .find_by_name("Update Schedule Job")
        .await
        .unwrap()
        .unwrap();
    let id = job_before.id.to_string();

    let update_input = serde_json::json!({
        "action": "update",
        "job_id": id,
        "cron": "0 18 * * *"
    });
    let res = tool.execute(update_input, &ctx()).await.unwrap();
    assert!(res.success, "Update failed: {}", res.output);

    let job_after = repo.find_by_id(&id).await.unwrap().unwrap();
    assert!(
        job_after.next_run_at.is_some(),
        "next_run_at should not be null after update"
    );
    assert_ne!(
        job_before.next_run_at, job_after.next_run_at,
        "next_run_at should be recomputed when schedule changes"
    );
}

#[tokio::test]
async fn test_update_timezone_recomputes_next_run_at() {
    let (_db, repo, tool) = setup().await;
    let input = serde_json::json!({
        "action": "create",
        "name": "Update Tz Job",
        "cron": "0 9 * * *",
        "tz": "UTC",
        "prompt": "run test"
    });
    tool.execute(input, &ctx()).await.unwrap();
    let job_before = repo.find_by_name("Update Tz Job").await.unwrap().unwrap();
    let id = job_before.id.to_string();

    let update_input = serde_json::json!({
        "action": "update",
        "job_id": id,
        "tz": "America/New_York"
    });
    let res = tool.execute(update_input, &ctx()).await.unwrap();
    assert!(res.success, "Update failed: {}", res.output);

    let job_after = repo.find_by_id(&id).await.unwrap().unwrap();
    assert!(
        job_after.next_run_at.is_some(),
        "next_run_at should not be null after update"
    );
    assert_ne!(
        job_before.next_run_at, job_after.next_run_at,
        "next_run_at should be recomputed when timezone changes"
    );
}

#[tokio::test]
async fn test_startup_backfill_populates_null_next_run_at() {
    let (db, repo, _tool) = setup().await;
    let mut job = CronJob::new(
        "Legacy Unscheduled Job".to_string(),
        "0 12 * * *".to_string(),
        "UTC".to_string(),
        "report".to_string(),
        None,
        None,
        "off".to_string(),
        true,
        None,
        None,
    );
    job.next_run_at = None;
    job.last_run_at = Some(chrono::Utc::now() - chrono::Duration::hours(24));
    repo.insert(&job).await.unwrap();

    let saved = repo
        .find_by_name("Legacy Unscheduled Job")
        .await
        .unwrap()
        .unwrap();
    assert!(
        saved.next_run_at.is_none(),
        "Precondition: next_run_at must be None"
    );
    let original_last_run = saved.last_run_at;

    let run_repo = CronJobRunRepository::new(db.pool().clone());
    let service_ctx = ServiceContext::new(db.pool().clone());
    let factory = create_test_factory(service_ctx.clone());
    let scheduler = CronScheduler::new(repo.clone(), run_repo, factory, service_ctx);

    let backfilled = scheduler.backfill_missing_next_run().await.unwrap();
    assert_eq!(backfilled, 1, "Expected 1 job to be backfilled");

    let updated = repo
        .find_by_name("Legacy Unscheduled Job")
        .await
        .unwrap()
        .unwrap();
    assert!(
        updated.next_run_at.is_some(),
        "next_run_at should now be populated"
    );
    assert!(updated.next_run_at.unwrap() > chrono::Utc::now());
    assert_eq!(
        updated.last_run_at, original_last_run,
        "backfill must leave last_run_at untouched"
    );
}

#[tokio::test]
async fn test_backfill_handles_invalid_cron_gracefully() {
    let (db, repo, _tool) = setup().await;
    let mut job = CronJob::new(
        "Corrupt Cron Job".to_string(),
        "invalid cron expr".to_string(),
        "UTC".to_string(),
        "fail".to_string(),
        None,
        None,
        "off".to_string(),
        true,
        None,
        None,
    );
    job.next_run_at = None;
    repo.insert(&job).await.unwrap();

    let run_repo = CronJobRunRepository::new(db.pool().clone());
    let service_ctx = ServiceContext::new(db.pool().clone());
    let factory = create_test_factory(service_ctx.clone());
    let scheduler = CronScheduler::new(repo.clone(), run_repo, factory, service_ctx);

    let backfilled = scheduler.backfill_missing_next_run().await.unwrap();
    assert_eq!(
        backfilled, 0,
        "Corrupt cron should be skipped without error"
    );

    let job = repo
        .find_by_name("Corrupt Cron Job")
        .await
        .unwrap()
        .unwrap();
    assert!(
        job.next_run_at.is_none(),
        "Corrupt cron next_run_at remains None"
    );
}

#[tokio::test]
async fn test_list_displays_next_run() {
    let (_db, repo, tool) = setup().await;
    let input = serde_json::json!({
        "action": "create",
        "name": "Listing Test Job",
        "cron": "0 9 * * *",
        "tz": "UTC",
        "prompt": "list me"
    });
    tool.execute(input, &ctx()).await.unwrap();

    let list_res = tool
        .execute(serde_json::json!({"action": "list"}), &ctx())
        .await
        .unwrap();
    assert!(list_res.success);
    assert!(
        list_res.output.contains("Next run:"),
        "Listing output should contain 'Next run:'"
    );

    let job = repo
        .find_by_name("Listing Test Job")
        .await
        .unwrap()
        .unwrap();
    let next_str = job
        .next_run_at
        .unwrap()
        .format("%Y-%m-%d %H:%M UTC")
        .to_string();
    assert!(
        list_res.output.contains(&next_str),
        "Listing output should contain formatted timestamp {}",
        next_str
    );
}
