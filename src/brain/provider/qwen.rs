//! Qwen native provider — OAuth device flow + DashScope OpenAI-compatible API.
//!
//! This is the **native** Qwen integration: OpenCrabs talks directly to
//! `portal.qwen.ai/v1/chat/completions` and orchestrates tools, context, and
//! caching itself. It does NOT shell out to the `qwen` CLI binary (see
//! `qwen_code.rs` for the subprocess-based path).
//!
//! ## Authentication
//!
//! Mirrors `qwen-code-cli` byte-for-byte so users who already authenticated
//! with the CLI can use OpenCrabs immediately — credentials are read from
//! and written to the same `~/.qwen/oauth_creds.json` file.
//!
//! Flow (RFC 8628 Device Authorization Grant + PKCE S256):
//! 1. POST `https://chat.qwen.ai/api/v1/oauth2/device/code` with the public
//!    client_id, requested scope, and a SHA-256 PKCE challenge.
//! 2. Show the user `verification_uri_complete` (or open it in their browser)
//!    plus the short `user_code`.
//! 3. Poll `https://chat.qwen.ai/api/v1/oauth2/token` every ~2s until the user
//!    authorizes (or the device code expires).
//! 4. Persist `{access_token, refresh_token, resource_url, expiry_date}` to
//!    `~/.qwen/oauth_creds.json` (mode 0600).
//! 5. On every API call, refresh proactively if `expiry_date - 30s` is past.
//! 6. On 401/403, force-refresh once and retry.
//!
//! Free tier: 60 req/minute, 1000 req/day, model `coder-model` only.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Constants ──────────────────────────────────────────────────────────────

/// OAuth public client_id baked into qwen-code-cli. Public client (no secret).
pub const QWEN_OAUTH_CLIENT_ID: &str = "f0304373b74a44d2b584a3fb70ca9e56";

/// OAuth scopes — `openid profile email model.completion`.
pub const QWEN_OAUTH_SCOPE: &str = "openid profile email model.completion";

/// Device-code endpoint.
pub const QWEN_DEVICE_CODE_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/device/code";

/// Token endpoint (used for both initial poll and refresh).
pub const QWEN_TOKEN_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/token";

/// Device-code grant URN.
pub const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Default chat host when the OAuth response doesn't include a `resource_url`.
pub const QWEN_DEFAULT_API_HOST: &str = "dashscope.aliyuncs.com/compatible-mode/v1";

/// Default chat completions URL when the OAuth response gives `portal.qwen.ai`.
pub const QWEN_DEFAULT_CHAT_URL: &str = "https://portal.qwen.ai/v1/chat/completions";

/// Free-tier model id (the only model available on the OAuth path).
/// API id is `coder-model`; display label is "Qwen 3.6 Plus".
pub const QWEN_OAUTH_MODEL: &str = "coder-model";

/// Refresh tokens this many ms before they expire (matches qwen-cli's 30s).
const REFRESH_BUFFER_MS: u64 = 30_000;

// ── Persisted credentials ─────────────────────────────────────────────────

/// On-disk shape of `~/.qwen/oauth_creds.json` — matches qwen-code-cli exactly
/// so the two tools share authentication state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QwenCredentials {
    pub access_token: String,
    pub token_type: String,
    pub refresh_token: String,
    /// Hostname (no scheme, no path) of the API to call. Typically
    /// `portal.qwen.ai`. May be omitted on legacy creds.
    #[serde(default)]
    pub resource_url: String,
    /// Absolute expiry as ms since UNIX epoch.
    pub expiry_date: u64,
}

