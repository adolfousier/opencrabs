//! Configuration types, defaults, loading, and validation.

use super::crabrace::CrabraceConfig;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Flag set when Config::load() recovered from a last-known-good snapshot.
static CONFIG_RECOVERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Unknown top-level keys found in config.toml (possible typos).
static CONFIG_TYPO_WARNINGS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Mutex protecting read-modify-write cycles on config.toml / keys.toml.
/// Without this, concurrent `write_key` calls can race: one reads while
/// another is mid-write, gets a partial/empty file, parses it as empty,
/// and overwrites the real config with an empty table.
static CONFIG_FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Crabrace integration configuration
    #[serde(default)]
    pub crabrace: CrabraceConfig,

    /// Database configuration
    #[serde(default)]
    pub database: DatabaseConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// Debug options
    #[serde(default)]
    pub debug: DebugConfig,

    /// LLM provider configurations
    #[serde(default)]
    pub providers: ProviderConfigs,

    /// Messaging channel integrations
    #[serde(default)]
    pub channels: ChannelsConfig,

    /// Agent behaviour configuration
    #[serde(default)]
    pub agent: AgentConfig,

    /// Daemon mode configuration (systemd / launchd service)
    #[serde(default)]
    pub daemon: DaemonConfig,

    /// A2A (Agent-to-Agent) protocol gateway configuration
    #[serde(default, alias = "gateway")]
    pub a2a: A2aConfig,

    /// Image generation and vision configuration
    #[serde(default)]
    pub image: ImageConfig,

    /// Cron job defaults
    #[serde(default)]
    pub cron: CronConfig,
}

/// Daemon mode configuration (systemd / launchd service).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Health check HTTP port. When set, `opencrabs daemon` binds a tiny HTTP
    /// server on `0.0.0.0:<port>` that responds to `GET /health` with 200 OK.
    /// Useful for systemd watchdog, uptime monitors, and external health probes.
    #[serde(default)]
    pub health_port: Option<u16>,
}

/// A2A (Agent-to-Agent) protocol gateway configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aConfig {
    /// Whether the A2A gateway is enabled (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Bind address (default: "127.0.0.1")
    #[serde(default = "default_a2a_bind")]
    pub bind: String,

    /// Gateway port (default: 18790)
    #[serde(default = "default_a2a_port")]
    pub port: u16,

    /// Allowed CORS origins — must be set explicitly, no cross-origin requests allowed by default
    #[serde(default)]
    pub allowed_origins: Vec<String>,

    /// Optional API key for authenticating incoming A2A requests (Bearer token).
    /// If set, all JSON-RPC requests must include `Authorization: Bearer <key>`.
    /// If unset, no authentication is required (suitable for loopback-only use).
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_a2a_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_a2a_port() -> u16 {
    18790
}

impl Default for A2aConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_a2a_bind(),
            port: default_a2a_port(),
            allowed_origins: vec![],
            api_key: None,
        }
    }
}

/// Messaging channel integrations configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub whatsapp: WhatsAppConfig,
    #[serde(default)]
    pub slack: SlackConfig,
    #[serde(default)]
    pub trello: TrelloConfig,
    #[serde(default)]
    pub signal: SignalConfig,
    #[serde(default)]
    pub google_chat: GoogleChatConfig,
    #[serde(default)]
    pub imessage: IMessageConfig,
}

/// When the bot should respond to messages in group channels.
/// DMs always get a response regardless of this setting.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RespondTo {
    /// Respond to all messages from allowed users
    All,
    /// Only respond to direct messages, ignore group channels entirely
    DmOnly,
    /// Only respond when @mentioned (or replied-to on Telegram)
    #[default]
    Mention,
}

/// Deserialize `allowed_users` from either a TOML integer array (legacy) or string array.
fn deser_users_compat<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Int(i64),
        Str(String),
    }
    Vec::<NumOrStr>::deserialize(d).map(|v| {
        v.into_iter()
            .map(|x| match x {
                NumOrStr::Int(n) => n.to_string(),
                NumOrStr::Str(s) => s,
            })
            .collect()
    })
}

/// Telegram channel configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: Option<String>,
    /// Allowlisted Telegram user IDs (numeric). Accepts int or string arrays.
    #[serde(default, deserialize_with = "deser_users_compat")]
    pub allowed_users: Vec<String>,
    /// Restrict bot to specific channel IDs. Empty = all channels. DMs always pass.
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    /// When the bot should respond: "all", "dm_only", or "mention" (default)
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Idle session timeout in hours for non-owner sessions.
    #[serde(default)]
    pub session_idle_hours: Option<f64>,
}

/// Discord channel configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: Option<String>,
    /// Allowlisted Discord user IDs (numeric). Accepts int or string arrays.
    #[serde(default, deserialize_with = "deser_users_compat")]
    pub allowed_users: Vec<String>,
    /// Restrict bot to specific channel IDs. Empty = all channels.
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    /// When the bot should respond: "all", "dm_only", or "mention" (default)
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Idle session timeout in hours for non-owner sessions.
    #[serde(default)]
    pub session_idle_hours: Option<f64>,
}

/// Slack channel configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlackConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bot token (xoxb-...)
    #[serde(default)]
    pub token: Option<String>,
    /// App-level token for Socket Mode (xapp-...)
    #[serde(default)]
    pub app_token: Option<String>,
    /// Allowlisted Slack user IDs (U12345678). Accepts int or string arrays.
    #[serde(default, deserialize_with = "deser_users_compat")]
    pub allowed_users: Vec<String>,
    /// Restrict bot to specific channel IDs. Empty = all channels.
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    /// When the bot should respond: "all", "dm_only", or "mention" (default)
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Idle session timeout in hours for non-owner sessions.
    #[serde(default)]
    pub session_idle_hours: Option<f64>,
}

/// WhatsApp channel configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WhatsAppConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Allowlisted phone numbers (E.164 format: "+15551234567").
    /// Empty = accept messages from everyone (not recommended for business numbers).
    #[serde(default)]
    pub allowed_phones: Vec<String>,
    /// Idle session timeout in hours for non-owner sessions.
    #[serde(default)]
    pub session_idle_hours: Option<f64>,
}

/// Trello channel configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrelloConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Trello API Token
    #[serde(default)]
    pub token: Option<String>,
    /// Trello API Key (stored as app_token for keys.toml symmetry)
    #[serde(default)]
    pub app_token: Option<String>,
    /// Allowlisted Trello member IDs. Empty = respond to all members.
    #[serde(default, deserialize_with = "deser_users_compat")]
    pub allowed_users: Vec<String>,
    /// Board IDs to monitor for @mentions.
    /// Accepts the old `allowed_channels` key as an alias for migration compatibility.
    #[serde(default, alias = "allowed_channels")]
    pub board_ids: Vec<String>,
    /// Optional polling interval in seconds. Absent or 0 = no polling (tool-only mode).
    #[serde(default)]
    pub poll_interval_secs: Option<u64>,
    /// Idle session timeout in hours for non-owner sessions.
    #[serde(default)]
    pub session_idle_hours: Option<f64>,
}

/// Signal channel configuration (placeholder — not yet implemented)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SignalConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Allowlisted phone numbers (E.164 format)
    #[serde(default)]
    pub allowed_phones: Vec<String>,
    /// Idle session timeout in hours.
    #[serde(default)]
    pub session_idle_hours: Option<f64>,
}

/// Google Chat channel configuration (placeholder — not yet implemented)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GoogleChatConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: Option<String>,
    /// Allowlisted user IDs. Accepts int or string arrays.
    #[serde(default, deserialize_with = "deser_users_compat")]
    pub allowed_users: Vec<String>,
    /// Idle session timeout in hours.
    #[serde(default)]
    pub session_idle_hours: Option<f64>,
}

/// iMessage channel configuration (placeholder — not yet implemented)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IMessageConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Allowlisted phone numbers (E.164 format)
    #[serde(default)]
    pub allowed_phones: Vec<String>,
    /// Idle session timeout in hours.
    #[serde(default)]
    pub session_idle_hours: Option<f64>,
}

/// STT mode: API (Groq Whisper) or Local (whisper.cpp)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SttMode {
    #[default]
    Api,
    Local,
}

/// TTS mode: API (OpenAI) or Local (Piper)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TtsMode {
    #[default]
    Api,
    Local,
}

/// Runtime voice configuration — assembled from providers.stt / providers.tts.
/// NOT serialized to config file.
#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub stt_enabled: bool,
    pub stt_mode: SttMode,
    pub local_stt_model: String,
    pub tts_enabled: bool,
    pub tts_mode: TtsMode,
    pub tts_voice: String,
    pub tts_model: String,
    pub local_tts_voice: String,
    pub stt_provider: Option<ProviderConfig>,
    pub tts_provider: Option<ProviderConfig>,
}

fn default_local_stt_model() -> String {
    "local-tiny".to_string()
}
fn default_tts_voice() -> String {
    "echo".to_string()
}
fn default_tts_model() -> String {
    "gpt-4o-mini-tts".to_string()
}
fn default_local_tts_voice() -> String {
    "ryan".to_string()
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_enabled: false,
            stt_mode: SttMode::default(),
            local_stt_model: default_local_stt_model(),
            tts_enabled: false,
            tts_mode: TtsMode::default(),
            tts_voice: default_tts_voice(),
            tts_model: default_tts_model(),
            local_tts_voice: default_local_tts_voice(),
            stt_provider: None,
            tts_provider: None,
        }
    }
}

/// Image generation and vision configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageConfig {
    #[serde(default)]
    pub generation: ImageGenerationConfig,
    #[serde(default)]
    pub vision: ImageVisionConfig,
}

/// Image generation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_image_model")]
    pub model: String,
    /// Loaded from keys.toml at runtime, never serialized to config.toml
    #[serde(skip, default)]
    pub api_key: Option<String>,
}

impl Default for ImageGenerationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: default_image_model(),
            api_key: None,
        }
    }
}

