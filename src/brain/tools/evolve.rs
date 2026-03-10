//! Evolve Tool
//!
//! Updates OpenCrabs to the latest release. Detects the install method
//! (pre-built binary, cargo install, or source build) and uses the
//! appropriate upgrade strategy:
//!
//! - **Pre-built binary**: Downloads from GitHub releases, health-checks, swaps.
//! - **cargo install**: Runs `cargo install opencrabs --force`.
//! - **Source build**: Suggests using `/rebuild` instead.
//!
//! Before swapping binaries, it health-checks the new binary. If the swap
//! fails, it rolls back to the previous version automatically.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::brain::agent::{ProgressCallback, ProgressEvent};
use crate::utils::install::{InstallMethod, binary_name, platform_suffix};
use async_trait::async_trait;
use serde_json::Value;

const GITHUB_API: &str = "https://api.github.com/repos/adolfousier/opencrabs/releases/latest";

/// Check GitHub for a newer release. Returns `Some(latest_version)` if an
/// update is available **and** a binary asset exists for this platform,
/// `None` if already on latest, no asset ready, or on error.
pub async fn check_for_update() -> Option<String> {
    let current_version = crate::VERSION;
    let client = reqwest::Client::new();
    let release: serde_json::Value = client
        .get(GITHUB_API)
        .header("User-Agent", format!("opencrabs/{}", current_version))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    let latest_tag = release["tag_name"].as_str()?;
    let latest_version = latest_tag.strip_prefix('v').unwrap_or(latest_tag);

    if !is_newer(latest_version, current_version) {
        return None;
    }

    // If running from source, check if Cargo.toml already has the latest version
    if let Some(source_version) = source_cargo_version()
        && source_version == latest_version
    {
        return None;
    }

    // For pre-built binary installs, only report "available" if the platform
    // asset actually exists in the release (release may still be building).
    if matches!(InstallMethod::detect(), InstallMethod::PrebuiltBinary)
        && !has_platform_asset(&release, latest_tag)
    {
        tracing::debug!(
            "Release {} exists but no asset for this platform yet",
            latest_tag
        );
        return None;
    }

    Some(latest_version.to_string())
}

/// Check whether the release JSON contains a downloadable asset for the
/// current platform.
fn has_platform_asset(release: &serde_json::Value, tag: &str) -> bool {
    let suffix = match platform_suffix() {
        Some(s) => s,
        None => return false,
    };
    let ext = if std::env::consts::OS == "windows" {
        "zip"
    } else {
        "tar.gz"
    };
    let expected = format!("opencrabs-{}-{}.{}", tag, suffix, ext);
    let legacy = format!("opencrabs-{}.{}", suffix, ext);

    release["assets"]
        .as_array()
        .map(|arr| {
            arr.iter().any(|a| {
                let name = a["name"].as_str().unwrap_or("");
                name == expected || name == legacy
            })
        })
        .unwrap_or(false)
}

/// Compare semver strings: returns true if `latest` is strictly newer than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split('.').filter_map(|s| s.parse().ok()).collect() };
    let l = parse(latest);
    let c = parse(current);
    l > c
}

/// Try to read the version from the source Cargo.toml relative to the running
/// binary. Returns `None` if not running from a source build or file not found.
fn source_cargo_version() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let target_dir = exe.parent()?;
    let repo_root = target_dir.parent()?.parent()?;
    let cargo_toml = repo_root.join("Cargo.toml");
    let content = std::fs::read_to_string(&cargo_toml).ok()?;
    let table: toml::Table = content.parse().ok()?;
    table
        .get("package")?
        .get("version")?
        .as_str()
        .map(String::from)
}

/// Run a health check on a binary: execute it with `--version`,
/// verify it exits cleanly within a timeout. Returns a detailed error
/// with stderr output on failure.
async fn health_check_binary(path: &std::path::Path) -> std::result::Result<(), String> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new(path)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => Ok(()),
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr_snippet: String = stderr.chars().take(200).collect();
            Err(format!(
                "exited with {} (binary: {} bytes, platform: {}/{}{})",
                output.status,
                file_size,
                std::env::consts::OS,
                std::env::consts::ARCH,
                if stderr_snippet.is_empty() {
                    String::new()
                } else {
                    format!(", stderr: {}", stderr_snippet)
                }
            ))
        }
        Ok(Err(e)) => Err(format!(
            "failed to spawn: {} (binary: {} bytes, platform: {}/{})",
            e,
            file_size,
            std::env::consts::OS,
            std::env::consts::ARCH
        )),
        Err(_) => Err(format!("timed out after 10s (binary: {} bytes)", file_size)),
    }
}

pub struct EvolveTool {
    progress: Option<ProgressCallback>,
}

impl EvolveTool {
    pub fn new(progress: Option<ProgressCallback>) -> Self {
        Self { progress }
    }
}