impl QwenCredentials {
    /// Path to the shared `qwen-code-cli` credentials file. Used **only** as
    /// an import source during onboarding so users who already authenticated
    /// with the CLI get instant access. The canonical store is `keys.toml`
    /// under `[providers.qwen]` — see `persist_to_keys`.
    pub fn qwen_cli_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".qwen")
            .join("oauth_creds.json")
    }

    /// One-shot import from `~/.qwen/oauth_creds.json`. Returns `None` if
    /// missing or invalid. Onboarding may call this to skip the device flow
    /// when the user already authenticated via qwen-cli.
    pub fn import_from_qwen_cli() -> Option<Self> {
        let path = Self::qwen_cli_path();
        let bytes = std::fs::read(&path).ok()?;
        let creds: Self = serde_json::from_slice(&bytes).ok()?;
        if creds.access_token.is_empty() || creds.refresh_token.is_empty() {
            return None;
        }
        Some(creds)
    }

    /// Last-modified time of `~/.qwen/oauth_creds.json` in whole seconds
    /// since UNIX epoch, or 0 if the file is missing / unreadable. Used by
    /// `QwenTokenManager` to detect when qwen-cli has rotated the token on
    /// disk so we can live-reload without restarting.
    pub fn qwen_cli_mtime() -> u64 {
        let path = Self::qwen_cli_path();
        std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Write credentials back to `~/.qwen/oauth_creds.json` so qwen-cli
    /// running alongside us sees the refreshed token and doesn't burn
    /// its own refresh. Best-effort: logs a warning on failure instead
    /// of propagating it.
    pub fn write_back_to_qwen_cli(&self) {
        let path = Self::qwen_cli_path();
        let Ok(json) = serde_json::to_vec_pretty(self) else {
            tracing::warn!("Failed to serialize Qwen creds for qwen-cli write-back");
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::warn!("Failed to create {} parent dir: {}", parent.display(), e);
            return;
        }
        if let Err(e) = std::fs::write(&path, &json) {
            tracing::warn!("Failed to write back to {}: {}", path.display(), e);
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }

    /// Build credentials from a `ProviderConfig` (loaded from keys.toml).
    /// Returns `None` if the config doesn't have the required OAuth fields.
    pub fn from_provider_config(cfg: &crate::config::ProviderConfig) -> Option<Self> {
        let access_token = cfg.api_key.as_ref().filter(|s| !s.is_empty())?.clone();
        let refresh_token = cfg
            .refresh_token
            .as_ref()
            .filter(|s| !s.is_empty())?
            .clone();
        Some(Self {
            access_token,
            token_type: "Bearer".to_string(),
            refresh_token,
            resource_url: cfg.resource_url.clone().unwrap_or_default(),
            expiry_date: cfg.expiry_date.unwrap_or(0),
        })
    }

    /// Wipe single-account credentials from `keys.toml` (`[providers.qwen]`)
    /// and `~/.qwen/oauth_creds.json`. Called when token is expired so stale
    /// creds don't linger. Rotation accounts are handled separately via
    /// `persist_all_accounts` with only the valid subset.
    ///
    /// Idempotent: skips write if there are no creds to remove. Uses a static
    /// guard to prevent multiple threads racing to wipe simultaneously.
    pub fn wipe_dead_credentials() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WIPE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

        // Prevent concurrent wipe races (multiple background threads)
        if WIPE_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        struct WipeGuard;
        impl Drop for WipeGuard {
            fn drop(&mut self) {
                WIPE_IN_PROGRESS.store(false, Ordering::SeqCst);
            }
        }
        let _guard = WipeGuard;

        if let Ok(path) = std::panic::catch_unwind(crate::config::keys_path)
            && path.exists()
            && let Ok(raw) = std::fs::read_to_string(&path)
            && let Ok(mut doc) = raw.parse::<toml::Value>()
        {
            let mut changed = false;
            if let Some(qwen) = doc
                .get_mut("providers")
                .and_then(|p| p.get_mut("qwen"))
                .and_then(|q| q.as_table_mut())
            {
                let had_key = qwen.remove("api_key").is_some();
                let had_refresh = qwen.remove("refresh_token").is_some();
                let had_expiry = qwen.remove("expiry_date").is_some();
                let had_url = qwen.remove("resource_url").is_some();
                changed = had_key || had_refresh || had_expiry || had_url;
            }
            if changed {
                if let Ok(out) = toml::to_string_pretty(&doc) {
                    let _ = std::fs::write(&path, out);
                }
                tracing::info!("Wiped dead Qwen single-account creds from keys.toml");
            }
        }
        let cli_path = Self::qwen_cli_path();
        if cli_path.exists() {
            let _ = std::fs::remove_file(&cli_path);
            tracing::info!("Wiped dead Qwen creds from {}", cli_path.display());
        }
    }

    /// Persist credentials to `keys.toml` under `[providers.qwen]`.
    /// This is the canonical store. Background refresh writes here too.
    ///
    /// Manipulates the TOML document directly so we can write the
    /// `expiry_date` as a real integer (the existing `write_secret_key`
    /// helper is string-only).
    pub fn persist_to_keys(&self) -> anyhow::Result<()> {
        use crate::config::{daily_backup, keys_path};
        use anyhow::Context;

        let path = keys_path();

        let mut doc: toml::Value = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        let root = doc
            .as_table_mut()
            .context("keys.toml root is not a table")?;
        let providers = root
            .entry("providers".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .context("[providers] is not a table")?;
        let qwen = providers
            .entry("qwen".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .context("[providers.qwen] is not a table")?;

        // Only OAuth credentials live in keys.toml. `enabled` and
        // `default_model` are non-secret config concerns and belong in
        // config.toml (the onboarding flow writes them there). Persisting
        // them here pollutes keys.toml and causes config drift every time
        // the background refresher rewrites the access token.
        qwen.insert(
            "api_key".to_string(),
            toml::Value::String(self.access_token.clone()),
        );
        qwen.insert(
            "refresh_token".to_string(),
            toml::Value::String(self.refresh_token.clone()),
        );
        qwen.insert(
            "expiry_date".to_string(),
            toml::Value::Integer(self.expiry_date as i64),
        );
        if !self.resource_url.is_empty() {
            qwen.insert(
                "resource_url".to_string(),
                toml::Value::String(self.resource_url.clone()),
            );
        }
        // Scrub any legacy non-secret fields from prior versions.
        qwen.remove("enabled");
        qwen.remove("default_model");

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        daily_backup(&path, 7);
        let toml_str = toml::to_string_pretty(&doc)?;
        std::fs::write(&path, toml_str)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        tracing::debug!("Persisted Qwen credentials to keys.toml");
        Ok(())
    }

    /// Build a vec of credentials from a `[[providers.qwen_accounts]]` array.
    pub fn from_account_configs(cfgs: &[crate::config::ProviderConfig]) -> Vec<Self> {
        cfgs.iter().filter_map(Self::from_provider_config).collect()
    }

    /// Persist refreshed credentials to a specific slot in `[[providers.qwen_accounts]]`
    /// in keys.toml. Used by rotation account background refresh so each account
    /// updates its own slot instead of overwriting `[providers.qwen]`.
    pub fn persist_to_account_slot(&self, index: usize) -> anyhow::Result<()> {
        use crate::config::{daily_backup, keys_path};
        use anyhow::Context;

        let path = keys_path();
        let mut doc: toml::Value = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
        } else {
            return Ok(()); // no keys.toml yet — nothing to update
        };

        let arr = doc
            .get_mut("providers")
            .and_then(|p| p.get_mut("qwen_accounts"))
            .and_then(|a| a.as_array_mut())
            .context("[[providers.qwen_accounts]] not found in keys.toml")?;

        // Ensure the array is large enough
        while arr.len() <= index {
            arr.push(toml::Value::Table(toml::map::Map::new()));
        }

        let slot = arr[index]
            .as_table_mut()
            .context("qwen_accounts slot is not a table")?;
        slot.insert(
            "api_key".to_string(),
            toml::Value::String(self.access_token.clone()),
        );
        slot.insert(
            "refresh_token".to_string(),
            toml::Value::String(self.refresh_token.clone()),
        );

        // Also update expiry_date in config.toml so the factory sees it's valid
        daily_backup(&path, 7);
        let toml_str = toml::to_string_pretty(&doc)?;
        std::fs::write(&path, toml_str)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        // Update expiry_date in config.toml (metadata lives there)
        Self::update_account_expiry_in_config(index, self.expiry_date)?;

        tracing::debug!(
            "Persisted refreshed Qwen rotation account {} to keys.toml",
            index
        );
        Ok(())
    }

    /// Update expiry_date for a single rotation account slot in config.toml.
    fn update_account_expiry_in_config(index: usize, expiry_date: u64) -> anyhow::Result<()> {
        let path = crate::config::opencrabs_home().join("config.toml");
        if !path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut doc: toml::Value =
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()));

        if let Some(arr) = doc
            .get_mut("providers")
            .and_then(|p| p.get_mut("qwen_accounts"))
            .and_then(|a| a.as_array_mut())
            && let Some(slot) = arr.get_mut(index).and_then(|s| s.as_table_mut())
        {
            slot.insert(
                "expiry_date".to_string(),
                toml::Value::Integer(expiry_date as i64),
            );
            let toml_str = toml::to_string_pretty(&doc)?;
            std::fs::write(&path, toml_str)?;
        }
        Ok(())
    }

    /// Clear secrets from keys.toml for expired accounts only.
    /// Config.toml is NOT touched — all accounts remain listed so the user
    /// can re-auth expired ones later via the UI.
    ///
    /// In keys.toml, expired account slots become `{}` (empty table) while
    /// valid accounts keep their `api_key` + `refresh_token`.
    pub fn clear_expired_account_secrets(all_creds: &[Self]) -> anyhow::Result<()> {
        use crate::config::{daily_backup, keys_path};
        use anyhow::Context;

        let path = keys_path();
        let mut doc: toml::Value = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
        } else {
            return Ok(());
        };

        let root = doc
            .as_table_mut()
            .context("keys.toml root is not a table")?;
        let providers = root
            .entry("providers".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .context("[providers] is not a table")?;

        // Build array preserving slot positions: valid accounts keep secrets,
        // expired accounts get empty entries (placeholder to maintain index alignment)
        let arr: Vec<toml::Value> = all_creds
            .iter()
            .map(|a| {
                let mut tbl = toml::map::Map::new();
                if a.is_valid() {
                    tbl.insert(
                        "api_key".to_string(),
                        toml::Value::String(a.access_token.clone()),
                    );
                    tbl.insert(
                        "refresh_token".to_string(),
                        toml::Value::String(a.refresh_token.clone()),
                    );
                }
                // Expired: empty table — secrets cleared, slot preserved
                toml::Value::Table(tbl)
            })
            .collect();

        providers.insert("qwen_accounts".to_string(), toml::Value::Array(arr));

        daily_backup(&path, 7);
        let toml_str = toml::to_string_pretty(&doc)?;
        std::fs::write(&path, toml_str)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        let valid = all_creds.iter().filter(|c| c.is_valid()).count();
        tracing::info!(
            "Cleared secrets for {} expired accounts, kept {} valid (keys.toml only)",
            all_creds.len() - valid,
            valid
        );
        Ok(())
    }

    /// Persist multiple rotation accounts with proper separation:
    /// - **keys.toml**: secrets only (`api_key`, `refresh_token`)
    /// - **config.toml**: metadata (`name`, `expiry_date`, `resource_url`)
    pub fn persist_all_accounts(accounts: &[Self]) -> anyhow::Result<()> {
        Self::persist_accounts_keys(accounts)?;
        Self::persist_accounts_config(accounts)?;
        tracing::debug!(
            "Persisted {} Qwen rotation accounts (keys.toml + config.toml)",
            accounts.len()
        );
        Ok(())
    }

    /// Write secrets only (`api_key`, `refresh_token`) to keys.toml.
    fn persist_accounts_keys(accounts: &[Self]) -> anyhow::Result<()> {
        use crate::config::{daily_backup, keys_path};
        use anyhow::Context;

        let path = keys_path();
        let mut doc: toml::Value = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        let root = doc
            .as_table_mut()
            .context("keys.toml root is not a table")?;
        let providers = root
            .entry("providers".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .context("[providers] is not a table")?;

        let arr: Vec<toml::Value> = accounts
            .iter()
            .map(|a| {
                let mut tbl = toml::map::Map::new();
                tbl.insert(
                    "api_key".to_string(),
                    toml::Value::String(a.access_token.clone()),
                );
                tbl.insert(
                    "refresh_token".to_string(),
                    toml::Value::String(a.refresh_token.clone()),
                );
                toml::Value::Table(tbl)
            })
            .collect();

        providers.insert("qwen_accounts".to_string(), toml::Value::Array(arr));

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        daily_backup(&path, 7);
        let toml_str = toml::to_string_pretty(&doc)?;
        std::fs::write(&path, toml_str)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Write metadata (`name`, `expiry_date`, `resource_url`) to config.toml.
    fn persist_accounts_config(accounts: &[Self]) -> anyhow::Result<()> {
        let path = crate::config::opencrabs_home().join("config.toml");
        let mut doc: toml::Value = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        let root = doc
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("config.toml root is not a table"))?;
        let providers = root
            .entry("providers".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("[providers] is not a table"))?;

        let arr: Vec<toml::Value> = accounts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let mut tbl = toml::map::Map::new();
                tbl.insert(
                    "name".to_string(),
                    toml::Value::String(format!("Account {}", i + 1)),
                );
                tbl.insert(
                    "expiry_date".to_string(),
                    toml::Value::Integer(a.expiry_date as i64),
                );
                if !a.resource_url.is_empty() {
                    tbl.insert(
                        "resource_url".to_string(),
                        toml::Value::String(a.resource_url.clone()),
                    );
                }
                toml::Value::Table(tbl)
            })
            .collect();

        providers.insert("qwen_accounts".to_string(), toml::Value::Array(arr));

        let toml_str = toml::to_string_pretty(&doc)?;
        std::fs::write(&path, toml_str)?;
        Ok(())
    }

    /// Migrate legacy keys.toml `[[providers.qwen_accounts]]` that had
    /// everything (secrets + metadata) into the split format:
    /// - Move `expiry_date`, `resource_url` to config.toml, add `name`
    /// - Keep only `api_key`, `refresh_token` in keys.toml
    ///
    /// Called once at startup. No-op if config.toml already has the entries.
    pub fn migrate_accounts_split() {
        let keys_path = crate::config::keys_path();
        let config_path = crate::config::opencrabs_home().join("config.toml");

        let keys_raw = match std::fs::read_to_string(&keys_path) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut keys_doc: toml::Value = match keys_raw.parse() {
            Ok(d) => d,
            Err(_) => return,
        };

        // Check if keys.toml has qwen_accounts with expiry_date (pre-migration)
        let needs_migration = keys_doc
            .get("providers")
            .and_then(|p| p.get("qwen_accounts"))
            .and_then(|a| a.as_array())
            .is_some_and(|arr| arr.iter().any(|entry| entry.get("expiry_date").is_some()));

        if !needs_migration {
            return;
        }

        // Check if config.toml already has qwen_accounts (already migrated)
        let config_raw = std::fs::read_to_string(&config_path).unwrap_or_default();
        let config_doc: toml::Value = config_raw
            .parse()
            .unwrap_or(toml::Value::Table(toml::map::Map::new()));
        let config_has_accounts = config_doc
            .get("providers")
            .and_then(|p| p.get("qwen_accounts"))
            .and_then(|a| a.as_array())
            .is_some_and(|arr| !arr.is_empty());

        if config_has_accounts {
            return;
        }

        tracing::info!(
            "Migrating Qwen rotation accounts: splitting keys.toml -> config.toml + keys.toml"
        );

        let accounts = keys_doc
            .get("providers")
            .and_then(|p| p.get("qwen_accounts"))
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();

        // Build config.toml entries (metadata only)
        let config_entries: Vec<toml::Value> = accounts
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let mut tbl = toml::map::Map::new();
                tbl.insert(
                    "name".to_string(),
                    toml::Value::String(format!("Account {}", i + 1)),
                );
                if let Some(exp) = entry.get("expiry_date") {
                    tbl.insert("expiry_date".to_string(), exp.clone());
                }
                if let Some(url) = entry.get("resource_url") {
                    tbl.insert("resource_url".to_string(), url.clone());
                }
                toml::Value::Table(tbl)
            })
            .collect();

        // Write config.toml entries
        let mut cfg_doc: toml::Value = config_raw
            .parse()
            .unwrap_or(toml::Value::Table(toml::map::Map::new()));
        if let Some(root) = cfg_doc.as_table_mut() {
            let providers = root
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(p) = providers.as_table_mut() {
                p.insert(
                    "qwen_accounts".to_string(),
                    toml::Value::Array(config_entries),
                );
            }
        }
        if let Ok(out) = toml::to_string_pretty(&cfg_doc) {
            let _ = std::fs::write(&config_path, out);
        }

        // Strip metadata from keys.toml (keep only api_key + refresh_token)
        let keys_entries: Vec<toml::Value> = accounts
            .iter()
            .filter_map(|entry| {
                let mut tbl = toml::map::Map::new();
                if let Some(key) = entry.get("api_key") {
                    tbl.insert("api_key".to_string(), key.clone());
                }
                if let Some(rt) = entry.get("refresh_token") {
                    tbl.insert("refresh_token".to_string(), rt.clone());
                }
                if tbl.is_empty() {
                    None
                } else {
                    Some(toml::Value::Table(tbl))
                }
            })
            .collect();

        if let Some(providers) = keys_doc
            .as_table_mut()
            .and_then(|r| r.get_mut("providers"))
            .and_then(|p| p.as_table_mut())
        {
            providers.insert(
                "qwen_accounts".to_string(),
                toml::Value::Array(keys_entries),
            );
        }
        if let Ok(out) = toml::to_string_pretty(&keys_doc) {
            let _ = std::fs::write(&keys_path, out);
        }

        tracing::info!("Qwen rotation account migration complete");
    }

    /// True if the access_token is still valid (with the 30s safety buffer).
    pub fn is_valid(&self) -> bool {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        now_ms + REFRESH_BUFFER_MS < self.expiry_date
    }

    /// Resolve the chat completions URL from `resource_url`.
    /// Logic mirrors qwen-cli's `getCurrentEndpoint`.
    pub fn chat_completions_url(&self) -> String {
        let host = if self.resource_url.is_empty() {
            QWEN_DEFAULT_API_HOST.to_string()
        } else {
            self.resource_url.clone()
        };
        let with_scheme = if host.starts_with("http://") || host.starts_with("https://") {
            host
        } else {
            format!("https://{}", host)
        };
        let with_v1 = if with_scheme.ends_with("/v1") {
            with_scheme
        } else {
            format!("{}/v1", with_scheme.trim_end_matches('/'))
        };
        format!("{}/chat/completions", with_v1)
    }
}

