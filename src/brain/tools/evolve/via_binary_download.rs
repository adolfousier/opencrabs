//! The pre-built-binary upgrade strategy: download the release asset for
//! this platform, health-check it, swap it over the running binary with
//! rollback on failure, then schedule the restart.

use super::super::error::Result;
use super::super::r#trait::ToolResult;
use super::EvolveTool;
use super::archive::{extract_from_tar_gz, extract_from_zip};
use super::binary_health::{get_binary_migration_count, health_check_binary};
use super::restart_status::RestartStatus;
use super::systemd::{
    EVOLVE_UNIT_GLOB, SYSTEMD_UNIT_PATTERN, build_systemd_cleanup_command,
    build_systemd_restart_command, count_matching_systemd_units,
};
use crate::brain::agent::ProgressEvent;
use crate::utils::install::{binary_name, platform_suffix};
use serde_json::Value;

impl EvolveTool {
    /// Update by downloading a pre-built binary from GitHub releases.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn evolve_via_binary_download(
        &self,
        sid: uuid::Uuid,
        client: &reqwest::Client,
        release: &Value,
        current_version: &str,
        latest_tag: &str,
        latest_version: &str,
        current_user_version: Option<i64>,
    ) -> Result<ToolResult> {
        let suffix = match platform_suffix() {
            Some(s) => s,
            None => {
                return Ok(ToolResult::error(format!(
                    "Unsupported platform: {}/{}. Use /rebuild to build from source.",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )));
            }
        };

        // Root-swap opt-out (OC-03). Default allows it, so an existing root
        // daemon keeps auto-updating and a forgotten flag never breaks updates.
        // Only an operator who explicitly set evolve_allow_root=false is
        // refused here, with a note on what to do. The checksum verification
        // below is the real integrity gate and runs regardless.
        if super::verify::running_as_root()
            && !crate::config::Config::current().agent.evolve_allow_root
        {
            return Ok(ToolResult::error(
                "Refusing to self-update while running as root: [agent] evolve_allow_root is \
                 false. A root binary swap means a compromised release would run as root. To \
                 proceed, set [agent] evolve_allow_root = true (the default), or run /rebuild to \
                 build from source under a non-root user."
                    .to_string(),
            ));
        }

        let is_windows = std::env::consts::OS == "windows";
        let ext = if is_windows { "zip" } else { "tar.gz" };
        let expected_asset = format!("opencrabs-{}-{}.{}", latest_tag, suffix, ext);

        let assets = release["assets"].as_array();
        let download_url = assets
            .and_then(|arr| {
                arr.iter().find_map(|a| {
                    let name = a["name"].as_str()?;
                    if name == expected_asset {
                        a["browser_download_url"].as_str().map(String::from)
                    } else {
                        None
                    }
                })
            })
            .or_else(|| {
                // Fallback: try legacy naming without version tag
                let legacy_asset = format!("opencrabs-{}.{}", suffix, ext);
                assets.and_then(|arr| {
                    arr.iter().find_map(|a| {
                        let name = a["name"].as_str()?;
                        if name == legacy_asset {
                            a["browser_download_url"].as_str().map(String::from)
                        } else {
                            None
                        }
                    })
                })
            });

        let download_url = match download_url {
            Some(url) => url,
            None => {
                return Ok(ToolResult::error(format!(
                    "No binary found for {} in v{}. Expected: {}. \
                     Available assets: {}. Use /rebuild to build from source.",
                    suffix,
                    latest_version,
                    expected_asset,
                    assets
                        .map(|arr| arr
                            .iter()
                            .filter_map(|a| a["name"].as_str())
                            .collect::<Vec<_>>()
                            .join(", "))
                        .unwrap_or_default()
                )));
            }
        };

        // Pin the asset host before fetching (OC-03): a release JSON whose
        // browser_download_url points off-GitHub must not be fetched at all.
        if !super::verify::is_allowed_download_host(&download_url) {
            return Ok(ToolResult::error(format!(
                "Refusing to download the release asset from an unexpected host: {download_url}. \
                 Use /rebuild to build from source."
            )));
        }

        // Download
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::IntermediateText {
                    text: format!("Downloading opencrabs v{}...", latest_version),
                    reasoning: None,
                },
            );
        }

        tracing::info!(
            target: "evolve",
            url = %download_url,
            expected_asset = %expected_asset,
            session_id = %sid,
            "evolve: downloading release asset"
        );
        let archive_bytes = match client.get(&download_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let content_length = resp.content_length();
                match resp.bytes().await {
                    Ok(b) if b.is_empty() => {
                        tracing::warn!(
                            target: "evolve",
                            url = %download_url,
                            content_length = ?content_length,
                            session_id = %sid,
                            "evolve: download returned empty body"
                        );
                        return Ok(ToolResult::error(format!(
                            "Download from {download_url} returned an empty file \
                             (content-length={content_length:?}). The release asset \
                             may still be uploading — try again in a few minutes."
                        )));
                    }
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!(
                            target: "evolve",
                            url = %download_url,
                            error = %e,
                            session_id = %sid,
                            "evolve: download body read failed"
                        );
                        return Ok(ToolResult::error(format!(
                            "Download from {download_url} failed mid-stream: {e}"
                        )));
                    }
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                let body_excerpt: String = body.chars().take(200).collect();
                tracing::warn!(
                    target: "evolve",
                    url = %download_url,
                    %status,
                    body_excerpt = %body_excerpt,
                    session_id = %sid,
                    "evolve: download returned non-2xx"
                );
                return Ok(ToolResult::error(format!(
                    "Download from {download_url} failed with status {status}{}",
                    if body_excerpt.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" — body: {body_excerpt}")
                    }
                )));
            }
            Err(e) => {
                tracing::warn!(
                    target: "evolve",
                    url = %download_url,
                    error = %e,
                    session_id = %sid,
                    "evolve: download request failed to send"
                );
                return Ok(ToolResult::error(format!(
                    "Download from {download_url} failed: {e}"
                )));
            }
        };

        tracing::info!(
            target: "evolve",
            asset = %expected_asset,
            bytes = archive_bytes.len(),
            session_id = %sid,
            "evolve: download complete"
        );

        // Integrity: verify the archive against the release's SHA256SUMS before
        // it is ever extracted or swapped in (OC-03). The prior gate was only a
        // `--version` health probe, which a hostile binary passes. Fails closed:
        // no SHA256SUMS, no entry, or a mismatch aborts the swap. The asset name
        // in SHA256SUMS is the download URL's basename.
        let sums_asset =
            super::verify::asset_basename(&download_url).unwrap_or_else(|| expected_asset.clone());
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::IntermediateText {
                    text: "Verifying release checksum...".into(),
                    reasoning: None,
                },
            );
        }
        if let Err(reason) =
            super::verify::verify_asset_checksum(client, release, &sums_asset, &archive_bytes).await
        {
            tracing::error!(
                target: "evolve",
                asset = %sums_asset,
                %reason,
                session_id = %sid,
                "evolve: checksum verification failed, refusing to swap"
            );
            return Ok(ToolResult::error(format!(
                "Refusing to update: {reason}. Use /rebuild to build from source."
            )));
        }
        tracing::info!(
            target: "evolve",
            asset = %sums_asset,
            session_id = %sid,
            "evolve: checksum verified against SHA256SUMS"
        );

        // Extract
        let bin_name = binary_name();
        let binary_data = if is_windows {
            extract_from_zip(&archive_bytes, bin_name)?
        } else {
            extract_from_tar_gz(&archive_bytes, bin_name)?
        };

        // Locate current executable. Use running_binary_path() (not raw
        // current_exe()) so a retry after a previous in-place swap doesn't
        // capture a "<path> (deleted)" path and write the next binary to a
        // literal "opencrabs (deleted)" file.
        let exe_path = match crate::brain::self_update::running_binary_path() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    target: "evolve",
                    error = %e,
                    session_id = %sid,
                    "evolve: current_exe() failed — cannot locate running binary"
                );
                return Ok(ToolResult::error(format!(
                    "Cannot locate current binary: {e}"
                )));
            }
        };

        // Write temp file. The temp name is process-unique so a second
        // opencrabs process evolving at the same time (the in-process
        // single-flight guard can't see other processes) doesn't write to and
        // exec the same file — that collision produced the ETXTBSY/ENOENT
        // health-check failures.
        let tmp_path = exe_path.with_extension(format!("evolve_tmp.{}", std::process::id()));
        if let Err(e) = tokio::fs::write(&tmp_path, &binary_data).await {
            tracing::warn!(
                target: "evolve",
                tmp_path = %tmp_path.display(),
                error = %e,
                session_id = %sid,
                "evolve: failed to write temp binary"
            );
            return Ok(ToolResult::error(format!(
                "Failed to write new binary to {}: {e}",
                tmp_path.display()
            )));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            if let Err(e) = std::fs::set_permissions(&tmp_path, perms) {
                tracing::warn!(
                    target: "evolve",
                    tmp_path = %tmp_path.display(),
                    error = %e,
                    session_id = %sid,
                    "evolve: failed to set 0o755 on temp binary"
                );
                let _ = std::fs::remove_file(&tmp_path);
                return Ok(ToolResult::error(format!(
                    "Failed to set permissions on {}: {e}",
                    tmp_path.display()
                )));
            }
        }

        // Health-check before swap
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::IntermediateText {
                    text: "Verifying new binary...".into(),
                    reasoning: None,
                },
            );
        }

        if let Err(reason) = health_check_binary(&tmp_path).await {
            tracing::warn!(
                target: "evolve",
                tmp_path = %tmp_path.display(),
                %reason,
                session_id = %sid,
                "evolve: pre-swap health check failed, discarding new binary"
            );
            let _ = std::fs::remove_file(&tmp_path);
            return Ok(ToolResult::error(format!(
                "Health check failed ({reason}). Keeping current v{current_version}."
            )));
        }

        // Migration compatibility check: ensure new binary can handle the current DB schema
        if let Some(db_version) = current_user_version {
            match get_binary_migration_count(&tmp_path).await {
                Ok(new_migration_count) => {
                    let db_migration = db_version as usize;
                    if db_migration > new_migration_count {
                        tracing::warn!(
                            target: "evolve",
                            db_user_version = db_version,
                            new_binary_migration_count = new_migration_count,
                            session_id = %sid,
                            "evolve: database schema v{} is newer than new binary's migration \
                             count v{} — refusing to swap",
                            db_migration,
                            new_migration_count,
                        );
                        let _ = std::fs::remove_file(&tmp_path);
                        return Ok(ToolResult::error(format!(
                            "Database schema v{db_migration} is newer than v{latest_version}'s \
                             migration support (v{new_migration_count}). \
                             Keeping current v{current_version}. \
                             This usually means the release predates your database schema."
                        )));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "evolve",
                        error = %e,
                        session_id = %sid,
                        "evolve: could not determine new binary's migration count, \
                         skipping compatibility check"
                    );
                }
            }
        }

        // Backup
        let backup_path = exe_path.with_extension("evolve_backup");
        if let Err(e) = std::fs::copy(&exe_path, &backup_path) {
            tracing::warn!(
                target: "evolve",
                exe_path = %exe_path.display(),
                backup_path = %backup_path.display(),
                error = %e,
                session_id = %sid,
                "evolve: backup copy failed — rollback will not be possible if swap goes bad"
            );
        }

        // Unlink old binary first so the directory entry is freed. On Linux,
        // rename(2) by itself already replaces the directory entry atomically
        // without touching the old inode (the running process keeps its mapped
        // memory).  We still do remove_file first as a belt-and-suspenders
        // guard against NFS / FUSE mounts where rename(2) may behave
        // differently when the target is a running executable.
        //
        // Failure here is non-fatal: if exe_path is already gone or we lack
        // permission, the rename below will surface the real error. Logged
        // at debug so a future incident can still see whether the unlink
        // succeeded (helps distinguish "rename failed because exe was
        // busy" from "rename failed because directory is read-only" etc.).
        if let Err(e) = std::fs::remove_file(&exe_path) {
            tracing::debug!(
                target: "evolve",
                exe_path = %exe_path.display(),
                error = %e,
                session_id = %sid,
                "evolve: pre-rename unlink failed (non-fatal; rename will report the real error if any)"
            );
        }
        if let Err(e) = std::fs::rename(&tmp_path, &exe_path) {
            tracing::warn!(
                target: "evolve",
                tmp_path = %tmp_path.display(),
                exe_path = %exe_path.display(),
                error = %e,
                session_id = %sid,
                "evolve: atomic rename of tmp -> exe failed"
            );
            let _ = std::fs::remove_file(&tmp_path);
            return Ok(ToolResult::error(format!(
                "Failed to replace binary at {}: {e}",
                exe_path.display()
            )));
        }

        // Post-swap verification
        if let Err(reason) = health_check_binary(&exe_path).await {
            if backup_path.exists() {
                if let Err(e) = std::fs::rename(&backup_path, &exe_path) {
                    tracing::error!(
                        target: "evolve",
                        exe_path = %exe_path.display(),
                        backup_path = %backup_path.display(),
                        post_swap_reason = %reason,
                        rollback_error = %e,
                        session_id = %sid,
                        "evolve: CRITICAL — post-swap health check failed AND rollback failed; \
                         binary at exe_path is broken and backup could not be restored. \
                         Manual recovery needed."
                    );
                    return Ok(ToolResult::error(format!(
                        "CRITICAL: New binary failed ({reason}) AND rollback failed: {e}. \
                         Manual recovery needed (backup is at {}).",
                        backup_path.display()
                    )));
                }
                tracing::error!(
                    target: "evolve",
                    exe_path = %exe_path.display(),
                    post_swap_reason = %reason,
                    session_id = %sid,
                    "evolve: post-swap health check failed, rolled back to previous version"
                );
                return Ok(ToolResult::error(format!(
                    "New binary failed post-swap ({reason}). Rolled back to v{current_version}."
                )));
            }
            tracing::error!(
                target: "evolve",
                exe_path = %exe_path.display(),
                post_swap_reason = %reason,
                session_id = %sid,
                "evolve: post-swap health check failed and no backup exists for rollback"
            );
            return Ok(ToolResult::error(format!(
                "New binary failed post-swap ({reason}). No backup for rollback."
            )));
        }

        let _ = std::fs::remove_file(&backup_path);

        // Extract the bundled RTK binary from the same archive.
        // The release workflow packs `rtk` alongside `opencrabs` into the
        // release asset. This is best-effort: older releases without RTK
        // or extraction failures should not block the evolve.
        let rtk_bin_name = if is_windows { "rtk.exe" } else { "rtk" };
        let rtk_result = if is_windows {
            extract_from_zip(&archive_bytes, rtk_bin_name)
        } else {
            extract_from_tar_gz(&archive_bytes, rtk_bin_name)
        };
        match rtk_result {
            Ok(rtk_data) => {
                let rtk_path = exe_path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join(rtk_bin_name);
                match tokio::fs::write(&rtk_path, &rtk_data).await {
                    Ok(()) => {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let perms = std::fs::Permissions::from_mode(0o755);
                            if let Err(e) = std::fs::set_permissions(&rtk_path, perms) {
                                tracing::warn!(
                                    target: "evolve",
                                    rtk_path = %rtk_path.display(),
                                    error = %e,
                                    "evolve: extracted RTK but failed to set executable permissions"
                                );
                            }
                        }
                        tracing::info!(
                            target: "evolve",
                            rtk_path = %rtk_path.display(),
                            bytes = rtk_data.len(),
                            session_id = %sid,
                            "evolve: extracted and installed bundled RTK binary"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "evolve",
                            rtk_path = %rtk_path.display(),
                            error = %e,
                            "evolve: extracted RTK data but failed to write binary to disk"
                        );
                    }
                }
            }
            Err(e) => {
                // Older releases may not bundle RTK — not a failure
                tracing::debug!(
                    target: "evolve",
                    error = %e,
                    session_id = %sid,
                    "evolve: no RTK binary found in release archive (expected in newer releases)"
                );
            }
        }

        // Schedule a delayed daemon restart for systemd-managed services.
        // This runs 3 seconds after the tool returns, giving the current
        // response enough time to be delivered before the daemon exits.
        //
        // We use systemd-run --on-active=N, which creates a transient timer
        // unit tracked by PID 1, outside our service cgroup.  This means the
        // timer survives even after `systemctl restart opencrabs*.service`
        // kills the current process.
        //
        // Only units matching the glob pattern are restarted, so adding a
        // new profile (e.g. opencrabs-staging.service) picks it up
        // automatically with no code change.
        //
        // Pre-flight: count units that match the glob. If zero match,
        // the scheduled `systemctl restart` would be a no-op — same
        // user-visible symptom as #136 (agent says "Evolved!", daemon
        // never restarts) but for a different reason (unit name
        // mismatch instead of missing restart). Skip the spawn and
        // tell the user honestly.
        //
        // OpenCrabs is commonly installed as a user-level systemd service
        // (`systemctl --user`), so if system-level units return 0 we
        // fall through and check user-level units too.
        let mut restart_status = RestartStatus::NotSystemd;
        let mut use_user_units = false;
        if std::path::Path::new("/run/systemd/system").exists() {
            let mut unit_count = count_matching_systemd_units(SYSTEMD_UNIT_PATTERN, false);
            if unit_count == Some(0) {
                // No system-level units matched — try user-level.
                // OpenCrabs's `install_systemd_service()` writes to
                // ~/.config/systemd/user/ and uses `systemctl --user`.
                let user_count = count_matching_systemd_units(SYSTEMD_UNIT_PATTERN, true);
                match user_count {
                    Some(n) if n > 0 => {
                        use_user_units = true;
                        unit_count = Some(n);
                        tracing::info!(
                            target: "evolve",
                            pattern = SYSTEMD_UNIT_PATTERN,
                            user_units = n,
                            session_id = %sid,
                            "evolve: no system-level units found, using {n} user-level units — scheduling restart with --user"
                        );
                    }
                    _ => {
                        // Still 0 or None — keep unit_count as Some(0)
                    }
                }
            }
            match unit_count {
                Some(0) => {
                    tracing::warn!(
                        target: "evolve",
                        pattern = SYSTEMD_UNIT_PATTERN,
                        session_id = %sid,
                        "evolve: no systemd units matched the pattern (checked system and user level) — skipping scheduled restart"
                    );
                    restart_status = RestartStatus::NoUnitsMatched;
                }
                _ => {
                    // Either Some(n>=1) or None ("don't know" — systemctl
                    // failed to spawn / returned non-zero). In the None
                    // case, fall through and schedule the restart anyway:
                    // a diagnostic failure shouldn't penalize the user
                    // whose daemon DOES exist and DOES match the glob.
                    if let Some(n) = unit_count {
                        tracing::info!(
                            target: "evolve",
                            pattern = SYSTEMD_UNIT_PATTERN,
                            matched_units = n,
                            use_user_units,
                            session_id = %sid,
                            "evolve: pre-flight found matching systemd units, scheduling restart (+3s)"
                        );
                    } else {
                        tracing::warn!(
                            target: "evolve",
                            pattern = SYSTEMD_UNIT_PATTERN,
                            session_id = %sid,
                            "evolve: could not determine matching unit count (systemctl spawn failed), \
                             scheduling restart anyway"
                        );
                    }
                    let pid = std::process::id();
                    let unit_name = format!("opencrabs-evolve-{pid}");
                    // Garbage-collect spent evolve units from prior runs before
                    // scheduling a fresh one. Without --collect (unsupported on
                    // old systemd) finished/failed `opencrabs-evolve-<pid>`
                    // units pile up indefinitely; reset-failed clears them and
                    // works on every systemd we target. Best-effort — a cleanup
                    // failure must not block the restart.
                    match build_systemd_cleanup_command(use_user_units).status() {
                        Ok(st) => tracing::info!(
                            target: "evolve",
                            glob = EVOLVE_UNIT_GLOB,
                            success = st.success(),
                            session_id = %sid,
                            "evolve: reset-failed swept stale evolve units"
                        ),
                        Err(e) => tracing::warn!(
                            target: "evolve",
                            glob = EVOLVE_UNIT_GLOB,
                            error = %e,
                            session_id = %sid,
                            "evolve: could not sweep stale evolve units (reset-failed spawn failed) — \
                             they will linger until manually cleared, but the restart still proceeds"
                        ),
                    }
                    // Failure to spawn systemd-run is the most user-visible
                    // regression mode: the binary on disk is updated, the
                    // agent says "Evolved!", but the daemon keeps running
                    // the old inode forever because no restart was ever
                    // scheduled. Log at warn so the user has actionable
                    // forensic evidence when "evolve said success but
                    // didn't restart" happens — exactly the symptom this
                    // whole code path was added to prevent (#136).
                    match build_systemd_restart_command(pid, use_user_units).spawn() {
                        Ok(child) => {
                            tracing::info!(
                                target: "evolve",
                                unit = %unit_name,
                                systemd_run_pid = child.id(),
                                session_id = %sid,
                                "evolve: systemd-run spawned; daemon will restart in 3s"
                            );
                            restart_status = RestartStatus::Scheduled;
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "evolve",
                                unit = %unit_name,
                                error = %e,
                                session_id = %sid,
                                "evolve: failed to spawn systemd-run — daemon will NOT auto-restart, \
                                 manual `systemctl restart opencrabs*.service` (or `systemctl --user restart` \
                                 for user services) is required to load the new binary"
                            );
                            restart_status = RestartStatus::SpawnFailed(e.to_string());
                        }
                    }
                }
            }
        }

        // Signal restart
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::RestartReady {
                    status: format!(
                        "Evolved: v{} -> v{}. Restarting now.",
                        current_version, latest_version
                    ),
                    // Hand the restart the exact path we just wrote the new
                    // binary to — captured BEFORE the unlink/rename swap. If we
                    // passed None the handler would re-resolve via
                    // current_exe() AFTER the swap, and /proc/self/exe reads
                    // back as "<path> (deleted)" (the old inode was unlinked),
                    // so the exec would ENOENT on a literal "… (deleted)" path.
                    binary_path: Some(exe_path.clone()),
                },
            );
        }

        Ok(ToolResult::success(
            restart_status.user_message(current_version, latest_version),
        ))
    }
}
