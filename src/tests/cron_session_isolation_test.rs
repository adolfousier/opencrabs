//! Tests for #149: cron jobs must NOT share one session.
//!
//! Pre-#149 every job ran in a single shared "Cron" session. That design
//! cross-pollinates whenever jobs overlap in time (routine under the 60s
//! tick): context loads from the LAST compaction marker, and a job starting
//! mid-flight of another reads that job's prompt/tool activity as its own
//! context; the per-session provider swap is likewise keyed to the shared id
//! and a concurrent job's swap overwrites the running job's provider.
//!
//! The fix resolves ONE SESSION PER JOB via a stable title suffix
//! `[cron-job:<job-uuid>]`. These tests cover the resolution contract: two
//! different jobs resolve to different sessions; the same job resolves to
//! the SAME session across fires (its history + provider swaps stay
//! self-consistent); and a rename of the readable title part does not
//! orphan the session (the suffix is the lookup key).

use crate::config::profile::with_home_override_async;
use crate::cron::scheduler::cron_session_title_suffix;
use crate::db::{Database, models::CronJob};
use crate::services::{ServiceContext, SessionService};
use uuid::Uuid;

async fn test_ctx() -> ServiceContext {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    ServiceContext::new(db.pool().clone())
}

fn make_job(name: &str) -> CronJob {
    CronJob::new(
        name.to_string(),
        "0 9 * * *".to_string(),
        "UTC".to_string(),
        "do things".to_string(),
        None,
        None,
        "off".to_string(),
        true,
        None,
        None,
    )
}

/// Two DIFFERENT jobs must resolve to DIFFERENT sessions — the core #149
/// guarantee. No shared "Cron" session exists anymore.
#[tokio::test]
async fn different_jobs_get_different_sessions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&home).expect("create home");

    with_home_override_async(home, async {
        let ctx = test_ctx().await;
        let job_a = make_job("job-a");
        let job_b = make_job("job-b");

        let sid_a = crate::cron::scheduler::resolve_or_create_cron_session(&ctx, &job_a)
            .await
            .expect("resolve session for job-a");
        let sid_b = crate::cron::scheduler::resolve_or_create_cron_session(&ctx, &job_b)
            .await
            .expect("resolve session for job-b");

        assert_ne!(
            sid_a, sid_b,
            "two different jobs resolved the SAME session — #149 cross-pollination is back"
        );
    })
    .await;
}

/// The SAME job must resolve to the SAME session across fires, so its own
/// history stays coherent and its end-of-run compaction marker bounds its
/// own next fire's context.
#[tokio::test]
async fn same_job_reuses_its_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&home).expect("create home");

    with_home_override_async(home, async {
        let ctx = test_ctx().await;
        let job = make_job("recurring-job");

        let first = crate::cron::scheduler::resolve_or_create_cron_session(&ctx, &job)
            .await
            .expect("first resolve");
        let second = crate::cron::scheduler::resolve_or_create_cron_session(&ctx, &job)
            .await
            .expect("second resolve");

        assert_eq!(first, second, "same job must reuse its own session");
    })
    .await;
}

/// The lookup key is the `[cron-job:<uuid>]` suffix, not the readable title:
/// renaming the title part must not orphan the job's session (mirrors the
/// channel-handler `[chat:N]` rename-safety pattern).
#[tokio::test]
async fn renamed_session_still_resolves_by_suffix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&home).expect("create home");

    with_home_override_async(home, async {
        let ctx = test_ctx().await;
        let job = make_job("renamed-job");

        let original = crate::cron::scheduler::resolve_or_create_cron_session(&ctx, &job)
            .await
            .expect("initial resolve");

        // Simulate a user rename: same session row, new readable title,
        // suffix preserved.
        let session_svc = SessionService::new(ctx.clone());
        let mut session = session_svc
            .get_session(original)
            .await
            .expect("get session")
            .expect("session exists");
        session.title = Some(format!(
            "Renamed by user {}",
            cron_session_title_suffix(&job)
        ));
        session_svc
            .update_session(&session)
            .await
            .expect("rename session");

        let resolved = crate::cron::scheduler::resolve_or_create_cron_session(&ctx, &job)
            .await
            .expect("resolve after rename");
        assert_eq!(
            resolved, original,
            "renaming the title part must not orphan the job's session"
        );
    })
    .await;
}

/// The suffix is derived from the job's ID, not its name — two jobs with the
/// SAME name (possible in the jobs table) still get separate sessions.
#[tokio::test]
async fn same_name_different_ids_get_different_sessions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&home).expect("create home");

    with_home_override_async(home, async {
        let ctx = test_ctx().await;
        let job_a = make_job("twin");
        let mut job_b = make_job("twin");
        job_b.id = Uuid::new_v4();

        assert_ne!(job_a.id, job_b.id, "test setup: ids must differ");
        let sid_a = crate::cron::scheduler::resolve_or_create_cron_session(&ctx, &job_a)
            .await
            .expect("resolve twin-a");
        let sid_b = crate::cron::scheduler::resolve_or_create_cron_session(&ctx, &job_b)
            .await
            .expect("resolve twin-b");

        assert_ne!(sid_a, sid_b, "same-name jobs must not share a session");
    })
    .await;
}