// ── PKCE ──────────────────────────────────────────────────────────────────

/// PKCE pair: random verifier + its SHA-256 base64url challenge.
#[derive(Debug, Clone)]
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

impl PkcePair {
    /// Generate a fresh PKCE pair (32-byte verifier, S256 challenge).
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let digest = hasher.finalize();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

        Self {
            verifier,
            challenge,
        }
    }
}

// ── Device flow request / response ────────────────────────────────────────

/// Response from the device-code endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Pre-filled URL containing the user_code — preferred for browser open.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    #[serde(default)]
    pub interval: Option<u64>,
}

/// Token poll response — either success (access_token populated) or pending
/// (error == "authorization_pending" / "slow_down").
#[derive(Debug, Deserialize)]
struct TokenPollResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    resource_url: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Kick off the OAuth device flow. Returns the device-code response so the
/// caller (onboarding wizard) can display the user code and verification URI
/// before calling `poll_for_token`.
pub async fn start_device_flow(pkce: &PkcePair) -> anyhow::Result<DeviceCodeResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(QWEN_DEVICE_CODE_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .header("x-request-id", uuid::Uuid::new_v4().to_string())
        .form(&[
            ("client_id", QWEN_OAUTH_CLIENT_ID),
            ("scope", QWEN_OAUTH_SCOPE),
            ("code_challenge", pkce.challenge.as_str()),
            ("code_challenge_method", "S256"),
        ])
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Qwen device flow request failed ({}): {}", status, body);
    }

    let raw = resp.text().await?;
    if raw.starts_with("<!doctype") || raw.starts_with("<!DOCTYPE") || raw.starts_with("<html") {
        anyhow::bail!(
            "Qwen device flow blocked by WAF (bot detection). \
             Try again in a few minutes or use a different network."
        );
    }
    let dcr: DeviceCodeResponse = serde_json::from_str(&raw)?;
    Ok(dcr)
}

