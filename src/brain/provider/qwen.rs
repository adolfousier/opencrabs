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

        qwen.insert("enabled".to_string(), toml::Value::Boolean(true));
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
        // Seed default_model on first write so the user sees what's in use.
        if !qwen.contains_key("default_model") {
            qwen.insert(
                "default_model".to_string(),
                toml::Value::String(QWEN_OAUTH_MODEL.to_string()),
            );
        }

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

    let dcr: DeviceCodeResponse = resp.json().await?;
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
        let body: TokenPollResponse = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Qwen token poll: failed to parse body: {}", e);
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

    let client = reqwest::Client::new();
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

    let body: TokenPollResponse = resp.json().await?;
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
}

impl QwenTokenManager {
    pub fn new(creds: QwenCredentials) -> Self {
        Self {
            state: RwLock::new(creds),
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

    /// Refresh the token if it's about to expire. Persists the new creds on success.
    pub async fn ensure_fresh(&self) -> anyhow::Result<()> {
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
        if let Err(e) = new_creds.persist_to_keys() {
            tracing::warn!("Failed to persist refreshed Qwen credentials: {}", e);
        }
        if let Ok(mut w) = self.state.write() {
            *w = new_creds;
        }
        tracing::debug!("Qwen access token refreshed");
        Ok(())
    }

    /// Force refresh regardless of current expiry. Used after a 401/403.
    pub async fn force_refresh(&self) -> anyhow::Result<()> {
        let snap = self.snapshot();
        let new_creds = refresh_credentials(&snap).await?;
        if let Err(e) = new_creds.persist_to_keys() {
            tracing::warn!("Failed to persist force-refreshed Qwen credentials: {}", e);
        }
        if let Ok(mut w) = self.state.write() {
            *w = new_creds;
        }
        Ok(())
    }

    /// Spawn a background task that proactively refreshes the token before
    /// it expires. Mirrors the Copilot manager's pattern.
    pub fn start_background_refresh(self: Arc<Self>) {
        tokio::spawn(async move {
            // Initial check on startup
            if let Err(e) = self.ensure_fresh().await {
                tracing::warn!("Qwen initial token refresh failed: {}", e);
            }

            loop {
                // Sleep until ~60s before expiry (min 60s between attempts).
                let sleep_secs = {
                    let snap = self.snapshot();
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let remaining_ms = snap.expiry_date.saturating_sub(now_ms);
                    let remaining_secs = remaining_ms / 1000;
                    remaining_secs.saturating_sub(60).max(60)
                };
                tokio::time::sleep(Duration::from_secs(sleep_secs)).await;

                if let Err(e) = self.ensure_fresh().await {
                    tracing::warn!("Qwen background token refresh failed: {}", e);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        });
    }
}

// ── DashScope headers ─────────────────────────────────────────────────────

/// Extra headers required by Qwen's DashScope OpenAI-compatible endpoint.
///
/// `portal.qwen.ai` aggressively fingerprints requests: any client that
/// doesn't look like qwen-code-cli (same UA, same `x-stainless-*` SDK
/// telemetry headers, same DashScope headers) gets either 400 or instant
/// 429 rate-limited even with a valid OAuth token. Verified empirically
/// by comparing our requests to qwen-cli's live traffic (captured via
/// Node `--require` preload fetch interceptor): stripping the stainless
/// headers causes instant rate-limit on a token that qwen-cli is
/// happily using. We must impersonate qwen-cli exactly.
///
/// - `X-DashScope-AuthType: qwen-oauth` — required, tells DashScope this is OAuth.
/// - `X-DashScope-UserAgent: QwenCode/...` — required, must look like qwen-cli.
/// - `X-DashScope-CacheControl: enable` — opts into ephemeral prompt caching.
/// - `User-Agent: QwenCode/...` — also validated by the gateway.
/// - `x-stainless-*` — OpenAI SDK telemetry, used by the gateway's
///   anti-scraping fingerprint to identify "authentic" qwen-cli traffic.
pub fn qwen_extra_headers() -> Vec<(String, String)> {
    // Hardcoded to match qwen-code-cli's format. Bumping these is fine as
    // long as the shape stays the same. Node-style platform tuple is what
    // qwen-cli sends from macOS arm64.
    let ua = "QwenCode/0.14.0 (darwin; arm64)".to_string();
    vec![
        ("X-DashScope-CacheControl".to_string(), "enable".to_string()),
        ("X-DashScope-UserAgent".to_string(), ua.clone()),
        ("X-DashScope-AuthType".to_string(), "qwen-oauth".to_string()),
        ("User-Agent".to_string(), ua),
        // Pretend to be the `openai` npm package v5.11.0 running on Node.
        // These exact values are what qwen-cli ships; the gateway
        // rate-limits anything missing them.
        ("x-stainless-arch".to_string(), "arm64".to_string()),
        ("x-stainless-lang".to_string(), "js".to_string()),
        ("x-stainless-os".to_string(), "MacOS".to_string()),
        (
            "x-stainless-package-version".to_string(),
            "5.11.0".to_string(),
        ),
        ("x-stainless-retry-count".to_string(), "0".to_string()),
        ("x-stainless-runtime".to_string(), "node".to_string()),
        (
            "x-stainless-runtime-version".to_string(),
            "v25.9.0".to_string(),
        ),
    ]
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
    fn extra_headers_include_dashscope() {
        let h = qwen_extra_headers();
        let names: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"X-DashScope-CacheControl"));
        assert!(names.contains(&"X-DashScope-AuthType"));
        assert!(names.contains(&"User-Agent"));
    }
}
