//! `/doctor --fix` repair engine (#1114).
//!
//! Detection + repair of common stuck states. Every action returns what it
//! changed so the CLI output doubles as the audit log (what, where, why).

use anyhow::Context;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One completed repair, rendered by `cmd_doctor`.
#[derive(Debug, PartialEq)]
pub(crate) struct FixReport {
    pub action: &'static str,
    pub detail: String,
}

/// Clear cron run rows stuck in `running` longer than `max_age_secs`.
///
/// A crash between insert and mark-finish leaves `status='running'` forever,
/// and anything reading runs treats those as live work (#1114). Rows only
/// enter `running` via the insert path, which writes RFC3339 `+00:00`
/// timestamps, so a lexicographic comparison against a cutoff in the same
/// format is exact for exactly this population.
pub async fn clear_stuck_cron_runs(
    pool: &crate::db::Pool,
    max_age_secs: i64,
) -> anyhow::Result<usize> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(max_age_secs)).to_rfc3339();
    let n = pool
        .get()
        .await
        .context("Failed to get connection")?
        .interact(move |conn| {
            conn.execute(
                "UPDATE cron_job_runs SET status='interrupted', \
                 error='interrupted: cleared by doctor --fix (no completion within max age)', \
                 completed_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') \
                 WHERE status='running' AND started_at < ?1",
                [cutoff],
            )
        })
        .await
        // InteractError doesn't implement std::error::Error, so map it manually
        .map_err(|_| anyhow::anyhow!("cron interact failed"))??;
    Ok(n)
}

/// Remove pre-init plan markers older than `max_age` under each root.
///
/// A `.opencrabs_plan_<uuid>.preinit` file means "entered plan intent, never
/// reached init"; a session that died there stays dead, so aged markers are
/// pure residue. Markers sit flat inside their session dir, so each root is
/// scanned one level deep. Fresh markers are never touched.
pub fn clear_stale_preinit_markers(roots: &[PathBuf], max_age: Duration) -> Vec<FixReport> {
    let mut removed = Vec::new();
    let cutoff = std::time::SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(std::time::UNIX_EPOCH);
    for root in roots {
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_preinit_marker(&path) {
                continue;
            }
            let stale = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| t < cutoff)
                .unwrap_or(false);
            if stale && std::fs::remove_file(&path).is_ok() {
                removed.push(FixReport {
                    action: "stale-preinit-marker",
                    detail: path.display().to_string(),
                });
            }
        }
    }
    removed
}

/// True for files named `.opencrabs_plan_<uuid>.preinit`.
fn is_preinit_marker(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with(".opencrabs_plan_") && n.ends_with(".preinit"))
            .unwrap_or(false)
}

/// Tighten brain/log files that are group- or world-accessible to 0600.
///
/// Brain files hold private context and logs can hold message content;
/// neither should ever be readable beyond the owner.
#[cfg(unix)]
pub fn fix_brain_log_permissions(home: &Path) -> Vec<FixReport> {
    use std::os::unix::fs::PermissionsExt;
    let mut fixed = Vec::new();
    for rel in ["brain", "logs"] {
        let entries = match std::fs::read_dir(home.join(rel)) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0
                && std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).is_ok()
            {
                fixed.push(FixReport {
                    action: "permissions-tightened",
                    detail: path.display().to_string(),
                });
            }
        }
    }
    fixed
}

/// A cron run stuck in `running` longer than this is dead (crash between
/// insert and mark-finish); anything shorter might still be live work.
pub const STUCK_CRON_MAX_AGE_SECS: i64 = 3600;

/// Pre-init markers older than this are residue: a session that entered
/// plan intent and never reached `init` within a week is not coming back.
pub const PREINIT_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// Run every repair and return the combined report (the audit trail).
///
/// Cron rows need the pool; markers and permissions are pure filesystem.
/// Callers render the reports: `cmd_doctor` prints them, the startup sweep
/// logs them.
#[cfg_attr(not(unix), allow(unused_variables))]
pub async fn run_all(
    pool: &crate::db::Pool,
    marker_roots: &[PathBuf],
    home: &Path,
) -> anyhow::Result<Vec<FixReport>> {
    let mut reports = Vec::new();
    let stuck = clear_stuck_cron_runs(pool, STUCK_CRON_MAX_AGE_SECS).await?;
    if stuck > 0 {
        reports.push(FixReport {
            action: "stuck-cron-rows-cleared",
            detail: format!("{stuck} row(s) marked interrupted"),
        });
    }
    reports.extend(clear_stale_preinit_markers(marker_roots, PREINIT_MAX_AGE));
    #[cfg(unix)]
    reports.extend(fix_brain_log_permissions(home));
    Ok(reports)
}