/// Poll the token endpoint until the user authorizes (or the device code
/// expires). Returns the freshly-built `QwenCredentials` on success — the
/// caller is responsible for persisting them.
pub async fn poll_for_token(device_code: &str, pkce: &PkcePair) -> anyhow::Result<QwenCredentials> {
    let client = reqwest::Client::new();
    // qwen-cli starts at 2s and ignores the server interval. Honor that.
    let mut interval = Duration::from_secs(2);
    const MAX_ATTEMPTS: u32 = 600; // ~20 min ceiling

    for _ in 0..MAX_ATTEMPTS {
        tokio::time::sleep(interval).await;

        let resp = client
            .post(QWEN_TOKEN_URL)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .form(&[
                ("grant_type", DEVICE_CODE_GRANT),
                ("client_id", QWEN_OAUTH_CLIENT_ID),
                ("device_code", device_code),
                ("code_verifier", pkce.verifier.as_str()),
            ])
            .send()
            .await?;

        let status = resp.status();
        let raw = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Qwen token poll: failed to read body: {}", e);
                continue;
            }
        };
        // WAF/bot-detection: Alibaba Cloud WAF returns HTML instead of JSON.
        // Fail immediately — retrying won't help.
        if raw.starts_with("<!doctype") || raw.starts_with("<!DOCTYPE") || raw.starts_with("<html")
        {
            anyhow::bail!(
                "Qwen token endpoint blocked by WAF (bot detection). \
                 Try again in a few minutes or use a different network."
            );
        }
        let body: TokenPollResponse = match serde_json::from_str(&raw) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    "Qwen token poll: parse error: {} (status={}, body={})",
                    e,
                    status,
                    &raw[..raw.len().min(500)]
                );
                continue;
            }
        };

        if let Some(access_token) = body.access_token
            && !access_token.is_empty()
        {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let expires_in_ms = body.expires_in.unwrap_or(3600).saturating_mul(1000);
            return Ok(QwenCredentials {
                access_token,
                token_type: body.token_type.unwrap_or_else(|| "Bearer".into()),
                refresh_token: body.refresh_token.unwrap_or_default(),
                resource_url: body.resource_url.unwrap_or_default(),
                expiry_date: now_ms + expires_in_ms,
            });
        }

        match body.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                // qwen-cli multiplies by 1.5 capped at 10s
                let new_secs = (interval.as_secs_f32() * 1.5).min(10.0);
                interval = Duration::from_secs_f32(new_secs);
                continue;
            }
            Some("expired_token") | Some("access_denied") => {
                anyhow::bail!(
                    "Qwen device authorization {} ({}). Please retry.",
                    body.error.as_deref().unwrap_or("failed"),
                    body.error_description.unwrap_or_default()
                );
            }
            Some(other) if status.as_u16() == 401 => {
                anyhow::bail!("Qwen device code expired ({}). Please retry.", other);
            }
            Some(other) => {
                anyhow::bail!(
                    "Qwen token poll error: {} ({})",
                    other,
                    body.error_description.unwrap_or_default()
                );
            }
            None => continue,
        }
    }

    anyhow::bail!("Qwen device flow timed out before authorization completed");
}

/// Refresh an expired access token using the refresh_token grant.
pub async fn refresh_credentials(creds: &QwenCredentials) -> anyhow::Result<QwenCredentials> {
    if creds.refresh_token.is_empty() {
        anyhow::bail!("Qwen refresh: no refresh_token stored — re-authentication required");
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client
        .post(QWEN_TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", creds.refresh_token.as_str()),
            ("client_id", QWEN_OAUTH_CLIENT_ID),
        ])
        .send()
        .await?;

    let status = resp.status();
    if status.as_u16() == 400 {
        // qwen-cli treats 400 as "refresh token dead — re-auth required".
        anyhow::bail!(
            "Qwen refresh_token invalid (HTTP 400). Run /onboard:provider to re-authenticate."
        );
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Qwen refresh failed ({}): {}", status, body);
    }

    let raw = resp.text().await?;
    if raw.starts_with("<!doctype") || raw.starts_with("<!DOCTYPE") || raw.starts_with("<html") {
        anyhow::bail!(
            "Qwen refresh blocked by WAF (bot detection). \
             Try again in a few minutes or use a different network."
        );
    }
    let body: TokenPollResponse = serde_json::from_str(&raw)?;
    let access_token = body
        .access_token
        .ok_or_else(|| anyhow::anyhow!("Qwen refresh response missing access_token"))?;

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let expires_in_ms = body.expires_in.unwrap_or(3600).saturating_mul(1000);

    Ok(QwenCredentials {
        access_token,
        token_type: body.token_type.unwrap_or_else(|| creds.token_type.clone()),
        // Keep the previous refresh_token if the response omits one.
        refresh_token: body
            .refresh_token
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| creds.refresh_token.clone()),
        resource_url: body
            .resource_url
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| creds.resource_url.clone()),
        expiry_date: now_ms + expires_in_ms,
    })
}