/// Image vision configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageVisionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_image_model")]
    pub model: String,
    /// Loaded from keys.toml at runtime, never serialized to config.toml
    #[serde(skip, default)]
    pub api_key: Option<String>,
}

impl Default for ImageVisionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: default_image_model(),
            api_key: None,
        }
    }
}

fn default_image_model() -> String {
    "gemini-3.1-flash-image-preview".to_string()
}

/// Agent behaviour configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Approval policy: "ask", "auto-session", "auto-always"
    #[serde(default = "default_approval_policy")]
    pub approval_policy: String,

    /// Maximum concurrent tool calls
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,

    /// Context window limit in tokens (default: 200000)
    #[serde(default = "default_context_limit")]
    pub context_limit: u32,

    /// Max output tokens for API calls (default: 65536)
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    /// Default provider for spawned sub-agents (e.g., "openrouter", "anthropic", "custom:lmstudio").
    /// If unset, sub-agents inherit the parent session's active provider.
    #[serde(default)]
    pub subagent_provider: Option<String>,

    /// Default model for spawned sub-agents (e.g., "claude-sonnet-4-6").
    /// Only used when subagent_provider is set.
    #[serde(default)]
    pub subagent_model: Option<String>,

    /// Auto-install new releases on startup without prompting (default: true).
    /// When false, the user is shown an update prompt dialog instead.
    #[serde(default = "default_auto_update")]
    pub auto_update: bool,

    /// Override provider for autonomous RSI self-improvement cycles (e.g. "zhipu", "minimax").
    /// RSI runs on its own provider chain so it never competes with chat or sub-agents for quota.
    /// When set, RSI jobs use this provider instead of the session's active one.
    #[serde(default)]
    pub self_improvement_provider: Option<String>,

    /// Override model for RSI self-improvement cycles. Only used when self_improvement_provider is set.
    /// Prefer cheap, fast models for autonomous analysis — results are deterministic.
    #[serde(default)]
    pub self_improvement_model: Option<String>,
}

fn default_approval_policy() -> String {
    "auto-always".to_string()
}

fn default_max_concurrent() -> u32 {
    4
}

fn default_context_limit() -> u32 {
    200_000
}

fn default_max_tokens() -> u32 {
    65536
}

fn default_auto_update() -> bool {
    true
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            approval_policy: default_approval_policy(),
            max_concurrent: default_max_concurrent(),
            context_limit: default_context_limit(),
            max_tokens: default_max_tokens(),
            subagent_provider: None,
            subagent_model: None,
            auto_update: default_auto_update(),
            self_improvement_provider: None,
            self_improvement_model: None,
        }
    }
}

/// Cron job default settings.
///
/// When a cron job has no `provider` or `model` set, these defaults are used
/// instead of the system's active provider. Useful for routing cron jobs to
/// cheaper providers while keeping the interactive session on a premium one.
///
/// Example in config.toml:
/// ```toml
/// [cron]
/// default_provider = "minimax"
/// default_model = "MiniMax-M2.7"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CronConfig {
    /// Default provider for cron jobs without an explicit provider
    #[serde(default)]
    pub default_provider: Option<String>,

    /// Default model for cron jobs without an explicit model
    #[serde(default)]
    pub default_model: Option<String>,
}

/// Debug configuration options
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DebugConfig {
    /// Enable LSP debug logging
    #[serde(default)]
    pub debug_lsp: bool,

    /// Enable profiling
    #[serde(default)]
    pub profiling: bool,
}

/// LLM Provider configurations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfigs {
    /// Anthropic configuration
    #[serde(default)]
    pub anthropic: Option<ProviderConfig>,

    /// OpenAI configuration (official API)
    #[serde(default)]
    pub openai: Option<ProviderConfig>,

    /// OpenRouter configuration
    #[serde(default)]
    pub openrouter: Option<ProviderConfig>,

    /// Minimax configuration
    #[serde(default)]
    pub minimax: Option<ProviderConfig>,

    /// z.ai GLM configuration (supports API and Coding endpoints)
    #[serde(default)]
    pub zhipu: Option<ProviderConfig>,

    /// Named custom OpenAI-compatible providers (e.g. [providers.custom.ollama])
    #[serde(default, deserialize_with = "deserialize_custom_providers")]
    pub custom: Option<BTreeMap<String, ProviderConfig>>,

    /// GitHub Copilot configuration (uses OAuth device flow token)
    #[serde(default)]
    pub github: Option<ProviderConfig>,

    /// Google Gemini configuration
    #[serde(default)]
    pub gemini: Option<ProviderConfig>,

    /// Claude CLI (Max subscription) — direct subprocess, no proxy needed
    #[serde(default)]
    pub claude_cli: Option<ProviderConfig>,

    /// OpenCode CLI — direct subprocess, access to opencode's free models
    #[serde(default)]
    pub opencode_cli: Option<ProviderConfig>,

    /// Qwen Code CLI — direct subprocess, Qwen OAuth (1k free/day) or custom APIs
    #[serde(default)]
    pub qwen_code_cli: Option<ProviderConfig>,

    /// Qwen native API — direct OAuth device flow against chat.qwen.ai,
    /// OpenCrabs orchestrates tools and context. No CLI subprocess.
    #[serde(default)]
    pub qwen: Option<ProviderConfig>,

    /// Multiple Qwen OAuth accounts for rotation. When present (len >= 2),
    /// the factory builds a `RotatingQwenProvider` that round-robins on 429
    /// before falling to the outer FallbackProvider chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qwen_accounts: Option<Vec<ProviderConfig>>,

    /// AWS Bedrock configuration
    #[serde(default)]
    pub bedrock: Option<ProviderConfig>,

    /// VertexAI configuration
    #[serde(default)]
    pub vertex: Option<ProviderConfig>,

    /// STT (Speech-to-Text) provider configurations
    #[serde(default)]
    pub stt: Option<SttProviders>,

    /// TTS (Text-to-Speech) provider configurations
    #[serde(default)]
    pub tts: Option<TtsProviders>,

    /// Web search provider configurations
    #[serde(default)]
    pub web_search: Option<WebSearchProviders>,

    /// Image provider configurations (e.g. [providers.image.gemini])
    #[serde(default)]
    pub image: Option<ImageProviders>,

    /// Fallback provider configuration (under [providers.fallback] in config)
    #[serde(default)]
    pub fallback: Option<FallbackProviderConfig>,
}

impl ProviderConfigs {
    /// Get the first enabled custom provider (name + config)
    pub fn active_custom(&self) -> Option<(&str, &ProviderConfig)> {
        self.custom
            .as_ref()?
            .iter()
            .find(|(_, cfg)| cfg.enabled)
            .map(|(name, cfg)| (name.as_str(), cfg))
    }

    /// Get a specific custom provider by name (case-insensitive, normalized)
    pub fn custom_by_name(&self, name: &str) -> Option<&ProviderConfig> {
        let normalized = normalize_toml_key(name);
        self.custom.as_ref()?.get(&normalized)
    }

    /// Return `(provider_name, default_model)` for the currently active provider,
    /// using the same priority order as `factory::create_provider`.
    pub fn active_provider_and_model(&self) -> (String, String) {
        // CLI providers first — no API key needed, just enabled check
        if let Some(c) = self.claude_cli.as_ref()
            && c.enabled
        {
            let model = c
                .default_model
                .clone()
                .unwrap_or_else(|| "sonnet".to_string());
            return ("claude-cli".to_string(), model);
        }
        if let Some(c) = self.opencode_cli.as_ref()
            && c.enabled
        {
            let model = c
                .default_model
                .clone()
                .unwrap_or_else(|| "opencode/gpt-5-nano".to_string());
            return ("opencode".to_string(), model);
        }
        if let Some(c) = self.qwen_code_cli.as_ref()
            && c.enabled
        {
            let model = c
                .default_model
                .clone()
                .unwrap_or_else(|| "qwen3-coder-plus".to_string());
            return ("qwen-cli".to_string(), model);
        }
        let candidates: &[(&str, Option<&ProviderConfig>)] = &[
            ("qwen", self.qwen.as_ref()),
            ("minimax", self.minimax.as_ref()),
            ("zhipu", self.zhipu.as_ref()),
            ("openrouter", self.openrouter.as_ref()),
            ("anthropic", self.anthropic.as_ref()),
            ("openai", self.openai.as_ref()),
            ("github", self.github.as_ref()),
            ("gemini", self.gemini.as_ref()),
        ];
        for &(name, cfg) in candidates {
            if let Some(c) = cfg
                && c.enabled
                && c.api_key.is_some()
            {
                let model = c
                    .default_model
                    .clone()
                    .unwrap_or_else(|| "(default)".to_string());
                return (name.to_string(), model);
            }
        }
        // Check custom providers
        if let Some((name, cfg)) = self.active_custom() {
            let model = cfg
                .default_model
                .clone()
                .unwrap_or_else(|| "(default)".to_string());
            return (format!("custom:{}", name), model);
        }
        ("none".to_string(), "none".to_string())
    }
}

/// Custom deserializer that handles both old flat format `[providers.custom]`
/// and new named map format `[providers.custom.<name>]`.
fn deserialize_custom_providers<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<BTreeMap<String, ProviderConfig>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    let value: Option<toml::Value> = Option::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };

    // Check if there are nested tables (named providers like [providers.custom.nvidia])
    // alongside top-level keys (flat format like [providers.custom] with enabled/api_key).
    // If both exist, extract the flat keys as "default" and parse named tables separately.
    let table = match value.as_table() {
        Some(t) => t,
        None => return Ok(None),
    };

    let flat_keys = ["enabled", "api_key", "base_url", "default_model", "models"];
    let has_flat = flat_keys.iter().any(|k| table.contains_key(*k));
    let has_named = table.values().any(|v| v.is_table());

    if has_flat && has_named {
        // Mixed: flat "default" provider + named providers in same section
        let mut map = BTreeMap::new();
        let mut flat_table = toml::map::Map::new();
        for key in &flat_keys {
            if let Some(v) = table.get(*key) {
                flat_table.insert(key.to_string(), v.clone());
            }
        }
        let default_cfg: ProviderConfig = toml::Value::Table(flat_table)
            .try_into()
            .map_err(de::Error::custom)?;
        map.insert("default".to_string(), default_cfg);
        for (name, val) in table {
            if flat_keys.contains(&name.as_str()) {
                continue;
            }
            if val.is_table() {
                let cfg: ProviderConfig = val.clone().try_into().map_err(de::Error::custom)?;
                map.insert(normalize_toml_key(name), cfg);
            }
        }
        Ok(Some(map))
    } else if has_flat {
        // Pure flat format — wrap as "default"
        let config: ProviderConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(de::Error::custom)?;
        let mut map = BTreeMap::new();
        map.insert("default".to_string(), config);
        Ok(Some(map))
    } else {
        // Pure named map format — normalize keys on load
        let raw: BTreeMap<String, ProviderConfig> = toml::Value::Table(table.clone())
            .try_into()
            .map_err(de::Error::custom)?;
        let map: BTreeMap<String, ProviderConfig> = raw
            .into_iter()
            .map(|(k, v)| (normalize_toml_key(&k), v))
            .collect();
        Ok(if map.is_empty() { None } else { Some(map) })
    }
}

