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

/// Resolve the running executable's real on-disk path.
///
/// After an in-place binary replacement (`/evolve` unlinks the old inode,
/// then renames the new binary into place), Linux reports `/proc/self/exe`
/// as `"<path> (deleted)"`, and `std::env::current_exe()` returns that
/// literal string verbatim. Using it as a path then `exec()`s a
/// non-existent `"… (deleted)"` target (ENOENT), and — worse — a retry
/// writes the next downloaded binary to a real file literally named
/// `"opencrabs (deleted)"`. Both were observed in the wild. Strip the
/// marker so every caller works off the genuine path.
pub fn running_binary_path() -> std::io::Result<PathBuf> {
    Ok(strip_deleted_marker(std::env::current_exe()?))
}

/// Pure helper for `running_binary_path`, factored out so the
/// `" (deleted)"`-stripping can be tested without `current_exe()`.
///
/// Strips EVERY trailing `" (deleted)"` marker, not just one: a binary that
/// was evolved while running from an already-deleted inode stacks the suffix
/// (`opencrabs (deleted) (deleted)` was observed on a VPS after repeated
/// pre-fix evolves), so a single strip would still resolve to a junk file.
pub(crate) fn strip_deleted_marker(exe: PathBuf) -> PathBuf {
    match exe.to_str() {
        Some(s) => {
            let mut trimmed = s;
            while let Some(stripped) = trimmed.strip_suffix(" (deleted)") {
                trimmed = stripped;
            }
            if trimmed.len() == s.len() {
                exe
            } else {
                PathBuf::from(trimmed)
            }
        }
        None => exe,
    }
}

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
    /// **Source tree found** (Cargo.toml walks up from exe): uses
    /// `<project_root>/target/release/opencrabs` as the binary path.
    /// This is the `/rebuild` path.
    ///
    /// **Pre-built binary** (no Cargo.toml): uses the current executable
    /// path (`std::env::current_exe()`) as the binary path. This is the
    /// `/evolve` path: the download already replaced the binary at this
    /// location, so restart just exec's it. Source is lazily cloned into
    /// `~/.opencrabs/source/` only when `/rebuild` is invoked.
    ///
    /// Before this fix, auto_detect() cloned unconditionally then pointed
    /// binary_path at a target/ dir that was never built, causing
    /// "exec() failed: No such file or directory" on restart (#179).
    pub fn auto_detect() -> Result<Self> {
        let exe = running_binary_path()?;
        let source_dir = crate::config::opencrabs_home().join("source");
        let (project_root, binary_path) = Self::resolve_paths(&exe, source_dir)?;
        tracing::info!(
            "SelfUpdater: project_root={}, binary={}",
            project_root.display(),
            binary_path.display()
        );
        Ok(Self {
            project_root,
            binary_path,
        })
    }

    /// Pure path-resolution for `auto_detect`, factored out so it can be
    /// tested without depending on `current_exe()`.
    ///
    /// Walking up from `exe`, the FIRST directory containing `Cargo.toml` is
    /// a source tree → `(root, root/target/release/opencrabs)` (the
    /// `/rebuild` build output). If no `Cargo.toml` is found, this is a
    /// pre-built install → `(source_dir, exe)`: the binary path is the
    /// running exe itself (which `/evolve` replaces in place), and
    /// `source_dir` is only used as the lazy-clone target for `/rebuild`.
    ///
    /// The pre-built branch returning `exe` (not a never-built
    /// `source_dir/target/release/opencrabs`) is the #179 fix: restart after
    /// auto-update must exec the real binary, not a path that was never built.
    pub(crate) fn resolve_paths(
        exe: &std::path::Path,
        source_dir: std::path::PathBuf,
    ) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
        let mut search_dir = exe
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine executable parent directory"))?
            .to_path_buf();

        loop {
            if search_dir.join("Cargo.toml").exists() {
                let binary_path = search_dir.join("target").join("release").join("opencrabs");
                return Ok((search_dir, binary_path));
            }
            if !search_dir.pop() {
                break;
            }
        }

        // Pre-built binary: no source tree. binary_path = the running exe.
        Ok((source_dir, exe.to_path_buf()))
    }

    /// Ensure the source tree exists at project_root (lazy clone).
    /// Called by build() when the user invokes /rebuild on a pre-built binary.
    pub(crate) fn ensure_source_tree(&self) -> Result<()> {
        if self.project_root.join("Cargo.toml").exists() {
            // Already have source. Pull latest.
            tracing::info!("Updating source at {}", self.project_root.display());
            let _ = std::process::Command::new("git")
                .args(["pull", "--ff-only"])
                .current_dir(&self.project_root)
                .output();
            return Ok(());
        }

        // Clone the repo
        tracing::info!(
            "Cloning OpenCrabs source to {}",
            self.project_root.display()
        );
        let output = std::process::Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                REPO_URL,
                &self.project_root.to_string_lossy(),
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to clone source (is git installed?): {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("git clone failed: {}", stderr));
        }

        Ok(())
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

        // Lazy clone source tree if needed (pre-built binary + /rebuild).
        if let Err(e) = self.ensure_source_tree() {
            return Err(format!("Failed to prepare source tree: {e}"));
        }

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
            // For pre-built binaries, binary_path points at the exe path
            // (which /evolve replaced). After /rebuild, the binary is at
            // <project_root>/target/release/opencrabs. Use whichever exists.
            let built_path = self
                .project_root
                .join("target")
                .join("release")
                .join("opencrabs");
            let result_path = if built_path.exists() {
                built_path
            } else {
                self.binary_path.clone()
            };
            tracing::info!("Build succeeded: {}", result_path.display());
            Ok(result_path)
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
        Self::restart_into(&self.binary_path, session_id)
    }

    /// On non-Unix platforms, restart is not supported via exec().
    #[cfg(not(unix))]
    pub fn restart(&self, session_id: Uuid) -> Result<()> {
        Self::restart_into(&self.binary_path, session_id)
    }

    /// Exec-restart into a SPECIFIC binary path. Used by the RestartReady
    /// handler to launch the exact binary that was just produced (e.g.
    /// `/rebuild` returns a freshly-built binary that is NOT the running
    /// exe on a pre-built install). Resolving the path via `auto_detect()`
    /// instead would pick the stale running exe and restart into the old
    /// version (#179 follow-up). Passes `chat --session <id>` to resume the
    /// same session. Only returns on error — on success the process is
    /// replaced.
    #[cfg(unix)]
    pub fn restart_into(binary_path: &std::path::Path, session_id: Uuid) -> Result<()> {
        use std::os::unix::process::CommandExt;

        tracing::info!(
            "Restarting OpenCrabs: {} chat --session {}",
            binary_path.display(),
            session_id
        );

        let err = std::process::Command::new(binary_path)
            .args(["chat", "--session", &session_id.to_string()])
            .env("OPENCRABS_EVOLVED_FROM", crate::VERSION)
            .exec(); // Replaces the process — only returns on error

        Err(anyhow::anyhow!("exec() failed: {}", err))
    }

    #[cfg(not(unix))]
    pub fn restart_into(_binary_path: &std::path::Path, _session_id: Uuid) -> Result<()> {
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