// ── Browser helper ────────────────────────────────────────────────────────

/// Best-effort browser open. Falls back to printing — caller should always
/// also display the URL so the user can copy/paste manually.
pub fn open_browser(url: &str) -> bool {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "xdg-open"
    };

    let result = if cfg!(target_os = "windows") {
        std::process::Command::new(cmd)
            .args(["/C", "start", "", url])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    } else {
        std::process::Command::new(cmd)
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    };

    result.is_ok()
}

// ── Token manager ─────────────────────────────────────────────────────────

/// Runtime manager for the Qwen OAuth credentials. Holds the current
/// `QwenCredentials` behind an `RwLock` so the synchronous `token_fn` callback
/// from `OpenAIProvider` can read the live access token, and a background
/// task can rotate it before expiry.
pub struct QwenTokenManager {
    state: RwLock<QwenCredentials>,
    /// Last observed mtime (seconds since epoch) of `~/.qwen/oauth_creds.json`.
    /// Used by the background task to detect out-of-band refreshes by qwen-cli.
    last_cli_mtime: RwLock<u64>,
    /// If `Some(idx)`, this manager belongs to rotation account `idx` and
    /// persists refreshed tokens to `[[providers.qwen_accounts]][idx]`
    /// instead of the single-account `[providers.qwen]`.
    account_index: Option<usize>,
}

impl QwenTokenManager {
    pub fn new(creds: QwenCredentials) -> Self {
        Self {
            state: RwLock::new(creds),
            last_cli_mtime: RwLock::new(QwenCredentials::qwen_cli_mtime()),
            account_index: None,
        }
    }

    /// Create a token manager for a rotation account at the given index.
    pub fn new_rotation(creds: QwenCredentials, index: usize) -> Self {
        Self {
            state: RwLock::new(creds),
            last_cli_mtime: RwLock::new(QwenCredentials::qwen_cli_mtime()),
            account_index: Some(index),
        }
    }

    /// Read the current access token (sync, for the OpenAIProvider header callback).
    pub fn current_access_token(&self) -> String {
        self.state
            .read()
            .map(|c| c.access_token.clone())
            .unwrap_or_default()
    }

    /// Read the current chat-completions base URL (resource_url-derived).
    pub fn current_chat_url(&self) -> String {
        self.state
            .read()
            .map(|c| c.chat_completions_url())
            .unwrap_or_else(|_| QWEN_DEFAULT_CHAT_URL.to_string())
    }

    /// Snapshot the credentials.
    pub fn snapshot(&self) -> QwenCredentials {
        self.state
            .read()
            .map(|c| c.clone())
            .unwrap_or_else(|_| QwenCredentials {
                access_token: String::new(),
                token_type: "Bearer".into(),
                refresh_token: String::new(),
                resource_url: String::new(),
                expiry_date: 0,
            })
    }

    /// Persist refreshed credentials to the correct location based on whether
    /// this is a single-account or rotation account manager.
    fn persist_refreshed(&self, creds: &QwenCredentials) {
        match self.account_index {
            Some(idx) => {
                if let Err(e) = creds.persist_to_account_slot(idx) {
                    tracing::warn!(
                        "Failed to persist refreshed Qwen rotation account {}: {}",
                        idx,
                        e
                    );
                }
                // Rotation accounts do NOT touch ~/.qwen/oauth_creds.json —
                // that file is single-account territory shared with qwen-cli.
            }
            None => {
                if let Err(e) = creds.persist_to_keys() {
                    tracing::warn!("Failed to persist refreshed Qwen credentials: {}", e);
                }
                // Write back to ~/.qwen/oauth_creds.json so qwen-cli sees the
                // new token too. Keeps the two clients from fighting over refreshes.
                creds.write_back_to_qwen_cli();
            }
        }
    }

    /// Refresh the token if it's about to expire. Persists the new creds on success.
    pub async fn ensure_fresh(&self) -> anyhow::Result<()> {
        // First, check if qwen-cli has rotated the token on disk. If its
        // mtime moved forward, import the new creds before deciding whether
        // we still need to refresh — avoids burning our refresh_token when
        // qwen-cli already did the work.
        // (Only for single-account — rotation accounts don't share the cli file.)
        if self.account_index.is_none() {
            self.reload_from_qwen_cli_if_changed();
        }

        let needs_refresh = {
            let c = self
                .state
                .read()
                .map_err(|_| anyhow::anyhow!("Qwen token state poisoned"))?;
            !c.is_valid()
        };
        if !needs_refresh {
            return Ok(());
        }
        let snap = self.snapshot();
        let new_creds = refresh_credentials(&snap).await?;
        self.persist_refreshed(&new_creds);
        if let Ok(mut w) = self.state.write() {
            *w = new_creds;
        }
        if self.account_index.is_none()
            && let Ok(mut m) = self.last_cli_mtime.write()
        {
            *m = QwenCredentials::qwen_cli_mtime();
        }
        tracing::debug!(
            "Qwen access token refreshed (account: {:?})",
            self.account_index
        );
        Ok(())
    }

    /// Force refresh regardless of current expiry. Used after a 401/403.
    pub async fn force_refresh(&self) -> anyhow::Result<()> {
        // Check qwen-cli's file first — if it rotated the token since we
        // last looked, we can avoid burning our own refresh quota.
        // (Only for single-account — rotation accounts don't share the cli file.)
        if self.account_index.is_none() && self.reload_from_qwen_cli_if_changed() {
            tracing::info!("Qwen: picked up rotated token from qwen-cli — no refresh needed");
            return Ok(());
        }
        let snap = self.snapshot();
        let new_creds = refresh_credentials(&snap).await?;
        self.persist_refreshed(&new_creds);
        if let Ok(mut w) = self.state.write() {
            *w = new_creds;
        }
        if self.account_index.is_none()
            && let Ok(mut m) = self.last_cli_mtime.write()
        {
            *m = QwenCredentials::qwen_cli_mtime();
        }
        Ok(())
    }

    /// Check `~/.qwen/oauth_creds.json` mtime; if it moved forward since we
    /// last looked, re-import the credentials from disk. Returns `true` if
    /// creds were reloaded.
    fn reload_from_qwen_cli_if_changed(&self) -> bool {
        let current_mtime = QwenCredentials::qwen_cli_mtime();
        if current_mtime == 0 {
            return false;
        }
        let last = self.last_cli_mtime.read().map(|g| *g).unwrap_or(0);
        if current_mtime <= last {
            return false;
        }
        match QwenCredentials::import_from_qwen_cli() {
            Some(fresh) => {
                tracing::info!(
                    "Qwen: detected oauth_creds.json update (mtime {} -> {}) — reloading",
                    last,
                    current_mtime
                );
                if let Err(e) = fresh.persist_to_keys() {
                    tracing::warn!("Failed to persist reloaded Qwen creds to keys.toml: {}", e);
                }
                if let Ok(mut w) = self.state.write() {
                    *w = fresh;
                }
                if let Ok(mut m) = self.last_cli_mtime.write() {
                    *m = current_mtime;
                }
                true
            }
            None => {
                tracing::warn!("Qwen: oauth_creds.json changed but could not be parsed");
                false
            }
        }
    }