/// Fallback provider configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FallbackProviderConfig {
    /// Enable fallback
    #[serde(default)]
    pub enabled: bool,

    /// Legacy: single fallback provider type (backwards compat)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Ordered list of fallback provider names — tried in sequence on failure.
    /// Each name must match a configured provider (e.g. "anthropic", "openrouter").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
}

/// STT (Speech-to-Text) provider configurations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SttProviders {
    /// Groq STT configuration ([providers.stt.groq])
    #[serde(default)]
    pub groq: Option<ProviderConfig>,

    /// Local whisper.cpp STT configuration ([providers.stt.local])
    #[serde(default)]
    pub local: Option<LocalSttConfig>,
}

/// Local STT (whisper.cpp) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSttConfig {
    /// Whether local STT is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Model preset (e.g. "local-tiny", "local-base", "local-small", "local-medium")
    #[serde(default = "default_local_stt_model")]
    pub model: String,
}

impl Default for LocalSttConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: default_local_stt_model(),
        }
    }
}

/// TTS (Text-to-Speech) provider configurations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TtsProviders {
    /// OpenAI TTS configuration ([providers.tts.openai])
    #[serde(default)]
    pub openai: Option<ProviderConfig>,

    /// Local Piper TTS configuration ([providers.tts.local])
    #[serde(default)]
    pub local: Option<LocalTtsConfig>,
}

/// Local TTS (Piper) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTtsConfig {
    /// Whether local TTS is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Piper voice name (default: "ryan")
    #[serde(default = "default_local_tts_voice")]
    pub voice: String,
}

impl Default for LocalTtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            voice: default_local_tts_voice(),
        }
    }
}

/// Web Search provider configurations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebSearchProviders {
    /// EXA search configuration
    #[serde(default)]
    pub exa: Option<ProviderConfig>,

    /// Brave search configuration
    #[serde(default)]
    pub brave: Option<ProviderConfig>,
}

/// Image provider configurations (e.g. Gemini for generation/vision)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageProviders {
    /// Google Gemini image configuration
    #[serde(default)]
    pub gemini: Option<ProviderConfig>,
}

/// Individual provider configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    /// Provider enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// API key (will be loaded from env or secrets)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// API base URL override
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Default model to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,

    /// Available models for this provider (can be updated at runtime)
    #[serde(default)]
    pub models: Vec<String>,

    /// Vision-capable model to use when the default model doesn't support images.
    /// When set and images are present, the provider swaps to this model for that
    /// request only (e.g. `vision_model = "MiniMax-Text-01"` for MiniMax M2.7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision_model: Option<String>,

    /// Context window size in tokens for this provider's model.
    /// Used by auto-compaction to know when to summarize history.
    /// Essential for custom/local providers whose models aren't recognized by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,

    /// Endpoint type for providers with multiple API modes (e.g. zhipu: "api" or "coding")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_type: Option<String>,

    /// TTS voice name (e.g. "echo") — only used by TTS providers
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,

    /// TTS model override (e.g. "gpt-4o-mini-tts") — only used by TTS providers
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    // ── OAuth fields (currently used by [providers.qwen]) ────────────────
    /// OAuth refresh token. Only set for providers using a device-code flow
    /// where OpenCrabs handles refresh itself (currently: Qwen native).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// OAuth bearer token expiry as ms since UNIX epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry_date: Option<u64>,

    /// API host returned by the OAuth provider (e.g. `portal.qwen.ai`).
    /// No scheme, no path — used to derive the chat completions URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_url: Option<String>,

    /// Opt-in toggle for Qwen3 hybrid thinking mode.
    /// Qwen3 is a single model with a runtime switch — set `enable_thinking =
    /// true` in `[providers.qwen]` to ask the gateway to emit reasoning
    /// tokens. Unset / false keeps the model in non-thinking (fast) mode.
    /// Currently only honoured by the native Qwen provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,

    /// Display name / alias for this account (e.g. "Personal", "Work").
    /// Only used in `[[providers.qwen_accounts]]` entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to SQLite database file
    #[serde(default = "default_db_path")]
    pub path: PathBuf,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

fn default_db_path() -> PathBuf {
    opencrabs_home().join("opencrabs.db")
}

/// Expand leading `~` or `~/` in a path to the actual home directory.
fn expand_tilde(p: &Path) -> PathBuf {
    if let Ok(rest) = p.strip_prefix("~") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest)
    } else {
        p.to_path_buf()
    }
}

/// Canonical base directory for the active profile.
///
/// - Default profile: `~/.opencrabs/`
/// - Named profile: `~/.opencrabs/profiles/<name>/`
///
/// Selection priority: `set_active_profile()` > `OPENCRABS_PROFILE` env > default.
pub fn opencrabs_home() -> PathBuf {
    let p = super::profile::resolve_profile_home();
    if !p.exists()
        && let Err(e) = std::fs::create_dir_all(&p)
    {
        tracing::error!("Failed to create opencrabs home directory {p:?}: {e}");
    }
    p
}

/// Daily backup of a config file. One copy per day, keeps `max_days` days.
///
/// Names backups `file.YYYY-MM-DD.bak`. If today's backup already exists,
/// skips (avoids overwriting a clean daily snapshot with mid-day edits).
/// Prunes backups older than `max_days`. Silently ignores errors — backup
/// failure must never block a config write.
pub fn daily_backup(path: &Path, max_days: usize) {
    if !path.exists() {
        return;
    }
    let parent = match path.parent() {
        Some(p) => p,
        None => return,
    };
    let stem = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let today_backup = parent.join(format!("{stem}.{today}.bak"));

    // Skip if today's backup already exists (preserve the day's first snapshot)
    if today_backup.exists() {
        return;
    }

    // Create today's backup
    if let Err(e) = fs::copy(path, &today_backup) {
        tracing::warn!("Failed to back up {} before write: {e}", path.display());
        return;
    }
    tracing::debug!("Daily backup: {}", today_backup.display());

    // Prune old backups beyond max_days
    let prefix = format!("{stem}.");
    let suffix = ".bak";
    if let Ok(entries) = fs::read_dir(parent) {
        let mut backups: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with(&prefix) && name.ends_with(suffix) && name != stem {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        backups.sort();
        backups.reverse(); // newest first
        for old in backups.iter().skip(max_days) {
            let _ = fs::remove_file(parent.join(old));
            tracing::debug!("Pruned old backup: {old}");
        }
    }
}

/// Snapshot current config + keys as "last known good".
///
/// Called after a successful provider response proves the config works.
/// On config parse failure, `Config::load()` falls back to these files.
/// Silently ignores errors — must never block normal operation.
pub fn save_last_good_config() {
    let home = opencrabs_home();

    let config_path = home.join("config.toml");
    let keys_path_src = home.join("keys.toml");
    let config_good = home.join("config.last_good.toml");
    let keys_good = home.join("keys.last_good.toml");

    if config_path.exists()
        && let Err(e) = fs::copy(&config_path, &config_good)
    {
        tracing::debug!("Failed to save last-good config: {e}");
    }
    if keys_path_src.exists()
        && let Err(e) = fs::copy(&keys_path_src, &keys_good)
    {
        tracing::debug!("Failed to save last-good keys: {e}");
    }
}

/// Try loading config from last-known-good snapshot.
///
/// Returns None if no snapshot exists or if it also fails to parse.
pub fn load_last_good_config() -> Option<Config> {
    let home = opencrabs_home();
    let config_good = home.join("config.last_good.toml");

    if !config_good.exists() {
        return None;
    }

    tracing::warn!("Attempting recovery from last-known-good config");

    // Load base config from the good snapshot
    let mut config = match Config::load_from_path(&config_good) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Last-good config also failed: {e}");
            return None;
        }
    };

    // Try loading keys from good snapshot
    let keys_good = home.join("keys.last_good.toml");
    if keys_good.exists()
        && let Ok(content) = fs::read_to_string(&keys_good)
        && let Ok(keys) = toml::from_str::<KeysFile>(&content)
    {
        config.providers = merge_provider_keys(config.providers, keys.providers);
        config.channels = merge_channel_keys(config.channels, keys.channels);
    }

    tracing::warn!("Recovered config from last-known-good snapshot");
    Some(config)
}

/// Get path to keys.toml - separate file for sensitive API keys
pub fn keys_path() -> PathBuf {
    opencrabs_home().join("keys.toml")
}