#[async_trait]
impl Tool for EvolveTool {
    fn name(&self) -> &str {
        "evolve"
    }

    fn description(&self) -> &str {
        "Check for and install the latest OpenCrabs release. \
         Automatically detects the install method (pre-built binary, \
         cargo install, or source) and uses the right update strategy. \
         Hot-restarts into the new version after installation."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "check_only": {
                    "type": "boolean",
                    "description": "If true, only check for updates without installing. Default: false."
                }
            },
            "required": []
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let check_only = input
            .get("check_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let current_version = crate::VERSION;
        let sid = context.session_id;
        let install_method = InstallMethod::detect();

        // Emit progress
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::IntermediateText {
                    text: format!(
                        "Checking for updates (install: {})...",
                        install_method.description()
                    ),
                    reasoning: None,
                },
            );
        }

        // Fetch latest release info from GitHub
        let client = reqwest::Client::new();
        let release: Value = match client
            .get(GITHUB_API)
            .header("User-Agent", format!("opencrabs/{}", current_version))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Failed to parse release info: {}",
                        e
                    )));
                }
            },
            Ok(resp) => {
                return Ok(ToolResult::error(format!(
                    "GitHub API returned {}: rate limited or unavailable",
                    resp.status()
                )));
            }
            Err(e) => return Ok(ToolResult::error(format!("Failed to reach GitHub: {}", e))),
        };

        let latest_tag = release["tag_name"].as_str().unwrap_or("unknown");
        let latest_version = latest_tag.strip_prefix('v').unwrap_or(latest_tag);

        // Compare versions
        if latest_version == current_version {
            return Ok(ToolResult::success(format!(
                "Already on the latest version (v{}).",
                current_version
            )));
        }

        // For pre-built binary installs, verify the platform asset exists
        // before reporting the update as available (release may still be building).
        if matches!(install_method, InstallMethod::PrebuiltBinary)
            && !has_platform_asset(&release, latest_tag)
        {
            let asset_count = release["assets"].as_array().map(|a| a.len()).unwrap_or(0);
            return Ok(ToolResult::error(format!(
                "v{} release exists but the binary for {}/{} is not available yet \
                 ({} assets uploaded so far). The release may still be building — \
                 try again in a few minutes.",
                latest_version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                asset_count
            )));
        }

        if check_only {
            return Ok(ToolResult::success(format!(
                "Update available: v{} -> v{} (install method: {}). Run /evolve to install.",
                current_version,
                latest_version,
                install_method.description()
            )));
        }

        // Dispatch based on install method
        match install_method {
            InstallMethod::Source(_) => {
                return Ok(ToolResult::success(format!(
                    "Update available: v{} -> v{}. You're running from source — use /rebuild \
                     to pull and build the latest version, or `git checkout v{}` to switch.",
                    current_version, latest_version, latest_version
                )));
            }
            InstallMethod::CargoInstall => {
                return self
                    .evolve_via_cargo_install(sid, current_version, latest_version)
                    .await;
            }
            InstallMethod::PrebuiltBinary => {
                return self
                    .evolve_via_binary_download(
                        sid,
                        &client,
                        &release,
                        current_version,
                        latest_tag,
                        latest_version,
                    )
                    .await;
            }
        }
    }
}