    /// Spawn a background task that proactively refreshes the token before
    /// it expires AND polls `~/.qwen/oauth_creds.json` every 30s for
    /// out-of-band updates from qwen-cli.
    pub fn start_background_refresh(self: Arc<Self>) {
        let acct_label = match self.account_index {
            Some(idx) => format!("rotation-{}", idx),
            None => "single".to_string(),
        };
        tokio::spawn(async move {
            // Initial check on startup
            if let Err(e) = self.ensure_fresh().await {
                tracing::warn!("Qwen [{}] initial token refresh failed: {}", acct_label, e);
            }

            loop {
                // Compute time until ~60s before expiry.
                let deadline_secs = {
                    let snap = self.snapshot();
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let remaining_ms = snap.expiry_date.saturating_sub(now_ms);
                    let remaining_secs = remaining_ms / 1000;
                    remaining_secs.saturating_sub(60).max(30)
                };

                // Wake every 30s (or at the refresh deadline, whichever is
                // sooner) to check qwen-cli's mtime.
                let tick_secs = deadline_secs.min(30);
                tokio::time::sleep(Duration::from_secs(tick_secs)).await;

                // Out-of-band pickup from qwen-cli — only for single-account.
                // Rotation accounts each have their own refresh_token and must
                // not read/write ~/.qwen/oauth_creds.json.
                if self.account_index.is_none() && self.reload_from_qwen_cli_if_changed() {
                    continue;
                }

                // Normal proactive refresh window?
                let should_refresh = {
                    let snap = self.snapshot();
                    !snap.is_valid()
                };
                if should_refresh && let Err(e) = self.ensure_fresh().await {
                    let msg = e.to_string();
                    if msg.contains("HTTP 400") {
                        if self.account_index.is_some() {
                            // Rotation account: just stop refreshing this account.
                            // The factory will see it as expired on next rebuild.
                            // Do NOT wipe single-account creds or qwen-cli file.
                            tracing::error!(
                                "Qwen [{}] refresh_token permanently dead — \
                                 stopping background refresh for this account.",
                                acct_label
                            );
                        } else {
                            tracing::error!(
                                "Qwen refresh_token permanently dead — wiping credentials. \
                                 Run /onboard:provider to re-authenticate."
                            );
                            QwenCredentials::wipe_dead_credentials();
                        }
                        break;
                    }
                    tracing::warn!(
                        "Qwen [{}] background token refresh failed: {}",
                        acct_label,
                        e
                    );
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        });
    }
}

// ── DashScope headers ─────────────────────────────────────────────────────

/// Version string sent in `User-Agent` and `X-DashScope-UserAgent`.
/// Must stay `QwenCode/<semver>` — the gateway validates the prefix.
const QWEN_CLI_VERSION: &str = "0.14.0";

/// Node-style arch token for `User-Agent` / `X-DashScope-UserAgent`.
/// qwen-cli uses Node's `process.arch` values directly.
fn node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "arm" => "arm",
        "x86" => "ia32",
        other => other,
    }
}

/// Platform tuple baked into `User-Agent` and `X-DashScope-UserAgent`.
/// qwen-cli constructs these as `${process.platform}; ${process.arch}`
/// where `platform` is `darwin` / `linux` / `win32`.
fn user_agent_platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other, // linux, freebsd, openbsd, android stay as-is
    };
    format!("{}; {}", os, node_arch())
}

/// Extra headers sent with every request to `portal.qwen.ai`.
///
/// These are the **exact four** headers that
/// `DashScopeOpenAICompatibleProvider.buildHeaders()` emits in
/// `@qwen-code/qwen-code`'s bundle (verified against
/// `packages/core/dist/src/core/openaiContentGenerator/provider/dashscope.js`):
///
/// ```text
/// User-Agent: QwenCode/<version> (<platform>; <arch>)
/// X-DashScope-CacheControl: enable
/// X-DashScope-UserAgent: QwenCode/<version> (<platform>; <arch>)
/// X-DashScope-AuthType: qwen-oauth
/// ```
///
/// The `openai` npm SDK auto-injects its own `x-stainless-*` telemetry
/// at runtime. We used to hardcode those values, but any byte-level
/// mismatch with what the real node+openai-v5.x combo produces gets
/// fingerprinted by the gateway and downgraded to a tight rate-limit
/// bucket — even on a token the CLI is happily using. The safest
/// behavior is to **not** send those headers at all and let the HTTP
/// stack emit its natural defaults. DashScope only gates on the four
/// headers above.
pub fn qwen_extra_headers() -> Vec<(String, String)> {
    let ua = format!("QwenCode/{} ({})", QWEN_CLI_VERSION, user_agent_platform());
    vec![
        ("User-Agent".to_string(), ua.clone()),
        ("X-DashScope-CacheControl".to_string(), "enable".to_string()),
        ("X-DashScope-UserAgent".to_string(), ua),
        ("X-DashScope-AuthType".to_string(), "qwen-oauth".to_string()),
    ]
}

// ── DashScope body shape ──────────────────────────────────────────────────

/// Stable per-process session id, mirroring qwen-cli's `metadata.sessionId`.
/// DashScope appears to use this for per-session quota tracking; reusing one
/// id for the lifetime of the process keeps us inside a single quota bucket
/// instead of looking like a fresh client every request.
fn qwen_session_id() -> &'static str {
    use std::sync::OnceLock;
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// Per-request id, mirroring qwen-cli's `metadata.promptId`. qwen-cli uses
/// a short hex string; we use 12 hex chars derived from a random u64.
fn qwen_prompt_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Simple xorshift for variability without pulling in `rand`.
    let mut x = nanos as u64 ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    format!("{:013x}", x & 0x000F_FFFF_FFFF_FFFF)
}

/// Vision-capable model identifiers recognized by qwen-cli's
/// `DashScopeOpenAICompatibleProvider.isVisionModel()`. The exact set
/// lives in the bundled CLI (`packages/core/.../provider/dashscope.js`):
/// exact match on `coder-model`, or prefix on `qwen-vl`, `qwen3-vl-plus`,
/// `qwen3.5-plus`.
fn is_vision_model(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    if m == "coder-model" {
        return true;
    }
    for prefix in ["qwen-vl", "qwen3-vl-plus", "qwen3.5-plus"] {
        if m.starts_with(prefix) {
            return true;
        }
    }
    false
}

/// Normalize an OpenAI `content` field to the array form qwen-cli uses
/// when attaching cache control. Mirrors `normalizeContentToArray`:
///   - `"hello"` → `[{"type":"text","text":"hello"}]`
///   - existing array → returned as-is (multi-modal user messages)
fn normalize_content_to_array(content: &serde_json::Value) -> Vec<serde_json::Value> {
    match content {
        serde_json::Value::String(s) => {
            vec![serde_json::json!({ "type": "text", "text": s })]
        }
        serde_json::Value::Array(arr) => arr.clone(),
        _ => Vec::new(),
    }
}

/// Apply `cache_control: {type: "ephemeral"}` to the LAST part of the
/// content array. Mirrors `addCacheControlToContentArray`.
fn add_cache_control_to_content(content: &serde_json::Value) -> serde_json::Value {
    let mut arr = normalize_content_to_array(content);
    if let Some(last) = arr.last_mut()
        && let Some(obj) = last.as_object_mut()
    {
        obj.insert(
            "cache_control".to_string(),
            serde_json::json!({ "type": "ephemeral" }),
        );
    }
    serde_json::Value::Array(arr)
}