/// Save API keys to keys.toml using merge (preserves existing keys).
/// Only writes non-empty api_key values; never deletes other providers' keys.
pub fn save_keys(keys: &ProviderConfigs) -> Result<()> {
    // Merge each provider key individually via write_secret_key (read-modify-write)
    let providers: &[(&str, Option<&ProviderConfig>)] = &[
        ("providers.anthropic", keys.anthropic.as_ref()),
        ("providers.openai", keys.openai.as_ref()),
        ("providers.openrouter", keys.openrouter.as_ref()),
        ("providers.minimax", keys.minimax.as_ref()),
        ("providers.gemini", keys.gemini.as_ref()),
    ];

    for (section, provider) in providers {
        if let Some(p) = provider
            && let Some(key) = &p.api_key
            && !key.is_empty()
        {
            write_secret_key(section, "api_key", key)?;
        }
    }

    // Handle custom providers (flat "default" and named)
    if let Some(customs) = &keys.custom {
        for (name, p) in customs {
            if let Some(key) = &p.api_key
                && !key.is_empty()
            {
                let section = if name == "default" {
                    "providers.custom".to_string()
                } else {
                    format!("providers.custom.{}", name)
                };
                write_secret_key(&section, "api_key", key)?;
            }
        }
    }

    tracing::info!("Saved API keys to: {:?}", keys_path());
    Ok(())
}

/// Write a single key-value pair into keys.toml at the given dotted section path.
///
/// Equivalent to `Config::write_key` but targets `~/.opencrabs/keys.toml`.
/// Use for persisting secrets (tokens, API keys) that must not go into config.toml.
///
/// Normalize a TOML section key: lowercase, replace dots/underscores/spaces
/// with hyphens, strip non-alphanumeric chars (except hyphen).
/// e.g. "Qwen_2.5_4B" → "qwen-2-5-4b", "My Provider" → "my-provider"
pub fn normalize_toml_key(key: &str) -> String {
    key.trim()
        .to_lowercase()
        .replace(['.', '_', ' '], "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// # Example
/// ```no_run
/// # fn main() -> anyhow::Result<()> {
/// use opencrabs::config::write_secret_key;
/// write_secret_key("channels.telegram", "token", "123456:ABC...")?;
/// // results in keys.toml: [channels.telegram] token = "123456:ABC..."
/// # Ok(())
/// # }
/// ```
pub fn write_secret_key(section: &str, key: &str, value: &str) -> Result<()> {
    // Sanitize: strip carriage returns, take only first token (reject pasted URLs/junk after key)
    let value = value.split(['\r', '\n']).next().unwrap_or("").trim();
    if value.is_empty() {
        return Ok(()); // Don't write empty values
    }

    // Hold lock for entire read-modify-write to prevent races
    let _guard = CONFIG_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let path = keys_path();

    let mut doc: toml::Value = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str(&content)?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    // Normalize custom provider names (e.g. "Qwen_2.5_4B" → "qwen-2-5-4b")
    let parts: Vec<String> = section
        .split('.')
        .enumerate()
        .map(|(i, p)| {
            if i >= 2 && section.starts_with("providers.custom") {
                normalize_toml_key(p)
            } else {
                p.to_string()
            }
        })
        .collect();
    let mut current = doc
        .as_table_mut()
        .context("keys.toml root is not a table")?;

    for part in &parts {
        if !current.contains_key(part.as_str()) {
            current.insert(part.clone(), toml::Value::Table(toml::map::Map::new()));
        }
        current = current
            .get_mut(part.as_str())
            .context("section not found after insert")?
            .as_table_mut()
            .with_context(|| format!("'{}' is not a table", part))?;
    }
    current.insert(key.to_string(), toml::Value::String(value.to_string()));

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    daily_backup(&path, 7);
    let toml_str = toml::to_string_pretty(&doc)?;
    fs::write(&path, toml_str)?;
    tracing::info!("Wrote secret key [{section}].{key}");
    Ok(())
}

/// Keys file structure (keys.toml) - contains sensitive keys and tokens
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeysFile {
    #[serde(default)]
    pub providers: ProviderConfigs,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub a2a: Option<KeysA2a>,
    #[serde(default)]
    pub image: Option<ImageKeys>,
}

/// Image keys section in keys.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageKeys {
    pub api_key: Option<String>,
}

/// A2A keys section in keys.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeysA2a {
    pub api_key: Option<String>,
}

/// Load API keys from keys.toml
/// This file should be chmod 600 for security
fn load_keys_from_file() -> Result<KeysFile> {
    let keys_path = keys_path();
    if !keys_path.exists() {
        return Ok(KeysFile::default());
    }

    tracing::debug!("Loading keys from: {:?}", keys_path);
    let content = std::fs::read_to_string(&keys_path)?;
    let keys: KeysFile = toml::from_str(&content)?;
    Ok(keys)
}

