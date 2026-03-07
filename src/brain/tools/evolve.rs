//! Evolve Tool
//!
//! Downloads the latest OpenCrabs release binary from GitHub, replaces the
//! current executable, and exec()-restarts into the new version.
//! The crab molts its shell and wakes up evolved.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::brain::agent::{ProgressCallback, ProgressEvent};
use async_trait::async_trait;
use serde_json::Value;

const GITHUB_API: &str = "https://api.github.com/repos/adolfousier/opencrabs/releases/latest";

/// Resolves the asset suffix for the current platform.
fn platform_suffix() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("macos", "x86_64") => Some("macos-amd64"),
        ("linux", "x86_64") => Some("linux-amd64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("windows", "x86_64") => Some("windows-amd64"),
        _ => None,
    }
}

/// Check GitHub for a newer release. Returns `Some(latest_version)` if an
/// update is available, `None` if already on latest (or on error).
///
/// When running from source, the compiled-in version may lag behind the local
/// `Cargo.toml` (e.g. after `git pull` but before `cargo build`). In that case
/// we also check the source Cargo.toml — if it already matches the latest
/// release, we suppress the update notice to avoid false positives.
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

    if latest_version == current_version {
        return None;
    }

    // If running from source, check if Cargo.toml already has the latest version
    // (user pulled but hasn't rebuilt yet — not a real update)
    if let Some(source_version) = source_cargo_version()
        && source_version == latest_version
    {
        return None;
    }

    Some(latest_version.to_string())
}

/// Try to read the version from the source Cargo.toml relative to the running
/// binary. Returns `None` if not running from a source build or file not found.
fn source_cargo_version() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    // Source builds live in target/release/ or target/debug/ under the repo root
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
        "Check for and install the latest OpenCrabs release from GitHub. \
         Downloads the pre-built binary for the current platform and \
         hot-restarts into the new version. Use this to update OpenCrabs \
         without building from source."
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

        // Emit progress
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::IntermediateText {
                    text: "Checking for updates...".into(),
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

        if check_only {
            return Ok(ToolResult::success(format!(
                "Update available: v{} -> v{}. Run /evolve to install.",
                current_version, latest_version
            )));
        }

        // Determine platform asset
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

        // Find the matching asset in the release
        // Asset naming: opencrabs-v{version}-{suffix}.tar.gz (or .zip for Windows)
        let is_windows = std::env::consts::OS == "windows";
        let ext = if is_windows { "zip" } else { "tar.gz" };
        let expected_asset = format!("opencrabs-{}-{}.{}", latest_tag, suffix, ext);

        let assets = release["assets"].as_array();
        let download_url = assets.and_then(|arr| {
            arr.iter().find_map(|a| {
                let name = a["name"].as_str()?;
                if name == expected_asset {
                    a["browser_download_url"].as_str().map(String::from)
                } else {
                    None
                }
            })
        });

        // Fallback: try legacy naming without version (opencrabs-{suffix}.tar.gz)
        let download_url = match download_url {
            Some(url) => url,
            None => {
                let legacy_asset = format!("opencrabs-{}.{}", suffix, ext);
                match assets.and_then(|arr| {
                    arr.iter().find_map(|a| {
                        let name = a["name"].as_str()?;
                        if name == legacy_asset {
                            a["browser_download_url"].as_str().map(String::from)
                        } else {
                            None
                        }
                    })
                }) {
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
                }
            }
        };

        // Emit download progress
        if let Some(ref cb) = self.progress {
            cb(
                sid,
                ProgressEvent::IntermediateText {
                    text: format!("Downloading v{}...", latest_version),
                    reasoning: None,
                },
            );
        }

        // Download the archive
        let archive_bytes = match client.get(&download_url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
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

        // Extract binary from archive
        let binary_data = if is_windows {
            extract_from_zip(&archive_bytes, "opencrabs.exe")?
        } else {
            extract_from_tar_gz(&archive_bytes, "opencrabs")?
        };

        // Replace current executable
        let exe_path = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Cannot locate current binary: {}",
                    e
                )));
            }
        };

        // Write to a temp file next to the executable, then atomically rename
        let tmp_path = exe_path.with_extension("evolve_tmp");
        if let Err(e) = tokio::fs::write(&tmp_path, &binary_data).await {
            return Ok(ToolResult::error(format!(
                "Failed to write new binary: {}",
                e
            )));
        }

        // Set executable permission on Unix
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

        // Atomic rename (on Unix this replaces the running binary on disk —
        // the old binary stays in memory until exec() replaces the process)
        if let Err(e) = std::fs::rename(&tmp_path, &exe_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Ok(ToolResult::error(format!(
                "Failed to replace binary: {}",
                e
            )));
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