/// Rewrite a serialized OpenAI chat-completions body into the exact dialect
/// that qwen-cli's `DashScopeOpenAICompatibleProvider.buildRequest` emits
/// against `portal.qwen.ai`.
///
/// Byte-for-byte behavior reference:
/// `packages/core/dist/src/core/openaiContentGenerator/provider/dashscope.js`
///
/// Transforms applied:
///   1. **Cache control** — `addDashScopeCacheControl(request, stream ? "all" : "system_only")`:
///      - System message (if any): content array-ified, last part tagged
///        `cache_control: {type: "ephemeral"}`.
///      - If `stream === true`: the **last message regardless of role**
///        gets the same treatment.
///      - If `stream === true` AND there are tools: the **last tool** gets
///        `cache_control: {type: "ephemeral"}` appended at the top level.
///      - All other messages are left untouched (plain strings stay strings).
///   2. **metadata** — `{sessionId, promptId}` added at the top level.
///      (qwen-cli also includes `channel` if it has one; OpenCrabs doesn't
///      surface a channel at this layer, so we omit it — the field is
///      optional in the CLI too.)
///   3. **vl_high_resolution_images: true** — added **only** when the
///      model is in the vision list. Previously we added this unconditionally,
///      which flagged text-only requests as non-cli traffic.
///   4. **No field stripping.** `temperature`, `top_p`, `tool_choice`,
///      `max_completion_tokens`, `include_reasoning` etc. pass through.
///      DashScope's fingerprint expects these to be present when the
///      client supplies them.
///   5. **No forced max_tokens.** qwen-cli runs `applyOutputTokenLimit`
///      which caps but never synthesizes the field. We leave it absent
///      when the caller didn't provide one.
pub fn qwen_body_transform(mut body: serde_json::Value) -> serde_json::Value {
    let obj = match body.as_object_mut() {
        Some(o) => o,
        None => return body,
    };

    // Streaming flag drives the cache-control scope: "all" (system + last
    // message + last tool) vs. "system_only" (system only).
    let is_streaming = obj.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    // ── 1. Cache control on messages ────────────────────────────────────
    if let Some(serde_json::Value::Array(messages)) = obj.get_mut("messages") {
        let msg_count = messages.len();
        if msg_count > 0 {
            let system_idx = messages
                .iter()
                .position(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"));
            let last_idx = msg_count - 1;

            for (i, msg) in messages.iter_mut().enumerate() {
                let should_cache = (Some(i) == system_idx) || (is_streaming && i == last_idx);
                if !should_cache {
                    continue;
                }
                let Some(msg_obj) = msg.as_object_mut() else {
                    continue;
                };
                // Skip messages with null / missing content — qwen-cli
                // returns them unchanged.
                let content = match msg_obj.get("content") {
                    Some(c) if !c.is_null() => c.clone(),
                    _ => continue,
                };
                msg_obj.insert(
                    "content".to_string(),
                    add_cache_control_to_content(&content),
                );
            }
        }
    }

    // ── 2. Metadata ─────────────────────────────────────────────────────
    obj.insert(
        "metadata".to_string(),
        serde_json::json!({
            "sessionId": qwen_session_id(),
            "promptId": qwen_prompt_id(),
        }),
    );

    // ── 3. vl_high_resolution_images (only for vision models) ───────────
    let model = obj
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if is_vision_model(&model) {
        obj.insert(
            "vl_high_resolution_images".to_string(),
            serde_json::Value::Bool(true),
        );
    }

    // ── 4. Cache control on LAST tool (streaming only) ──────────────────
    if is_streaming
        && let Some(serde_json::Value::Array(tools)) = obj.get_mut("tools")
        && let Some(last) = tools.last_mut()
        && let Some(tool_obj) = last.as_object_mut()
    {
        tool_obj.insert(
            "cache_control".to_string(),
            serde_json::json!({ "type": "ephemeral" }),
        );
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_pair_is_well_formed() {
        let p = PkcePair::generate();
        // base64url-no-pad of 32 bytes is 43 chars
        assert_eq!(p.verifier.len(), 43);
        // sha256 -> 32 bytes -> base64url-no-pad -> 43 chars
        assert_eq!(p.challenge.len(), 43);
        assert!(!p.verifier.contains('='));
        assert!(!p.challenge.contains('='));
    }

    #[test]
    fn chat_url_default() {
        let creds = QwenCredentials {
            access_token: "x".into(),
            token_type: "Bearer".into(),
            refresh_token: "y".into(),
            resource_url: String::new(),
            expiry_date: 0,
        };
        let url = creds.chat_completions_url();
        assert!(url.ends_with("/v1/chat/completions"));
        assert!(url.starts_with("https://"));
    }

    #[test]
    fn chat_url_from_resource_url() {
        let creds = QwenCredentials {
            access_token: "x".into(),
            token_type: "Bearer".into(),
            refresh_token: "y".into(),
            resource_url: "portal.qwen.ai".into(),
            expiry_date: 0,
        };
        assert_eq!(
            creds.chat_completions_url(),
            "https://portal.qwen.ai/v1/chat/completions"
        );
    }

    #[test]
    fn chat_url_handles_existing_v1() {
        let creds = QwenCredentials {
            access_token: "x".into(),
            token_type: "Bearer".into(),
            refresh_token: "y".into(),
            resource_url: "https://example.com/v1".into(),
            expiry_date: 0,
        };
        assert_eq!(
            creds.chat_completions_url(),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn is_valid_returns_false_for_expired() {
        let creds = QwenCredentials {
            access_token: "x".into(),
            token_type: "Bearer".into(),
            refresh_token: "y".into(),
            resource_url: String::new(),
            expiry_date: 0,
        };
        assert!(!creds.is_valid());
    }

    #[test]
    fn extra_headers_match_qwen_cli_exactly() {
        // qwen-cli's DashScopeOpenAICompatibleProvider.buildHeaders() emits
        // exactly these 4 headers. Any extras (x-stainless-*, etc.) were
        // getting fingerprinted and downgraded to a tight rate-limit bucket.
        let h = qwen_extra_headers();
        let names: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(h.len(), 4, "expected exactly 4 headers, got {:?}", names);
        assert!(names.contains(&"User-Agent"));
        assert!(names.contains(&"X-DashScope-CacheControl"));
        assert!(names.contains(&"X-DashScope-UserAgent"));
        assert!(names.contains(&"X-DashScope-AuthType"));
        // UA and X-DashScope-UserAgent must be identical byte strings.
        let ua = h
            .iter()
            .find(|(k, _)| k == "User-Agent")
            .map(|(_, v)| v.clone())
            .unwrap();
        let ds_ua = h
            .iter()
            .find(|(k, _)| k == "X-DashScope-UserAgent")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(ua, ds_ua);
        assert!(ua.starts_with("QwenCode/"));
        // X-DashScope-AuthType: qwen-oauth
        let auth = h
            .iter()
            .find(|(k, _)| k == "X-DashScope-AuthType")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(auth, "qwen-oauth");
    }

    /// Baseline body resembling what OpenAIRequest serializes to for a small
    /// chat turn with one tool. The transform must convert it into the exact
    /// qwen-cli dialect captured in `/tmp/qwen-req.json`.
    fn sample_body() -> serde_json::Value {
        serde_json::json!({
            "model": "coder-model",
            "messages": [
                { "role": "system", "content": "sys prompt" },
                { "role": "user", "content": "first user" },
                { "role": "assistant", "content": "asst reply" },
                { "role": "user", "content": "last user" }
            ],
            "temperature": 0.7,
            "top_p": 0.95,
            "tool_choice": "auto",
            "max_completion_tokens": 8192,
            "include_reasoning": true,
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": [
                {
                    "type": "function",
                    "function": { "name": "first_tool", "description": "", "parameters": {} }
                },
                {
                    "type": "function",
                    "function": { "name": "last_tool", "description": "", "parameters": {} }
                }
            ]
        })
    }

    #[test]
    fn body_transform_cache_control_streaming_system_and_last_message() {
        // Streaming body → cacheControl = "all". System gets cache_control
        // on its last content part; the LAST message (index 3, a user)
        // also gets cache_control. First user (index 1) and assistant
        // (index 2) are left untouched as plain strings — qwen-cli does
        // NOT array-ify non-cached messages.
        let out = qwen_body_transform(sample_body());
        let msgs = out.get("messages").and_then(|v| v.as_array()).unwrap();

        // [0] system — array with cache_control on last part
        let sys = &msgs[0];
        assert_eq!(sys["role"], "system");
        assert!(sys["content"].is_array());
        assert_eq!(sys["content"][0]["type"], "text");
        assert_eq!(sys["content"][0]["cache_control"]["type"], "ephemeral");

        // [1] first user — UNTOUCHED, still a plain string
        let u1 = &msgs[1];
        assert!(
            u1["content"].is_string(),
            "first user should stay as plain string, got {:?}",
            u1["content"]
        );

        // [2] assistant — UNTOUCHED, still a plain string
        let asst = &msgs[2];
        assert!(asst["content"].is_string());

        // [3] last user (== last message overall) — array with cache_control
        let u2 = &msgs[3];
        assert!(u2["content"].is_array());
        assert_eq!(u2["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn body_transform_non_streaming_only_tags_system() {
        // Non-streaming → cacheControl = "system_only". Last message is
        // left untouched even though it's a user message.
        let mut body = sample_body();
        body["stream"] = serde_json::json!(false);
        let out = qwen_body_transform(body);
        let msgs = out.get("messages").and_then(|v| v.as_array()).unwrap();

        // system still gets tagged
        assert!(msgs[0]["content"].is_array());
        assert_eq!(msgs[0]["content"][0]["cache_control"]["type"], "ephemeral");

        // last message (user) untouched
        assert!(msgs[3]["content"].is_string());
    }

    #[test]
    fn body_transform_preserves_all_fields() {
        // Verify we no longer strip temperature, top_p, tool_choice,
        // max_completion_tokens, include_reasoning — DashScope expects
        // these to pass through when the caller supplies them.
        let out = qwen_body_transform(sample_body());
        let obj = out.as_object().unwrap();
        assert_eq!(obj.get("temperature"), Some(&serde_json::json!(0.7)));
        assert_eq!(obj.get("top_p"), Some(&serde_json::json!(0.95)));
        assert_eq!(obj.get("tool_choice"), Some(&serde_json::json!("auto")));
        assert_eq!(
            obj.get("max_completion_tokens"),
            Some(&serde_json::json!(8192))
        );
        assert_eq!(obj.get("include_reasoning"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn body_transform_adds_metadata_with_session_and_prompt_ids() {
        let out = qwen_body_transform(sample_body());
        let meta = out.get("metadata").unwrap();
        assert!(meta["sessionId"].is_string());
        assert!(meta["promptId"].is_string());
    }

    #[test]
    fn body_transform_vl_flag_only_for_vision_models() {
        // coder-model is in the vision list → flag present
        let out = qwen_body_transform(sample_body());
        assert_eq!(out["vl_high_resolution_images"], true);

        // A non-vision model → flag absent (previously we forced it on)
        let mut body = sample_body();
        body["model"] = serde_json::json!("qwen3-32b");
        let out = qwen_body_transform(body);
        assert!(
            out.as_object()
                .unwrap()
                .get("vl_high_resolution_images")
                .is_none(),
            "text-only model should not carry vl_high_resolution_images"
        );
    }

    #[test]
    fn body_transform_does_not_force_max_tokens() {
        // qwen-cli never synthesizes max_tokens. Body without it should
        // stay without it — previously we forced 32000 and widened the
        // fingerprint gap.
        let mut body = sample_body();
        body.as_object_mut().unwrap().remove("max_tokens");
        let out = qwen_body_transform(body);
        assert!(out.as_object().unwrap().get("max_tokens").is_none());
    }

    #[test]
    fn body_transform_tags_last_tool_only_when_streaming() {
        // Streaming → last tool gets cache_control
        let out = qwen_body_transform(sample_body());
        let tools = out.get("tools").and_then(|v| v.as_array()).unwrap();
        assert!(tools[0].get("cache_control").is_none());
        assert_eq!(tools[1]["cache_control"]["type"], "ephemeral");

        // Non-streaming → tools untouched
        let mut body = sample_body();
        body["stream"] = serde_json::json!(false);
        let out = qwen_body_transform(body);
        let tools = out.get("tools").and_then(|v| v.as_array()).unwrap();
        assert!(tools[0].get("cache_control").is_none());
        assert!(tools[1].get("cache_control").is_none());
    }

    #[test]
    fn body_transform_preserves_existing_max_tokens() {
        let mut body = sample_body();
        body["max_tokens"] = serde_json::json!(4096);
        let out = qwen_body_transform(body);
        assert_eq!(out["max_tokens"], 4096);
    }

    #[test]
    fn body_transform_cache_control_on_multimodal_last_message() {
        // Last message is already an array (multi-modal image+text user).
        // Streaming → cache_control should be added to the LAST content
        // part of that array, not destroy the existing structure.
        let body = serde_json::json!({
            "model": "coder-model",
            "stream": true,
            "messages": [
                { "role": "system", "content": "sys" },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "look at this" },
                        { "type": "image_url", "image_url": { "url": "data:..." } }
                    ]
                }
            ]
        });
        let out = qwen_body_transform(body);
        let msgs = out["messages"].as_array().unwrap();
        let u = &msgs[1];
        let content = u["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        // original image_url part preserved + tagged with cache_control
        // because it's the LAST part of the LAST message
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn is_vision_model_matches_qwen_cli_list() {
        assert!(is_vision_model("coder-model"));
        assert!(is_vision_model("qwen-vl-max"));
        assert!(is_vision_model("qwen-vl-max-latest"));
        assert!(is_vision_model("qwen3-vl-plus"));
        assert!(is_vision_model("qwen3.5-plus"));
        assert!(is_vision_model("CODER-MODEL")); // case-insensitive
        assert!(!is_vision_model("qwen3-32b"));
        assert!(!is_vision_model("qwen-max"));
        assert!(!is_vision_model(""));
    }

    #[test]
    fn session_id_is_stable_within_process() {
        let a = qwen_session_id();
        let b = qwen_session_id();
        assert_eq!(a, b);
    }

    #[test]
    fn prompt_id_is_13_hex_chars() {
        let id = qwen_prompt_id();
        assert_eq!(id.len(), 13);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
