//! #1114: `/doctor --fix` engine.
//!
//! Stuck cron rows clear, fresh ones survive; stale pre-init markers go,
//! fresh ones stay; permissions tighten only where loose.

use crate::cli::doctor_fix::{clear_stale_preinit_markers, clear_stuck_cron_runs};
use crate::db::Database;
use rusqlite::params;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;
use uuid::Uuid;

async fn test_db() -> crate::db::Pool {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    db.pool().clone()
}

/// Seed a cron run row in the exact shape the insert path writes:
/// status='running' with an RFC3339 (+00:00) started_at.
async fn seed_run(pool: &crate::db::Pool, id: &str, started_at: chrono::DateTime<chrono::Utc>) {
    let id = id.to_string();
    let job_id = Uuid::new_v4().to_string();
    let ts = started_at.to_rfc3339();
    let parent_job_id = job_id.clone();
    pool.get()
        .await
        .unwrap()
        .interact(move |conn| {
            // Parent row first — cron_job_runs.job_id has an FK to cron_jobs(id).
            conn.execute(
                "INSERT INTO cron_jobs (id, name, cron_expr, timezone, prompt, thinking, \
                 auto_approve, enabled, created_at, updated_at) \
                 VALUES (?1, 'doctor-fix-test', '0 0 * * *', 'UTC', 'x', 'off', 1, 1, ?2, ?2)",
                params![parent_job_id, ts],
            )?;
            conn.execute(
                "INSERT INTO cron_job_runs (id, job_id, job_name, status, content, error, \
                 input_tokens, output_tokens, cost, provider, model, started_at, completed_at, created_at)
                 VALUES (?1, ?2, 'job', 'running', '', '', 0, 0, 0, '', '', ?3, NULL, ?3)",
                params![id, job_id, ts],
            )
        })
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_clear_stuck_cron_runs_marks_interrupted() {
    let pool = test_db().await;
    let old = Uuid::new_v4().to_string();
    let fresh = Uuid::new_v4().to_string();
    seed_run(&pool, &old, chrono::Utc::now() - chrono::Duration::hours(3)).await;
    seed_run(&pool, &fresh, chrono::Utc::now()).await;

    let n = clear_stuck_cron_runs(&pool, 3600).await.unwrap();
    assert_eq!(n, 1, "exactly the stale row should clear");

    let (old_status, old_error, fresh_status): (String, String, String) = pool
        .get()
        .await
        .unwrap()
        .interact(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id, status, error FROM cron_job_runs ORDER BY id")
                .unwrap();
            let rows: Vec<(String, String, Option<String>)> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .unwrap()
                .flatten()
                .collect();
            let get = |id: &str| {
                rows.iter()
                    .find(|(i, _, _)| i == id)
                    .map(|(_, s, e)| (s.clone(), e.clone().unwrap_or_default()))
                    .unwrap()
            };
            let (old_s, old_e) = get(&old);
            let (fresh_s, _) = get(&fresh);
            (old_s, old_e, fresh_s)
        })
        .await
        .unwrap();
    assert_eq!(
        old_status, "interrupted",
        "stuck row must be closed as interrupted"
    );
    assert!(
        old_error.contains("interrupted: cleared by doctor --fix"),
        "error message must reflect doctor fix interruption"
    );
    assert_eq!(fresh_status, "running", "live row must stay untouched");
}

fn make_marker(dir: &PathBuf, age: Option<Duration>) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(format!(".opencrabs_plan_{}.preinit", Uuid::new_v4()));
    std::fs::write(&p, "").unwrap();
    if let Some(age) = age {
        let aged = SystemTime::now() - age;
        let f = std::fs::File::open(&p).unwrap();
        f.set_modified(aged).unwrap();
    }
    p
}

#[test]
fn stale_preinit_markers_removed_fresh_kept() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("session");
    let stale = make_marker(&root, Some(Duration::from_secs(8 * 24 * 3600)));
    let fresh = make_marker(&root, None);
    // Unrelated files must never match the marker predicate.
    let noise = root.join(".opencrabs_plan_something.json");
    std::fs::write(&noise, "").unwrap();

    let removed = clear_stale_preinit_markers(&[root], Duration::from_secs(7 * 24 * 3600));
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].detail, stale.display().to_string());
    assert!(!stale.exists(), "stale marker must be gone");
    assert!(fresh.exists(), "fresh marker must survive");
    assert!(noise.exists(), "non-marker sibling must survive");
}

#[test]
fn missing_roster_roots_are_silent_noops() {
    let removed = clear_stale_preinit_markers(
        &[PathBuf::from("/nonexistent/definitely-gone")],
        Duration::from_secs(60),
    );
    assert!(removed.is_empty());
}

#[cfg(unix)]
#[test]
fn loose_permissions_tightened_only() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    std::fs::create_dir_all(home.join("brain")).unwrap();
    std::fs::create_dir_all(home.join("logs")).unwrap();

    let loose = home.join("brain/MEMORY.md");
    std::fs::write(&loose, "x").unwrap();
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
    let tight = home.join("logs/opencrabs.today");
    std::fs::write(&tight, "x").unwrap();
    std::fs::set_permissions(&tight, std::fs::Permissions::from_mode(0o600)).unwrap();

    let fixed = crate::cli::doctor_fix::fix_brain_log_permissions(home);
    assert_eq!(fixed.len(), 1);
    assert_eq!(fixed[0].detail, loose.display().to_string());
    let mode = std::fs::metadata(&loose).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}
