//! Profile management — multi-instance isolated OpenCrabs environments.
//!
//! Each profile gets its own `config.toml`, `keys.toml`, `opencrabs.db`,
//! `memory/`, brain files, and `layout.json`. The "default" profile maps
//! to `~/.opencrabs/` for backward compatibility; named profiles live
//! under `~/.opencrabs/profiles/<name>/`.
//!
//! Selection priority (first wins):
//! 1. `set_active_profile()` (called from CLI `-p` flag)
//! 2. `OPENCRABS_PROFILE` environment variable
//! 3. Falls back to "default"

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Global active profile name. Set once at startup before anything calls `opencrabs_home()`.
static ACTIVE_PROFILE: OnceLock<Option<String>> = OnceLock::new();

/// Set the active profile. Must be called before any `opencrabs_home()` call.
/// Returns `Err` if called more than once (OnceLock semantics).
pub fn set_active_profile(name: Option<String>) -> Result<()> {
    ACTIVE_PROFILE
        .set(name)
        .map_err(|_| anyhow::anyhow!("active profile already set"))
}

/// Get the active profile name, or `None` for default.
pub fn active_profile() -> Option<&'static str> {
    ACTIVE_PROFILE.get().and_then(|opt| opt.as_deref())
}

/// Resolve the home directory for the active profile.
///
/// - `None` / `"default"` → `~/.opencrabs/`
/// - `"hermes"` → `~/.opencrabs/profiles/hermes/`
pub fn resolve_profile_home() -> PathBuf {
    let base = base_opencrabs_dir();

    let profile_name = active_profile().map(String::from).or_else(|| {
        std::env::var("OPENCRABS_PROFILE")
            .ok()
            .filter(|s| !s.is_empty())
    });

    match profile_name.as_deref() {
        None | Some("default") => base,
        Some(name) => base.join("profiles").join(name),
    }
}

/// The raw `~/.opencrabs/` directory (profile-agnostic).
pub fn base_opencrabs_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".opencrabs")
}

// ─── Profile Registry ────────────────────────────────────────────────

/// Metadata for a single profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEntry {
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub last_used: Option<String>,
}

/// Registry of all profiles, stored at `~/.opencrabs/profiles.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileRegistry {
    #[serde(default)]
    pub profiles: HashMap<String, ProfileEntry>,
}

impl ProfileRegistry {
    fn path() -> PathBuf {
        base_opencrabs_dir().join("profiles.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        fs::write(&path, contents).with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn register(&mut self, name: &str, description: Option<&str>) {
        self.profiles.insert(
            name.to_string(),
            ProfileEntry {
                name: name.to_string(),
                description: description.map(String::from),
                created_at: Utc::now().to_rfc3339(),
                last_used: None,
            },
        );
    }

    pub fn touch(&mut self, name: &str) {
        if let Some(entry) = self.profiles.get_mut(name) {
            entry.last_used = Some(Utc::now().to_rfc3339());
        }
    }
}

// ─── Profile CRUD ────────────────────────────────────────────────────

/// Create a new named profile with its directory structure.
pub fn create_profile(name: &str, description: Option<&str>) -> Result<PathBuf> {
    validate_profile_name(name)?;

    let profile_dir = base_opencrabs_dir().join("profiles").join(name);
    if profile_dir.exists() {
        bail!(
            "profile '{}' already exists at {}",
            name,
            profile_dir.display()
        );
    }

    // Create directory structure
    fs::create_dir_all(&profile_dir)?;
    fs::create_dir_all(profile_dir.join("memory"))?;
    fs::create_dir_all(profile_dir.join("logs"))?;

    // Register
    let mut registry = ProfileRegistry::load()?;
    registry.register(name, description);
    registry.save()?;

    tracing::info!("Created profile '{}' at {}", name, profile_dir.display());
    Ok(profile_dir)
}

/// List all profiles (always includes "default").
pub fn list_profiles() -> Result<Vec<ProfileEntry>> {
    let registry = ProfileRegistry::load()?;

    let mut profiles = vec![ProfileEntry {
        name: "default".to_string(),
        description: Some("Default profile (~/.opencrabs/)".to_string()),
        created_at: String::new(),
        last_used: None,
    }];

    let mut named: Vec<_> = registry.profiles.values().cloned().collect();
    named.sort_by(|a, b| a.name.cmp(&b.name));
    profiles.extend(named);

    Ok(profiles)
}

/// Delete a named profile and its directory.
pub fn delete_profile(name: &str) -> Result<()> {
    if name == "default" {
        bail!("cannot delete the default profile");
    }

    let profile_dir = base_opencrabs_dir().join("profiles").join(name);
    if !profile_dir.exists() {
        bail!("profile '{}' does not exist", name);
    }

    fs::remove_dir_all(&profile_dir).with_context(|| {
        format!(
            "failed to delete profile directory: {}",
            profile_dir.display()
        )
    })?;

    let mut registry = ProfileRegistry::load()?;
    registry.profiles.remove(name);
    registry.save()?;

    tracing::info!("Deleted profile '{}'", name);
    Ok(())
}

/// Export a profile as a tar.gz archive.
pub fn export_profile(name: &str, output: &Path) -> Result<()> {
    let profile_dir = if name == "default" {
        base_opencrabs_dir()
    } else {
        let dir = base_opencrabs_dir().join("profiles").join(name);
        if !dir.exists() {
            bail!("profile '{}' does not exist", name);
        }
        dir
    };

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::Builder;

    let file = fs::File::create(output)
        .with_context(|| format!("failed to create {}", output.display()))?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(enc);

    // Add profile directory contents
    tar.append_dir_all(name, &profile_dir)
        .with_context(|| "failed to add profile to archive")?;

    tar.finish()?;
    tracing::info!("Exported profile '{}' to {}", name, output.display());
    Ok(())
}

/// Import a profile from a tar.gz archive.
pub fn import_profile(archive: &Path) -> Result<String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    if !archive.exists() {
        bail!("archive not found: {}", archive.display());
    }