impl EvolveTool {
    /// Update via `cargo install opencrabs --force`.
    async fn evolve_via_cargo_install(
        &self,
        sid: uuid::Uuid,
        current_version: &str,
        latest_version: &str,
    ) -> Result<ToolResult> {
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::IntermediateText {
                    text: format!(
                        "Updating via cargo install (v{} -> v{})...",
                        current_version, latest_version
                    ),
                    reasoning: None,
                },
            );
        }

        let output = tokio::process::Command::new("cargo")
            .args(["install", "opencrabs", "--force"])
            .output()
            .await
            .map_err(|e| {
                super::error::ToolError::Execution(format!("Failed to spawn cargo: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Ok(ToolResult::error(format!(
                "cargo install failed: {}",
                stderr.chars().take(500).collect::<String>()
            )));
        }

        // Signal restart
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::RestartReady {
                    status: format!(
                        "Evolved via cargo install: v{} -> v{}. Restarting now.",
                        current_version, latest_version
                    ),
                },
            );
        }

        Ok(ToolResult::success(format!(
            "Evolved from v{} to v{} via cargo install. Restarting into the new version.",
            current_version, latest_version
        )))
    }

    /// Update by downloading a pre-built binary from GitHub releases.
    async fn evolve_via_binary_download(
        &self,
        sid: uuid::Uuid,
        client: &reqwest::Client,
        release: &Value,
        current_version: &str,
        latest_tag: &str,
        latest_version: &str,
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

        let archive_bytes = match client.get(&download_url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(b) if b.is_empty() => {
                    return Ok(ToolResult::error(
                        "Download returned empty file. The release asset may still be uploading — \
                         try again in a few minutes."
                            .into(),
                    ));
                }
                Ok(b) => b,
                Err(e) => return Ok(ToolResult::error(format!("Download failed: {}", e))),
            },
            Ok(resp) => {
                return Ok(ToolResult::error(format!(
                    "Download failed with status {}",
                    resp.status()
                )));
            }
            Err(e) => return Ok(ToolResult::error(format!("Download failed: {}", e))),
        };

        tracing::info!(
            "Downloaded {} ({} bytes)",
            expected_asset,
            archive_bytes.len()
        );

        // Extract
        let bin_name = binary_name();
        let binary_data = if is_windows {
            extract_from_zip(&archive_bytes, bin_name)?
        } else {
            extract_from_tar_gz(&archive_bytes, bin_name)?
        };

        // Locate current executable
        let exe_path = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Cannot locate current binary: {}",
                    e
                )));
            }
        };

        // Write temp file
        let tmp_path = exe_path.with_extension("evolve_tmp");
        if let Err(e) = tokio::fs::write(&tmp_path, &binary_data).await {
            return Ok(ToolResult::error(format!(
                "Failed to write new binary: {}",
                e
            )));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            if let Err(e) = std::fs::set_permissions(&tmp_path, perms) {
                let _ = std::fs::remove_file(&tmp_path);
                return Ok(ToolResult::error(format!(
                    "Failed to set permissions: {}",
                    e
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
            let _ = std::fs::remove_file(&tmp_path);
            return Ok(ToolResult::error(format!(
                "Health check failed ({}). Keeping current v{}.",
                reason, current_version
            )));
        }

        // Backup
        let backup_path = exe_path.with_extension("evolve_backup");
        if let Err(e) = std::fs::copy(&exe_path, &backup_path) {
            tracing::warn!("Could not create backup of current binary: {}", e);
        }

        // Atomic rename
        if let Err(e) = std::fs::rename(&tmp_path, &exe_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Ok(ToolResult::error(format!(
                "Failed to replace binary: {}",
                e
            )));
        }

        // Post-swap verification
        if let Err(reason) = health_check_binary(&exe_path).await {
            if backup_path.exists() {
                if let Err(e) = std::fs::rename(&backup_path, &exe_path) {
                    return Ok(ToolResult::error(format!(
                        "CRITICAL: New binary failed ({}) AND rollback failed: {}. Manual recovery needed.",
                        reason, e
                    )));
                }
                return Ok(ToolResult::error(format!(
                    "New binary failed post-swap ({}). Rolled back to v{}.",
                    reason, current_version
                )));
            }
            return Ok(ToolResult::error(format!(
                "New binary failed post-swap ({}). No backup for rollback.",
                reason
            )));
        }

        let _ = std::fs::remove_file(&backup_path);

        // Signal restart
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::RestartReady {
                    status: format!(
                        "Evolved: v{} -> v{}. Restarting now.",
                        current_version, latest_version
                    ),
                },
            );
        }

        Ok(ToolResult::success(format!(
            "Evolved from v{} to v{}. Restarting into the new version.",
            current_version, latest_version
        )))
    }
}

/// Extract a named file from a .tar.gz archive in memory.
fn extract_from_tar_gz(data: &[u8], file_name: &str) -> Result<Vec<u8>> {
    use std::io::Read;

    let decoder = flate2::read::GzDecoder::new(data);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|e| super::error::ToolError::Execution(format!("Failed to read archive: {}", e)))?
    {
        let mut entry = entry.map_err(|e| {
            super::error::ToolError::Execution(format!("Failed to read entry: {}", e))
        })?;

        let path = entry
            .path()
            .map_err(|e| {
                super::error::ToolError::Execution(format!("Invalid path in archive: {}", e))
            })?
            .to_path_buf();

        if path.file_name().and_then(|n| n.to_str()) == Some(file_name) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| {
                super::error::ToolError::Execution(format!("Failed to extract: {}", e))
            })?;
            return Ok(buf);
        }
    }

    Err(super::error::ToolError::Execution(format!(
        "'{}' not found in archive",
        file_name
    )))
}

/// Extract a named file from a .zip archive in memory.
fn extract_from_zip(data: &[u8], file_name: &str) -> Result<Vec<u8>> {
    use std::io::Read;

    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| super::error::ToolError::Execution(format!("Failed to read zip: {}", e)))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| {
            super::error::ToolError::Execution(format!("Failed to read zip entry: {}", e))
        })?;

        if file.name().ends_with(file_name) {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).map_err(|e| {
                super::error::ToolError::Execution(format!("Failed to extract: {}", e))
            })?;
            return Ok(buf);
        }
    }

    Err(super::error::ToolError::Execution(format!(
        "'{}' not found in zip",
        file_name
    )))
}
