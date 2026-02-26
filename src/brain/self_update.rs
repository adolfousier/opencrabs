//! Self-Update Module
//!
//! Handles building, testing, and hot-restarting OpenCrabs.
//! The running binary is in memory — modifying source on disk is safe.
//! After a successful build, `exec()` replaces the current process with the new binary.
//!
//! If the binary was downloaded (no source tree), `auto_detect()` automatically
//! clones the repo into `~/.opencrabs/source/` so `/rebuild` works everywhere.

use anyhow::Result;
use std::path::PathBuf;
use uuid::Uuid;

/// GitHub repo URL for auto-cloning when source is not available locally.
const REPO_URL: &str = "https://github.com/adolfousier/opencrabs.git";

/// Handles building, testing, and restarting OpenCrabs from source.
pub struct SelfUpdater {
    /// Root of the OpenCrabs project (where Cargo.toml lives)
    project_root: PathBuf,
    /// Path to the compiled binary
    binary_path: PathBuf,
}

impl SelfUpdater {
    /// Create a new SelfUpdater.
    ///
    /// `project_root` — directory containing Cargo.toml
    /// `binary_path` — where the release binary will be after build
    pub fn new(project_root: PathBuf, binary_path: PathBuf) -> Self {
        Self {
            project_root,
            binary_path,
        }
    }

    /// Auto-detect project root and binary path from the current executable.
    ///
    /// First walks up from the binary looking for `Cargo.toml` (build-from-source).
    /// If not found (pre-built binary), checks `~/.opencrabs/source/` for a
    /// previous clone. If that doesn't exist either, clones the repo there.
    pub fn auto_detect() -> Result<Self> {
        let exe = std::env::current_exe()?;

        // Walk up from the executable to find Cargo.toml
        let mut search_dir = exe
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine executable parent directory"))?
            .to_path_buf();

        loop {
            if search_dir.join("Cargo.toml").exists() {
                let binary_path = search_dir.join("target").join("release").join("opencrabs");
                return Ok(Self {
                    project_root: search_dir,
                    binary_path,
                });
            }
            if !search_dir.pop() {
                break;
            }
        }

        // No source tree found — use ~/.opencrabs/source/
        let source_dir = crate::config::opencrabs_home().join("source");

        if source_dir.join("Cargo.toml").exists() {
            // Source already cloned — pull latest
            tracing::info!("Updating source at {}", source_dir.display());
            let _ = std::process::Command::new("git")
                .args(["pull", "--ff-only"])
                .current_dir(&source_dir)
                .output();
        } else {
            // Clone the repo
            tracing::info!("Cloning OpenCrabs source to {}", source_dir.display());
            let output = std::process::Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    REPO_URL,
                    &source_dir.to_string_lossy(),
                ])
                .output()
                .map_err(|e| {
                    anyhow::anyhow!("Failed to clone source (is git installed?): {}", e)
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow::anyhow!("git clone failed: {}", stderr));
            }
        }

        let binary_path = source_dir.join("target").join("release").join("opencrabs");

        Ok(Self {
            project_root: source_dir,
            binary_path,
        })
    }

    /// Build the project with `cargo build --release`.
    ///
    /// Returns `Ok(binary_path)` on success or `Err(compiler_output)` on failure.
    pub async fn build(&self) -> Result<PathBuf, String> {
        self.build_streaming(|_| {}).await
    }

    /// Build with streaming progress — calls `on_line` for each compiler output line.
    ///
    /// Returns `Ok(binary_path)` on success or `Err(compiler_output)` on failure.
    pub async fn build_streaming<F>(&self, on_line: F) -> Result<PathBuf, String>
    where
        F: Fn(String) + Send + 'static,
    {
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio::process::Command;

        tracing::info!("Building OpenCrabs at {}", self.project_root.display());

        let mut child = Command::new("cargo")
            .args(["build", "--release"])
            .env("RUSTFLAGS", "-C target-cpu=native")
            .current_dir(&self.project_root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn cargo build: {}", e))?;

        // Stream stderr (where cargo writes progress) line by line
        if let Some(stderr) = child.stderr.take() {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                on_line(line);
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| format!("Build process error: {}", e))?;

        if status.success() {
            tracing::info!("Build succeeded: {}", self.binary_path.display());
            Ok(self.binary_path.clone())
        } else {
            Err("Build failed — see output above".to_string())
        }
    }

    /// Run tests with `cargo test`.
    ///
    /// Returns `Ok(())` on success or `Err(test_output)` on failure.
    pub async fn test(&self) -> Result<(), String> {
        tracing::info!("Running tests at {}", self.project_root.display());

        let output = tokio::process::Command::new("cargo")
            .arg("test")
            .current_dir(&self.project_root)
            .output()
            .await
            .map_err(|e| format!("Failed to spawn cargo test: {}", e))?;

        if output.status.success() {
            tracing::info!("Tests passed");
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            tracing::warn!("Tests failed:\n{}\n{}", stderr, stdout);
            Err(format!("{}\n{}", stderr, stdout))
        }
    }

    /// Replace the running process with the new binary via Unix exec().
    ///
    /// Passes `chat --session <session_id>` to resume the same session.
    /// This function only returns on error — on success, the process is replaced.
    #[cfg(unix)]
    pub fn restart(&self, session_id: Uuid) -> Result<()> {
        use std::os::unix::process::CommandExt;

        tracing::info!(
            "Restarting OpenCrabs: {} chat --session {}",
            self.binary_path.display(),
            session_id
        );

        let err = std::process::Command::new(&self.binary_path)
            .args(["chat", "--session", &session_id.to_string()])
            .exec(); // Replaces the process — only returns on error

        Err(anyhow::anyhow!("exec() failed: {}", err))
    }

    /// On non-Unix platforms, restart is not supported via exec().
    #[cfg(not(unix))]
    pub fn restart(&self, _session_id: Uuid) -> Result<()> {
        Err(anyhow::anyhow!(
            "Hot restart via exec() is only supported on Unix platforms"
        ))
    }

    /// Get the project root path.
    pub fn project_root(&self) -> &std::path::Path {
        &self.project_root
    }

    /// Get the binary path.
    pub fn binary_path(&self) -> &std::path::Path {
        &self.binary_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let updater = SelfUpdater::new(
            PathBuf::from("/tmp/project"),
            PathBuf::from("/tmp/project/target/release/opencrabs"),
        );
        assert_eq!(updater.project_root(), std::path::Path::new("/tmp/project"));
        assert_eq!(
            updater.binary_path(),
            std::path::Path::new("/tmp/project/target/release/opencrabs")
        );
    }
}