    let file = fs::File::open(archive)?;
    let dec = GzDecoder::new(file);
    let mut ar = Archive::new(dec);

    // Peek at the first entry to get the profile name
    let profile_name = {
        let file = fs::File::open(archive)?;
        let dec = GzDecoder::new(file);
        let mut ar = Archive::new(dec);
        let first = ar.entries()?.next();
        match first {
            Some(Ok(entry)) => {
                let path = entry.path()?;
                path.components()
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .unwrap_or_default()
            }
            _ => bail!("archive is empty"),
        }
    };

    if profile_name.is_empty() {
        bail!("could not determine profile name from archive");
    }

    let target = base_opencrabs_dir().join("profiles");
    fs::create_dir_all(&target)?;

    ar.unpack(&target)
        .with_context(|| "failed to extract archive")?;

    // Register the imported profile
    let mut registry = ProfileRegistry::load()?;
    if !registry.profiles.contains_key(&profile_name) {
        registry.register(&profile_name, Some("Imported profile"));
        registry.save()?;
    }

    tracing::info!(
        "Imported profile '{}' from {}",
        profile_name,
        archive.display()
    );
    Ok(profile_name)
}

// ─── Profile Migration ───────────────────────────────────────────────

/// Migrate config and brain files from one profile to another.
/// Copies `*.md`, `*.toml`, and `memory/` directory.
/// Does NOT copy database, sessions, logs, locks, or layout.
pub fn migrate_profile(from: &str, to: &str, force: bool) -> Result<Vec<String>> {
    let base = base_opencrabs_dir();

    let src_dir = if from == "default" {
        base.clone()
    } else {
        let dir = base.join("profiles").join(from);
        if !dir.exists() {
            bail!("source profile '{}' does not exist", from);
        }
        dir
    };

    let dst_dir = if to == "default" {
        base.clone()
    } else {
        let dir = base.join("profiles").join(to);
        if !dir.exists() {
            bail!(
                "destination profile '{}' does not exist. Create it first with: opencrabs profile create {}",
                to,
                to
            );
        }
        dir
    };

    if src_dir == dst_dir {
        bail!("source and destination profiles are the same");
    }

    let mut migrated = Vec::new();

    // Copy top-level *.md and *.toml files (config, keys, brain files)
    // Skip: profiles.toml (registry), layout.json, locks, DB
    let skip_files = ["profiles.toml", "layout.json"];

    for entry in fs::read_dir(&src_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if path.is_file() {
            let dominated = name_str.ends_with(".md") || name_str.ends_with(".toml");
            if !dominated || skip_files.contains(&name_str.as_ref()) {
                continue;
            }

            let dst_path = dst_dir.join(&name);
            if dst_path.exists() && !force {
                tracing::warn!(
                    "Skipping '{}' — already exists in '{}' (use --force to overwrite)",
                    name_str,
                    to
                );
                continue;
            }

            fs::copy(&path, &dst_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    path.display(),
                    dst_path.display()
                )
            })?;
            migrated.push(name_str.to_string());
        }
    }

    // Copy memory/ directory
    let src_memory = src_dir.join("memory");
    if src_memory.exists() && src_memory.is_dir() {
        let dst_memory = dst_dir.join("memory");
        fs::create_dir_all(&dst_memory)?;

        for entry in fs::read_dir(&src_memory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let dst_path = dst_memory.join(&name);

                if dst_path.exists() && !force {
                    tracing::warn!(
                        "Skipping memory/'{}' — already exists (use --force to overwrite)",
                        name_str
                    );
                    continue;
                }

                fs::copy(&path, &dst_path)?;
                migrated.push(format!("memory/{}", name_str));
            }
        }
    }

    tracing::info!(
        "Migrated {} files from profile '{}' to '{}'",
        migrated.len(),
        from,
        to
    );
    Ok(migrated)
}