/// Merge API keys from keys.toml into existing provider configs
/// Keys from keys.toml override values in config.toml
fn merge_provider_keys(mut base: ProviderConfigs, keys: ProviderConfigs) -> ProviderConfigs {
    // Guard: never merge the sentinel placeholder that /models uses internally
    let is_real_key = |k: &str| !k.is_empty() && k != "__EXISTING_KEY__";

    // Merge each provider's api_key if present in keys
    if let Some(k) = keys.anthropic
        && let Some(key) = k.api_key
        && is_real_key(&key)
    {
        let entry = base.anthropic.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(k) = keys.openai
        && let Some(key) = k.api_key
        && is_real_key(&key)
    {
        let entry = base.openai.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(k) = keys.openrouter
        && let Some(key) = k.api_key
        && is_real_key(&key)
    {
        let entry = base.openrouter.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    tracing::debug!(
        "merge_provider_keys: minimax keys present={}, base present={}",
        keys.minimax.is_some(),
        base.minimax.is_some()
    );
    if let Some(k) = keys.minimax
        && let Some(key) = k.api_key
        && is_real_key(&key)
    {
        let entry = base.minimax.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(k) = keys.gemini
        && let Some(key) = k.api_key
        && is_real_key(&key)
    {
        let entry = base.gemini.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(k) = keys.github
        && let Some(key) = k.api_key
        && is_real_key(&key)
    {
        let entry = base.github.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    // Merge zhipu
    if let Some(k) = keys.zhipu
        && let Some(key) = k.api_key
        && is_real_key(&key)
    {
        let entry = base.zhipu.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    // Merge qwen native (OAuth) — copies api_key + refresh_token + expiry + resource_url.
    // Auto-enables and creates the entry if keys.toml has credentials but config.toml doesn't,
    // since the user explicitly authenticated through onboarding.
    if let Some(k) = keys.qwen {
        let has_credentials = k.api_key.as_ref().map(|s| is_real_key(s)).unwrap_or(false)
            && k.refresh_token.as_ref().is_some_and(|s| !s.is_empty());
        if has_credentials {
            let entry = base.qwen.get_or_insert_with(|| ProviderConfig {
                enabled: true,
                ..Default::default()
            });
            entry.api_key = k.api_key;
            entry.refresh_token = k.refresh_token;
            entry.expiry_date = k.expiry_date;
            if k.resource_url.is_some() {
                entry.resource_url = k.resource_url;
            }
            if entry.default_model.is_none() && k.default_model.is_some() {
                entry.default_model = k.default_model;
            }
        }
    }
    // Merge qwen rotation accounts: config.toml has metadata (name, expiry_date,
    // resource_url), keys.toml has secrets (api_key, refresh_token). Merge per-index.
    match (&mut base.qwen_accounts, keys.qwen_accounts) {
        (Some(base_accounts), Some(key_accounts)) if !key_accounts.is_empty() => {
            // Per-index merge: overlay secrets from keys.toml onto metadata from config.toml
            for (i, key_cfg) in key_accounts.into_iter().enumerate() {
                if i < base_accounts.len() {
                    // Merge secrets into existing config entry
                    if key_cfg.api_key.is_some() {
                        base_accounts[i].api_key = key_cfg.api_key;
                    }
                    if key_cfg.refresh_token.is_some() {
                        base_accounts[i].refresh_token = key_cfg.refresh_token;
                    }
                    // keys.toml may also have expiry_date/resource_url from pre-migration
                    if key_cfg.expiry_date.is_some() && base_accounts[i].expiry_date.is_none() {
                        base_accounts[i].expiry_date = key_cfg.expiry_date;
                    }
                    if key_cfg.resource_url.is_some() && base_accounts[i].resource_url.is_none() {
                        base_accounts[i].resource_url = key_cfg.resource_url;
                    }
                } else {
                    // Legacy: keys.toml has more entries than config.toml (pre-migration)
                    base_accounts.push(key_cfg);
                }
            }
        }
        (None, Some(key_accounts)) if !key_accounts.is_empty() => {
            // Legacy: config.toml has no accounts, keys.toml has all (pre-migration)
            base.qwen_accounts = Some(key_accounts);
        }
        _ => {}
    }
    if let Some(custom_keys) = keys.custom
        && let Some(ref mut base_customs) = base.custom
    {
        // Only merge keys for custom providers that already exist in config.toml
        // Never create new config entries from keys.toml alone
        for (name, key_cfg) in custom_keys {
            if let Some(key) = key_cfg.api_key
                && is_real_key(&key)
                && base_customs.contains_key(&name)
            {
                let entry = base_customs.entry(name).or_default();
                entry.api_key = Some(key);
            }
        }
    }
    // Also handle STT/TTS keys
    if let Some(stt) = keys.stt
        && let Some(groq) = stt.groq
        && let Some(key) = groq.api_key
    {
        let base_stt = base.stt.get_or_insert_with(SttProviders::default);
        let entry = base_stt.groq.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(tts) = keys.tts
        && let Some(openai) = tts.openai
        && let Some(key) = openai.api_key
    {
        let base_tts = base.tts.get_or_insert_with(TtsProviders::default);
        let entry = base_tts.openai.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(ws) = keys.web_search {
        let base_ws = base
            .web_search
            .get_or_insert_with(WebSearchProviders::default);
        if let Some(exa) = ws.exa
            && let Some(key) = exa.api_key
            && !key.is_empty()
        {
            let entry = base_ws.exa.get_or_insert_with(ProviderConfig::default);
            entry.api_key = Some(key);
        }
        if let Some(brave) = ws.brave
            && let Some(key) = brave.api_key
            && !key.is_empty()
        {
            let entry = base_ws.brave.get_or_insert_with(ProviderConfig::default);
            entry.api_key = Some(key);
        }
    }
    // Merge image provider keys (e.g. [providers.image.gemini])
    if let Some(img) = keys.image {
        let base_img = base.image.get_or_insert_with(ImageProviders::default);
        if let Some(gemini) = img.gemini
            && let Some(key) = gemini.api_key
            && !key.is_empty()
        {
            let entry = base_img.gemini.get_or_insert_with(ProviderConfig::default);
            entry.api_key = Some(key);
        }
    }
    base
}

/// Merge channel tokens from keys.toml into existing channels config
/// Tokens from keys.toml override values in config.toml
fn merge_channel_keys(mut base: ChannelsConfig, keys: ChannelsConfig) -> ChannelsConfig {
    // Telegram
    if let Some(ref token) = keys.telegram.token
        && !token.is_empty()
    {
        base.telegram.token = Some(token.clone());
    }

    // Discord
    if let Some(ref token) = keys.discord.token
        && !token.is_empty()
    {
        base.discord.token = Some(token.clone());
    }

    // Slack
    if let Some(ref token) = keys.slack.token
        && !token.is_empty()
    {
        base.slack.token = Some(token.clone());
    }
    if let Some(ref app_token) = keys.slack.app_token
        && !app_token.is_empty()
    {
        base.slack.app_token = Some(app_token.clone());
    }

    // WhatsApp uses QR-code pairing stored in session.db — no token to merge.

    // Trello (app_token = API Key, token = API Token)
    if let Some(ref app_token) = keys.trello.app_token
        && !app_token.is_empty()
    {
        base.trello.app_token = Some(app_token.clone());
    }
    if let Some(ref token) = keys.trello.token
        && !token.is_empty()
    {
        base.trello.token = Some(token.clone());
    }

    base
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log to file
    #[serde(default)]
    pub file: Option<PathBuf>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file: None,
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            crabrace: CrabraceConfig::default(),
            database: DatabaseConfig {
                path: default_db_path(),
            },
            logging: LoggingConfig {
                level: default_log_level(),
                file: None,
            },
            debug: DebugConfig::default(),
            providers: ProviderConfigs::default(),
            channels: ChannelsConfig::default(),
            agent: AgentConfig::default(),
            daemon: DaemonConfig::default(),
            a2a: A2aConfig::default(),
            image: ImageConfig::default(),
            cron: CronConfig::default(),
        }
    }
}

impl Config {
    /// Build a runtime `VoiceConfig` from `providers.stt` / `providers.tts`.
    pub fn voice_config(&self) -> VoiceConfig {
        let stt = self.providers.stt.as_ref();
        let tts = self.providers.tts.as_ref();

        // STT: enabled if groq or local is enabled
        let groq_enabled = stt.and_then(|s| s.groq.as_ref()).is_some_and(|g| g.enabled);
        let local_stt_enabled = stt
            .and_then(|s| s.local.as_ref())
            .is_some_and(|l| l.enabled);
        let stt_enabled = groq_enabled || local_stt_enabled;
        let stt_mode = if local_stt_enabled {
            SttMode::Local
        } else {
            SttMode::Api
        };
        let local_stt_model = stt
            .and_then(|s| s.local.as_ref())
            .map(|l| l.model.clone())
            .unwrap_or_else(default_local_stt_model);

        // TTS: enabled if openai or local is enabled
        let openai_tts_enabled = tts
            .and_then(|t| t.openai.as_ref())
            .is_some_and(|o| o.enabled);
        let local_tts_enabled = tts
            .and_then(|t| t.local.as_ref())
            .is_some_and(|l| l.enabled);
        let tts_enabled = openai_tts_enabled || local_tts_enabled;
        let tts_mode = if local_tts_enabled {
            TtsMode::Local
        } else {
            TtsMode::Api
        };
        let tts_voice = tts
            .and_then(|t| t.openai.as_ref())
            .and_then(|o| o.voice.clone())
            .unwrap_or_else(default_tts_voice);
        let tts_model = tts
            .and_then(|t| t.openai.as_ref())
            .and_then(|o| o.model.clone().or_else(|| o.default_model.clone()))
            .unwrap_or_else(default_tts_model);
        let local_tts_voice = tts
            .and_then(|t| t.local.as_ref())
            .map(|l| l.voice.clone())
            .unwrap_or_else(default_local_tts_voice);

        let stt_provider = stt.and_then(|s| s.groq.clone());
        let tts_provider = tts.and_then(|t| t.openai.clone());

        VoiceConfig {
            stt_enabled,
            stt_mode,
            local_stt_model,
            tts_enabled,
            tts_mode,
            tts_voice,
            tts_model,
            local_tts_voice,
            stt_provider,
            tts_provider,
        }
    }

    /// Load configuration from default locations
    ///
    /// Priority (lowest to highest):
    /// 1. Default values
    /// 2. System config: ~/.opencrabs/config.toml
    /// 3. Local config: ./opencrabs.toml
    /// 4. Environment variables
    pub fn load() -> Result<Self> {
        match Self::load_inner() {
            Ok(config) => Ok(config),
            Err(e) => {
                tracing::error!("Config load failed: {e} — trying last-known-good");
                if let Some(good) = load_last_good_config() {
                    CONFIG_RECOVERED.store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok(good)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Returns true (once) if the last `Config::load()` fell back to a last-known-good snapshot.
    pub fn was_recovered() -> bool {
        CONFIG_RECOVERED.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    /// Inner load implementation — separated so `load()` can wrap with recovery.
    fn load_inner() -> Result<Self> {
        tracing::debug!("Loading configuration...");

        // Start with defaults
        let mut config = Self::default();

        // 1. Try to load system config
        if let Some(system_config_path) = Self::system_config_path()
            && system_config_path.exists()
        {
            tracing::debug!("Loading system config from: {:?}", system_config_path);
            config = Self::merge_from_file(config, &system_config_path)?;
        }

        // 2. Try to load local config
        let local_config_path = Self::local_config_path();
        if local_config_path.exists() {
            tracing::debug!("Loading local config from: {:?}", local_config_path);
            config = Self::merge_from_file(config, &local_config_path)?;
        }

        // 2.5 Migrate old config keys if needed (e.g. trello.allowed_channels → board_ids)
        if let Some(ref path) = Self::system_config_path() {
            Self::migrate_if_needed(path);
        }

        // 3. Load API keys from keys.toml (overrides config.toml keys)
        //    On parse failure, try keys.last_good.toml before giving up.
        match load_keys_from_file() {
            Err(e) => {
                tracing::error!("Failed to load keys.toml: {:#}", e);

                // Try recovering from last-good keys snapshot
                let keys_good = opencrabs_home().join("keys.last_good.toml");
                if keys_good.exists() {
                    match fs::read_to_string(&keys_good)
                        .context("reading keys.last_good.toml")
                        .and_then(|content| {
                            toml::from_str::<KeysFile>(&content)
                                .context("parsing keys.last_good.toml")
                        }) {
                        Ok(keys) => {
                            tracing::warn!(
                                "Recovered API keys from keys.last_good.toml — \
                                 fix or delete keys.toml to clear this warning"
                            );
                            config.providers =
                                merge_provider_keys(config.providers, keys.providers);
                            config.channels = merge_channel_keys(config.channels, keys.channels);
                            if let Some(a2a_keys) = keys.a2a
                                && let Some(key) = a2a_keys.api_key
                                && !key.is_empty()
                            {
                                config.a2a.api_key = Some(key);
                            }
                        }
                        Err(e2) => {
                            tracing::error!(
                                "keys.last_good.toml also failed: {:#} — no API keys loaded",
                                e2
                            );
                        }
                    }
                } else {
                    tracing::error!("No keys.last_good.toml backup — no API keys loaded");
                }
            }
            Ok(keys) => {
                config.providers = merge_provider_keys(config.providers, keys.providers);
                config.channels = merge_channel_keys(config.channels, keys.channels);
                // Merge A2A API key from keys.toml
                if let Some(a2a_keys) = keys.a2a
                    && let Some(key) = a2a_keys.api_key
                    && !key.is_empty()
                {
                    config.a2a.api_key = Some(key);
                }
                // Merge image API key into config.image (generation + vision)
                // New path: [providers.image.gemini] (already merged above)
                // Legacy fallback: flat [image] section in keys.toml
                let image_key = config
                    .providers
                    .image
                    .as_ref()
                    .and_then(|img| img.gemini.as_ref())
                    .and_then(|g| g.api_key.as_ref())
                    .filter(|k| !k.is_empty())
                    .cloned()
                    .or_else(|| {
                        keys.image
                            .and_then(|img| img.api_key)
                            .filter(|k| !k.is_empty())
                    });
                if let Some(key) = image_key {
                    config.image.generation.api_key = Some(key.clone());
                    config.image.vision.api_key = Some(key);
                }
            }
        }

        // 4. Apply environment variable overrides
        config = Self::apply_env_overrides(config)?;

        // Expand tilde in database path (TOML doesn't expand ~)
        config.database.path = expand_tilde(&config.database.path);

        // Warn about unknown top-level keys in config.toml
        if let Some(path) = Self::system_config_path()
            && path.exists()
        {
            Self::warn_unknown_keys(&path);
        }

        tracing::debug!("Configuration loaded successfully");
        Ok(config)
    }

    /// Known top-level sections in config.toml.
    const KNOWN_TOP_LEVEL_KEYS: &[&str] = &[
        "crabrace",
        "database",
        "logging",
        "debug",
        "providers",
        "channels",
        "agent",
        "daemon",
        "a2a",
        "gateway",
        "image",
        "cron",
    ];

    /// Check for unknown top-level keys and log warnings.
    /// Only collects warnings once — subsequent calls are no-ops.
    fn warn_unknown_keys(path: &Path) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static CHECKED: AtomicBool = AtomicBool::new(false);
        if CHECKED.swap(true, Ordering::Relaxed) {
            return;
        }

        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(table) = raw.parse::<toml::Table>() else {
            return;
        };
        let mut unknown: Vec<String> = Vec::new();
        for key in table.keys() {
            if !Self::KNOWN_TOP_LEVEL_KEYS.contains(&key.as_str()) {
                unknown.push(key.clone());
            }
        }
        if !unknown.is_empty() {
            tracing::warn!(
                "Unknown top-level keys in config.toml (possible typos): {}",
                unknown.join(", ")
            );
            CONFIG_TYPO_WARNINGS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(unknown);
        }
    }

    /// Returns any config typo warnings collected during load (drains the list).
    pub fn take_typo_warnings() -> Vec<String> {
        CONFIG_TYPO_WARNINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// Load configuration from a specific file path
    ///
    /// Priority (lowest to highest):
    /// 1. Default values
    /// 2. Custom config file (specified path)
    /// 3. Environment variables
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        tracing::debug!("Loading configuration from custom path: {:?}", path);

        // Start with defaults
        let mut config = Self::default();

        // Load from custom path
        if path.exists() {
            config = Self::merge_from_file(config, path)?;
        } else {
            anyhow::bail!("Config file not found: {:?}", path);
        }

        // Apply environment variable overrides
        config = Self::apply_env_overrides(config)?;

        // Expand tilde in database path (TOML doesn't expand ~)
        config.database.path = expand_tilde(&config.database.path);

        tracing::debug!("Configuration loaded successfully from custom path");
        Ok(config)
    }

    /// Migrate old config keys in-place.
    ///
    /// Currently handles: `channels.trello.allowed_channels` → `board_ids`.
    /// Called once after loading so old configs are silently upgraded on first run.
    fn migrate_if_needed(path: &Path) {
        let Ok(content) = fs::read_to_string(path) else {
            return;
        };

        let mut doc: toml::Value = match toml::from_str(&content) {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut changed = false;

        // ── Migration 1: channels.trello.allowed_channels → board_ids ──
        if let Some(trello) = doc
            .get_mut("channels")
            .and_then(|c| c.get_mut("trello"))
            .and_then(|t| t.as_table_mut())
            && let Some(val) = trello.remove("allowed_channels")
            && !trello.contains_key("board_ids")
        {
            trello.insert("board_ids".to_string(), val);
            changed = true;
        }

        // ── Migration 2: [voice] → providers.stt.* / providers.tts.* ──
        if let Some(voice) = doc.get("voice").and_then(|v| v.as_table()).cloned() {
            let root = doc.as_table_mut().unwrap();

            // Ensure providers.stt and providers.tts tables exist
            if !root.contains_key("providers") {
                root.insert(
                    "providers".to_string(),
                    toml::Value::Table(toml::map::Map::new()),
                );
            }
            let providers = root.get_mut("providers").unwrap().as_table_mut().unwrap();
            if !providers.contains_key("stt") {
                providers.insert("stt".to_string(), toml::Value::Table(toml::map::Map::new()));
            }
            if !providers.contains_key("tts") {
                providers.insert("tts".to_string(), toml::Value::Table(toml::map::Map::new()));
            }
            // STT: stt_enabled + stt_mode → providers.stt.groq / providers.stt.local
            let stt_enabled = voice
                .get("stt_enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let stt_mode = voice
                .get("stt_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("api")
                .to_string();
            if stt_enabled {
                let stt = providers.get_mut("stt").unwrap().as_table_mut().unwrap();
                if stt_mode == "local" {
                    if !stt.contains_key("local") {
                        stt.insert(
                            "local".to_string(),
                            toml::Value::Table(toml::map::Map::new()),
                        );
                    }
                    let local = stt.get_mut("local").unwrap().as_table_mut().unwrap();
                    local.entry("enabled").or_insert(toml::Value::Boolean(true));
                    if let Some(model) = voice.get("local_stt_model") {
                        local.entry("model").or_insert(model.clone());
                    }
                } else if let Some(groq) = stt.get_mut("groq").and_then(|g| g.as_table_mut()) {
                    groq.entry("enabled").or_insert(toml::Value::Boolean(true));
                }
            }

            // TTS: tts_enabled + tts_mode → providers.tts.openai / providers.tts.local
            let tts_enabled = voice
                .get("tts_enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let tts_mode = voice
                .get("tts_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("api")
                .to_string();
            if tts_enabled {
                let tts = providers.get_mut("tts").unwrap().as_table_mut().unwrap();
                if tts_mode == "local" {
                    if !tts.contains_key("local") {
                        tts.insert(
                            "local".to_string(),
                            toml::Value::Table(toml::map::Map::new()),
                        );
                    }
                    let local = tts.get_mut("local").unwrap().as_table_mut().unwrap();
                    local.entry("enabled").or_insert(toml::Value::Boolean(true));
                    if let Some(voice_name) = voice.get("local_tts_voice") {
                        local.entry("voice").or_insert(voice_name.clone());
                    }
                } else if let Some(openai) = tts.get_mut("openai").and_then(|o| o.as_table_mut()) {
                    openai
                        .entry("enabled")
                        .or_insert(toml::Value::Boolean(true));
                    if let Some(v) = voice.get("tts_voice") {
                        openai.entry("voice").or_insert(v.clone());
                    }
                    if let Some(m) = voice.get("tts_model") {
                        openai.entry("model").or_insert(m.clone());
                    }
                }
            }

            // Remove the old [voice] section
            root.remove("voice");
            changed = true;
        }

        if !changed {
            // Migration 3: inject commented subagent defaults into [agent] section
            // Uses text-level injection (comments can't survive toml::Value round-trip)
            let content = fs::read_to_string(path).unwrap_or_default();
            let has_subagent =
                content.contains("subagent_provider") || content.contains("subagent_model");
            if !has_subagent && let Ok(injected) = inject_subagent_defaults(&content) {
                match fs::write(path, &injected) {
                    Ok(()) => {
                        tracing::info!("Config migrated: injected subagent defaults into [agent]")
                    }
                    Err(e) => tracing::warn!(
                        "Config migration: failed to write injected defaults to {}: {e}",
                        path.display()
                    ),
                }
                return;
            }
        }

        // Migration 3: also inject subagent defaults when other migrations occurred
        let final_content = toml::to_string_pretty(&doc).unwrap_or_default();
        let has_subagent =
            final_content.contains("subagent_provider") || final_content.contains("subagent_model");
        if !has_subagent && let Ok(injected) = inject_subagent_defaults(&final_content) {
            Self::backup_config(path, 7);
            match fs::write(path, &injected) {
                Ok(()) => tracing::info!(
                    "Config migrated: [voice] → providers.stt/tts + subagent defaults"
                ),
                Err(e) => tracing::warn!(
                    "Config migration: failed to write injected defaults to {}: {e}",
                    path.display()
                ),
            }
            return;
        }

        Self::backup_config(path, 7);
        if let Ok(toml_str) = toml::to_string_pretty(&doc)
            && fs::write(path, toml_str).is_ok()
        {
            tracing::info!("Config migrated: [voice] → providers.stt/tts");
        }
    }

    /// Get the system config path: ~/.opencrabs/config.toml
    pub fn system_config_path() -> Option<PathBuf> {
        Some(opencrabs_home().join("config.toml"))
    }

    /// Get the local config path: ./opencrabs.toml
    fn local_config_path() -> PathBuf {
        PathBuf::from("./opencrabs.toml")
    }

    /// Load and merge configuration from a TOML file
    fn merge_from_file(base: Self, path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;

        let file_config: Self = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {:?}", path))?;

        Ok(Self::merge(base, file_config))
    }

    /// Merge two configs (file_config overwrites base where specified)
    fn merge(_base: Self, overlay: Self) -> Self {
        // For now, we'll do a simple overlay merge where overlay completely replaces base
        // In the future, we could make this more sophisticated with field-level merging
        Self {
            crabrace: overlay.crabrace,
            database: overlay.database,
            logging: overlay.logging,
            debug: overlay.debug,
            providers: overlay.providers,
            channels: overlay.channels,
            agent: overlay.agent,
            daemon: overlay.daemon,
            a2a: overlay.a2a,
            image: overlay.image,
            cron: overlay.cron,
        }
    }

    /// Apply environment variable overrides
    fn apply_env_overrides(mut config: Self) -> Result<Self> {
        // Database path
        if let Ok(db_path) = std::env::var("OPENCRABS_DB_PATH") {
            config.database.path = PathBuf::from(db_path);
        }

        // Log level
        if let Ok(log_level) = std::env::var("OPENCRABS_LOG_LEVEL") {
            config.logging.level = log_level;
        }

        // Log file
        if let Ok(log_file) = std::env::var("OPENCRABS_LOG_FILE") {
            config.logging.file = Some(PathBuf::from(log_file));
        }

        // Debug options
        if let Ok(debug_lsp) = std::env::var("OPENCRABS_DEBUG_LSP") {
            config.debug.debug_lsp = debug_lsp.parse().unwrap_or(false);
        }

        if let Ok(profiling) = std::env::var("OPENCRABS_PROFILING") {
            config.debug.profiling = profiling.parse().unwrap_or(false);
        }

        // Crabrace options
        if let Ok(enabled) = std::env::var("OPENCRABS_CRABRACE_ENABLED") {
            config.crabrace.enabled = enabled.parse().unwrap_or(true);
        }

        if let Ok(base_url) = std::env::var("OPENCRABS_CRABRACE_URL") {
            config.crabrace.base_url = base_url;
        }

        if let Ok(auto_update) = std::env::var("OPENCRABS_CRABRACE_AUTO_UPDATE") {
            config.crabrace.auto_update = auto_update.parse().unwrap_or(true);
        }

        Ok(config)
    }

    /// Reload configuration from disk (re-runs `Config::load()`).
    pub fn reload() -> Result<Self> {
        tracing::info!("Reloading configuration from disk");
        Self::load()
    }

    /// Write a key-value pair into the system config.toml using TOML merge.
    ///
    /// `section` is a dotted path like "agent" or "providers.tts.local".
    /// `key` is the field name inside that section.
    /// `value` is the TOML-serialisable value.
    pub fn write_key(section: &str, key: &str, value: &str) -> Result<()> {
        // Sanitize: trim whitespace/newlines that may leak from TUI input
        let value = value.trim();

        // Hold lock for entire read-modify-write to prevent races between
        // concurrent write_key calls (e.g. fallback provider switching fires
        // multiple writes in rapid succession).
        let _guard = CONFIG_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let path =
            Self::system_config_path().unwrap_or_else(|| opencrabs_home().join("config.toml"));

        // Read existing TOML or start fresh
        let mut doc: toml::Value = if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content)?
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        // Navigate/create the section table (supports dotted paths like "channels.slack")
        // Normalize custom provider names (e.g. "Qwen_2.5_4B" → "qwen-2-5-4b")
        let parts: Vec<String> = section
            .split('.')
            .enumerate()
            .map(|(i, p)| {
                // providers.custom.<name> — normalize the <name> part
                if i >= 2 && section.starts_with("providers.custom") {
                    normalize_toml_key(p)
                } else {
                    p.to_string()
                }
            })
            .collect();
        let mut current = doc.as_table_mut().context("config root is not a table")?;

        for part in &parts {
            if !current.contains_key(part.as_str()) {
                current.insert(part.clone(), toml::Value::Table(toml::map::Map::new()));
            }
            current = current
                .get_mut(part.as_str())
                .context("section not found after insert")?
                .as_table_mut()
                .with_context(|| format!("'{}' is not a table", part))?;
        }
        let section_table = current;

        // Parse the value — try JSON array, integer, float, bool, then fall back to string
        let parsed: toml::Value = if value.starts_with('[') && value.ends_with(']') {
            // Try parsing as JSON array → TOML array
            if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(value) {
                let toml_arr: Vec<toml::Value> = arr
                    .into_iter()
                    .filter_map(|v| match v {
                        serde_json::Value::String(s) => Some(toml::Value::String(s)),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                Some(toml::Value::Integer(i))
                            } else {
                                n.as_f64().map(toml::Value::Float)
                            }
                        }
                        serde_json::Value::Bool(b) => Some(toml::Value::Boolean(b)),
                        _ => None,
                    })
                    .collect();
                toml::Value::Array(toml_arr)
            } else {
                toml::Value::String(value.to_string())
            }
        } else if let Ok(v) = value.parse::<i64>() {
            toml::Value::Integer(v)
        } else if let Ok(v) = value.parse::<f64>() {
            toml::Value::Float(v)
        } else if let Ok(v) = value.parse::<bool>() {
            toml::Value::Boolean(v)
        } else {
            toml::Value::String(value.to_string())
        };

        section_table.insert(key.to_string(), parsed);

        // Write back
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Back up before overwriting
        Self::backup_config(&path, 7);

        let toml_str = toml::to_string_pretty(&doc)?;
        fs::write(&path, toml_str)?;
        tracing::info!("Wrote config key [{section}].{key}");
        Ok(())
    }

    /// Remove a dotted section from config.toml.
    /// e.g. `remove_section("providers.custom.default")` removes `[providers.custom.default]`.
    pub fn remove_section(section: &str) -> Result<()> {
        let _guard = CONFIG_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let path =
            Self::system_config_path().unwrap_or_else(|| opencrabs_home().join("config.toml"));
        if !path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&path)?;
        let mut doc: toml::Value =
            toml::from_str(&content)?;

        let parts: Vec<&str> = section.split('.').collect();
        if parts.is_empty() {
            return Ok(());
        }

        // Navigate to the parent table and remove the last key
        let parent_parts = &parts[..parts.len() - 1];
        let leaf = parts[parts.len() - 1];

        let mut current = doc.as_table_mut().context("config root is not a table")?;
        for part in parent_parts {
            match current.get_mut(*part) {
                Some(v) if v.is_table() => {
                    current = v.as_table_mut().unwrap();
                }
                _ => return Ok(()), // parent doesn't exist, nothing to remove
            }
        }

        current.remove(leaf);
        tracing::info!("Removed config section [{section}]");

        Self::backup_config(&path, 7);
        let toml_str = toml::to_string_pretty(&doc)?;
        fs::write(&path, toml_str)?;
        Ok(())
    }

    /// Clean up custom provider entries that have no base_url and no default_model.
    /// These are ghost entries created when disabling all providers on save.
    pub fn cleanup_empty_custom_providers() {
        // Clean config.toml
        if let Ok(config) = Self::load()
            && let Some(customs) = &config.providers.custom
        {
            for (name, cfg) in customs {
                let is_empty_name = name.is_empty();
                let has_url = cfg.base_url.as_ref().is_some_and(|u| !u.is_empty());
                let has_model = cfg.default_model.as_ref().is_some_and(|m| !m.is_empty());
                if is_empty_name || (!has_url && !has_model) {
                    let section = format!("providers.custom.{}", name);
                    if let Err(e) = Self::remove_section(&section) {
                        tracing::warn!("Failed to remove empty custom provider '{}': {}", name, e);
                    }
                }
            }
        }

        // Clean keys.toml — remove empty-key entries and entries with no
        // matching config (ghost keys left by normalization mismatches).
        Self::cleanup_keys_custom_providers();
    }

    /// Remove ghost custom provider entries from keys.toml.
    fn cleanup_keys_custom_providers() {
        let keys_file = keys_path();
        if !keys_file.exists() {
            return;
        }
        let Ok(content) = std::fs::read_to_string(&keys_file) else {
            return;
        };
        let Ok(mut doc) = content.parse::<toml::Value>() else {
            return;
        };

        // Navigate to providers.custom table
        let custom_table = match doc
            .as_table_mut()
            .and_then(|t| t.get_mut("providers"))
            .and_then(|t| t.as_table_mut())
            .and_then(|t| t.get_mut("custom"))
            .and_then(|t| t.as_table_mut())
        {
            Some(t) => t,
            None => return,
        };

        // Collect keys to remove: empty names or entries with no config counterpart
        let config_names: std::collections::HashSet<String> = Self::load()
            .ok()
            .and_then(|c| c.providers.custom.map(|m| m.keys().cloned().collect()))
            .unwrap_or_default();

        let remove: Vec<String> = custom_table
            .keys()
            .filter(|k| k.is_empty() || !config_names.contains(*k))
            .cloned()
            .collect();

        if remove.is_empty() {
            return;
        }

        for key in &remove {
            custom_table.remove(key);
            tracing::info!("Removed ghost key from keys.toml: providers.custom.{}", key);
        }

        daily_backup(&keys_file, 7);
        if let Ok(toml_str) = toml::to_string_pretty(&doc)
            && let Err(e) = std::fs::write(&keys_file, toml_str)
        {
            tracing::warn!("Failed to clean ghost keys from keys.toml: {e}");
        }
    }

    /// Write a string array to a dotted config section.
    /// e.g. `write_array("channels.slack", "allowed_users", &["U123"])` →
    /// `[channels.slack] allowed_users = ["U123"]`
    pub fn write_array(section: &str, key: &str, values: &[String]) -> Result<()> {
        let path =
            Self::system_config_path().unwrap_or_else(|| opencrabs_home().join("config.toml"));

        let mut doc: toml::Value = if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content)?
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        // Navigate/create nested section
        let parts: Vec<&str> = section.split('.').collect();
        let mut current = doc.as_table_mut().context("config root is not a table")?;

        for part in &parts {
            if !current.contains_key(*part) {
                current.insert(part.to_string(), toml::Value::Table(toml::map::Map::new()));
            }
            current = current
                .get_mut(*part)
                .context("section not found after insert")?
                .as_table_mut()
                .with_context(|| format!("'{}' is not a table", part))?;
        }

        let arr = values
            .iter()
            .map(|v| toml::Value::String(v.clone()))
            .collect();
        current.insert(key.to_string(), toml::Value::Array(arr));

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Self::backup_config(&path, 7);
        let toml_str = toml::to_string_pretty(&doc)?;
        fs::write(&path, toml_str)?;
        tracing::info!(
            "Wrote config array [{section}].{key} ({} items)",
            values.len()
        );
        Ok(())
    }

    /// Validate configuration
    /// Check if any provider has an API key configured (from config).
    pub fn has_any_api_key(&self) -> bool {
        let has_anthropic = self
            .providers
            .anthropic
            .as_ref()
            .is_some_and(|p| p.api_key.is_some());
        let has_openai = self
            .providers
            .openai
            .as_ref()
            .is_some_and(|p| p.api_key.is_some());
        let has_gemini = self
            .providers
            .gemini
            .as_ref()
            .is_some_and(|p| p.api_key.is_some());

        has_anthropic || has_openai || has_gemini
    }

    pub fn validate(&self) -> Result<()> {
        tracing::debug!("Validating configuration...");

        // Validate database path parent directory exists
        if let Some(parent) = self.database.path.parent()
            && !parent.exists()
        {
            tracing::warn!(
                "Database parent directory does not exist, will be created: {:?}",
                parent
            );
        }

        // Validate log level
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.logging.level.as_str()) {
            anyhow::bail!(
                "Invalid log level: {}. Must be one of: {:?}",
                self.logging.level,
                valid_levels
            );
        }

        // Validate Crabrace URL if enabled
        if self.crabrace.enabled && self.crabrace.base_url.is_empty() {
            anyhow::bail!("Crabrace is enabled but base_url is empty");
        }

        tracing::debug!("Configuration validation passed");
        Ok(())
    }

    /// Daily backup before writing. Delegates to the standalone function.
    fn backup_config(path: &Path, max_days: usize) {
        daily_backup(path, max_days);
    }

    /// Save configuration to a file
    pub fn save(&self, path: &Path) -> Result<()> {
        let _guard = CONFIG_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let toml_string =
            toml::to_string_pretty(self).context("Failed to serialize config to TOML")?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {:?}", parent))?;
        }

        // Back up before overwriting
        Self::backup_config(path, 7);

        fs::write(path, toml_string)
            .with_context(|| format!("Failed to write config file: {:?}", path))?;

        tracing::info!("Configuration saved to: {:?}", path);
        Ok(())
    }
}

/// Inject commented-out subagent provider/model defaults into the [agent] section.
///
/// Used by migration so users can discover the feature by uncommenting lines in config.toml.
/// Pure text-level operation — preserves all existing formatting and comments.
fn inject_subagent_defaults(content: &str) -> Result<String> {
    let comment_block = "\n# Sub-agent routing — override the parent session's provider for\n# spawned agents, team members, and background workers.\n# subagent_provider = \"anthropic\"    # e.g. openrouter, minimax, custom:ollama\n# subagent_model = \"claude-sonnet-4-6\"  # only used when subagent_provider is set\n";

    // Find [agent] section boundary
    let marker = "[agent]";
    let agent_pos = content
        .find(marker)
        .ok_or_else(|| anyhow::anyhow!("No [agent] section found"))?;

    let section_start = agent_pos + marker.len();

    // Find end of [agent] section: next section header or EOF
    let rest = &content[section_start..];
    let next_section = rest.find("\n[");
    let section_end = if let Some(pos) = next_section {
        section_start + pos
    } else {
        content.len()
    };

    // Insert the comment block at the end of the section
    let mut out = String::with_capacity(content.len() + comment_block.len());
    out.push_str(&content[..section_end]);
    let before = &content[..section_end];
    let ends_with_blank = before.ends_with("\n\n");
    if !ends_with_blank {
        out.push('\n');
    }
    out.push_str(comment_block);
    out.push_str(&content[section_end..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.crabrace.enabled);
        assert_eq!(config.logging.level, "info");
        assert!(!config.debug.debug_lsp);
        assert!(!config.debug.profiling);
    }

    #[test]
    fn test_config_validation() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validation_invalid_log_level() {
        let mut config = Config::default();
        config.logging.level = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_empty_crabrace_url() {
        let mut config = Config::default();
        config.crabrace.base_url = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_from_toml() {
        let toml_content = r#"
[database]
path = "/custom/path/db.sqlite"

[logging]
level = "debug"

[debug]
debug_lsp = true
profiling = true

[crabrace]
enabled = false
        "#;

        let config: Config = toml::from_str(toml_content).unwrap();
        assert_eq!(
            config.database.path,
            PathBuf::from("/custom/path/db.sqlite")
        );
        assert_eq!(config.logging.level, "debug");
        assert!(config.debug.debug_lsp);
        assert!(config.debug.profiling);
        assert!(!config.crabrace.enabled);
    }

    #[test]
    fn test_config_save_and_load() {
        let temp_file = NamedTempFile::new().unwrap();
        let config = Config::default();

        // Save config
        config.save(temp_file.path()).unwrap();

        // Load config back
        let contents = std::fs::read_to_string(temp_file.path()).unwrap();
        let loaded_config: Config = toml::from_str(&contents).unwrap();

        assert_eq!(loaded_config.logging.level, config.logging.level);
        assert_eq!(loaded_config.crabrace.enabled, config.crabrace.enabled);
    }

    #[test]
    fn test_config_from_toml_overrides() {
        let toml_content = r#"
[logging]
level = "trace"

[debug]
debug_lsp = true
profiling = true

[database]
path = "/tmp/test.db"
        "#;

        let config: Config = toml::from_str(toml_content).unwrap();
        assert_eq!(config.logging.level, "trace");
        assert!(config.debug.debug_lsp);
        assert!(config.debug.profiling);
        assert_eq!(config.database.path, PathBuf::from("/tmp/test.db"));
    }

    #[test]
    fn test_provider_config_from_toml() {
        let toml_content = r#"
[providers.anthropic]
enabled = true
api_key = "test-anthropic-key"
default_model = "claude-opus-4-6"

[providers.openai]
enabled = true
api_key = "test-openai-key"
        "#;

        let config: Config = toml::from_str(toml_content).unwrap();

        assert!(config.providers.anthropic.is_some());
        let anthropic = config.providers.anthropic.as_ref().unwrap();
        assert_eq!(anthropic.api_key, Some("test-anthropic-key".to_string()));
        assert_eq!(anthropic.default_model, Some("claude-opus-4-6".to_string()));

        assert!(config.providers.openai.is_some());
        assert_eq!(
            config.providers.openai.as_ref().unwrap().api_key,
            Some("test-openai-key".to_string())
        );
    }

    #[test]
    fn test_system_config_path() {
        let path = Config::system_config_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains("opencrabs"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn test_local_config_path() {
        let path = Config::local_config_path();
        assert_eq!(path, PathBuf::from("./opencrabs.toml"));
    }

    #[test]
    fn test_debug_config_default() {
        let debug = DebugConfig::default();
        assert!(!debug.debug_lsp);
        assert!(!debug.profiling);
    }

    #[test]
    fn test_provider_configs_default() {
        let providers = ProviderConfigs::default();
        assert!(providers.anthropic.is_none());
        assert!(providers.openai.is_none());
        assert!(providers.gemini.is_none());
        assert!(providers.bedrock.is_none());
        assert!(providers.vertex.is_none());
    }

    #[test]
    fn test_database_config_default() {
        let db_config = DatabaseConfig::default();
        assert!(!db_config.path.as_os_str().is_empty());
    }

    #[test]
    fn test_logging_config_default() {
        let logging = LoggingConfig::default();
        assert_eq!(logging.level, "info");
        assert!(logging.file.is_none());
    }

    #[test]
    fn test_agent_config_default() {
        let agent = AgentConfig::default();
        assert_eq!(agent.approval_policy, "auto-always");
        assert_eq!(agent.max_concurrent, 4);
    }

    #[test]
    fn test_agent_config_from_toml() {
        let toml_content = r#"
[agent]
approval_policy = "auto-always"
max_concurrent = 8
        "#;

        let config: Config = toml::from_str(toml_content).unwrap();
        assert_eq!(config.agent.approval_policy, "auto-always");
        assert_eq!(config.agent.max_concurrent, 8);
    }

    #[test]
    fn test_agent_config_defaults_when_absent() {
        // Config without [agent] section should use defaults
        let toml_content = r#"
[logging]
level = "info"
        "#;

        let config: Config = toml::from_str(toml_content).unwrap();
        assert_eq!(config.agent.approval_policy, "auto-always");
        assert_eq!(config.agent.max_concurrent, 4);
    }

    #[test]
    fn test_write_key_creates_and_updates() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");

        // Write initial content
        fs::write(&config_path, "[logging]\nlevel = \"info\"\n").unwrap();

        // Use write_key-style logic (can't call write_key directly since it
        // uses system_config_path, but we test the merge logic)
        let content = fs::read_to_string(&config_path).unwrap();
        let mut doc: toml::Value = toml::from_str(&content).unwrap();
        let table = doc.as_table_mut().unwrap();

        // Add a new section
        table.insert(
            "agent".to_string(),
            toml::Value::Table({
                let mut m = toml::map::Map::new();
                m.insert(
                    "approval_policy".to_string(),
                    toml::Value::String("auto-session".to_string()),
                );
                m
            }),
        );

        let output = toml::to_string_pretty(&doc).unwrap();
        fs::write(&config_path, &output).unwrap();

        // Verify it round-trips
        let content = fs::read_to_string(&config_path).unwrap();
        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(loaded.agent.approval_policy, "auto-session");
        assert_eq!(loaded.logging.level, "info");
    }

    #[test]
    fn test_config_save_with_agent_section() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut config = Config::default();
        config.agent.approval_policy = "auto-always".to_string();
        config.agent.max_concurrent = 2;

        config.save(temp_file.path()).unwrap();

        let contents = fs::read_to_string(temp_file.path()).unwrap();
        let loaded: Config = toml::from_str(&contents).unwrap();
        assert_eq!(loaded.agent.approval_policy, "auto-always");
        assert_eq!(loaded.agent.max_concurrent, 2);
    }
}

/// Resolve provider name and model from config (for display purposes)
#[allow(clippy::items_after_test_module)]
pub fn resolve_provider_from_config(config: &Config) -> (&str, &str) {
    // Check providers in priority order: Anthropic > OpenAI > GitHub > Gemini > OpenRouter > Minimax
    if config
        .providers
        .anthropic
        .as_ref()
        .is_some_and(|p| p.enabled)
    {
        let model = config
            .providers
            .anthropic
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("Anthropic", model);
    }
    if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
        let model = config
            .providers
            .openai
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("OpenAI", model);
    }
    if config.providers.github.as_ref().is_some_and(|p| p.enabled) {
        let model = config
            .providers
            .github
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("gpt-5-mini");
        return ("GitHub Copilot", model);
    }
    if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
        let model = config
            .providers
            .gemini
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("Google Gemini", model);
    }
    if config
        .providers
        .openrouter
        .as_ref()
        .is_some_and(|p| p.enabled)
    {
        let model = config
            .providers
            .openrouter
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("OpenRouter", model);
    }
    if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
        let model = config
            .providers
            .minimax
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("Minimax", model);
    }
    if config.providers.zhipu.as_ref().is_some_and(|p| p.enabled) {
        let model = config
            .providers
            .zhipu
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("z.ai GLM", model);
    }
    if config
        .providers
        .claude_cli
        .as_ref()
        .is_some_and(|p| p.enabled)
    {
        let model = config
            .providers
            .claude_cli
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("sonnet");
        return ("Claude CLI", model);
    }
    if config
        .providers
        .opencode_cli
        .as_ref()
        .is_some_and(|p| p.enabled)
    {
        let model = config
            .providers
            .opencode_cli
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("opencode/gpt-5-nano");
        return ("OpenCode CLI", model);
    }
    if config.providers.qwen.as_ref().is_some_and(|p| p.enabled) {
        let model = config
            .providers
            .qwen
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("qwen3.6-plus");
        return ("Qwen", model);
    }
    if config
        .providers
        .qwen_code_cli
        .as_ref()
        .is_some_and(|p| p.enabled)
    {
        let model = config
            .providers
            .qwen_code_cli
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("qwen3-coder-plus");
        return ("Qwen CLI", model);
    }
    if let Some((name, cfg)) = config.providers.active_custom() {
        let model = cfg.default_model.as_deref().unwrap_or("default");
        return (name, model);
    }
    // Default - nothing configured
    ("Not configured", "N/A")
}