// ─── Token Lock ──────────────────────────────────────────────────────

/// Check and acquire a token lock for a channel credential.
/// Returns `Err` if another profile holds the lock.
pub fn acquire_token_lock(channel: &str, token_hash: &str) -> Result<()> {
    let lock_dir = base_opencrabs_dir().join("locks");
    fs::create_dir_all(&lock_dir)?;

    let lock_file = lock_dir.join(format!("{}_{}.lock", channel, token_hash));
    let current_profile = active_profile().unwrap_or("default");
    let pid = std::process::id();

    if lock_file.exists() {
        let contents = fs::read_to_string(&lock_file).unwrap_or_default();
        let parts: Vec<&str> = contents.splitn(2, ':').collect();
        if parts.len() == 2 {
            let locked_profile = parts[0];
            let locked_pid: u32 = parts[1].parse().unwrap_or(0);

            // Same profile — check if PID is still alive
            if locked_profile == current_profile {
                if is_pid_alive(locked_pid) && locked_pid != pid {
                    bail!(
                        "profile '{}' already running (PID {}). Only one instance per profile allowed.",
                        current_profile,
                        locked_pid
                    );
                }
                // Stale lock from same profile — overwrite
            } else {
                // Different profile — check PID
                if is_pid_alive(locked_pid) {
                    bail!(
                        "channel '{}' token is locked by profile '{}' (PID {}). \
                         Two profiles cannot share the same bot credential.",
                        channel,
                        locked_profile,
                        locked_pid
                    );
                }
                // Stale lock from dead process — overwrite
            }
        }
    }

    fs::write(&lock_file, format!("{}:{}", current_profile, pid))?;
    Ok(())
}

/// Release a token lock.
pub fn release_token_lock(channel: &str, token_hash: &str) {
    let lock_file = base_opencrabs_dir()
        .join("locks")
        .join(format!("{}_{}.lock", channel, token_hash));
    let _ = fs::remove_file(lock_file);
}

/// Release all locks held by this process.
pub fn release_all_locks() {
    let lock_dir = base_opencrabs_dir().join("locks");
    let pid = std::process::id();
    let current_profile = active_profile().unwrap_or("default");
    let expected = format!("{}:{}", current_profile, pid);

    if let Ok(entries) = fs::read_dir(&lock_dir) {
        for entry in entries.flatten() {
            if let Ok(contents) = fs::read_to_string(entry.path())
                && contents.trim() == expected
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Hash a token for lock file naming (no raw secrets on disk).
pub fn hash_token(token: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ─── Helpers ─────────────────────────────────────────────────────────

pub fn validate_profile_name(name: &str) -> Result<()> {
    if name == "default" {
        bail!("'default' is reserved — the default profile is ~/.opencrabs/");
    }
    if name.is_empty() || name.len() > 64 {
        bail!("profile name must be 1-64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        bail!("profile name can only contain alphanumeric, hyphens, and underscores");
    }
    Ok(())
}

fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) returns 0 if we can signal the process.
        // If it returns -1, check errno: ESRCH means the process doesn't exist,
        // EPERM means it exists but we lack permission (still alive).
        let ret = unsafe { libc::kill(pid as i32, 0) };
        if ret == 0 {
            return true;
        }
        // EPERM = process exists but owned by another user (e.g. PID 1 = launchd)
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        extern "system" {
            fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize;
            fn CloseHandle(hObject: isize) -> i32;
        }
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle == 0 {
            false
        } else {
            unsafe { CloseHandle(handle) };
            true
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}
