//! Configuration types, defaults, loading, and validation.

use super::crabrace::CrabraceConfig;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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

    /// HTTP API gateway configuration
    #[serde(default)]
    pub gateway: GatewayConfig,

    /// Messaging channel integrations
    #[serde(default)]
    pub channels: ChannelsConfig,

    /// Voice processing (STT/TTS) configuration
    #[serde(default)]
    pub voice: VoiceConfig,

    /// Agent behaviour configuration
    #[serde(default)]
    pub agent: AgentConfig,

    /// A2A (Agent-to-Agent) protocol gateway configuration
    #[serde(default)]
    pub a2a: A2aConfig,
}

/// HTTP API gateway configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Gateway port (default: 18789)
    #[serde(default = "default_gateway_port")]
    pub port: u16,

    /// Bind address (default: "127.0.0.1")
    #[serde(default = "default_gateway_bind")]
    pub bind: String,

    /// Authentication mode: "token" or "none" (default: "token")
    #[serde(default = "default_gateway_auth")]
    pub auth_mode: String,

    /// Whether the gateway is enabled
    #[serde(default)]
    pub enabled: bool,
}

fn default_gateway_port() -> u16 {
    18789
}

fn default_gateway_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_gateway_auth() -> String {
    "token".to_string()
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

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: default_gateway_port(),
            bind: default_gateway_bind(),
            auth_mode: default_gateway_auth(),
            enabled: false,
        }
    }
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
    pub telegram: ChannelConfig,
    #[serde(default)]
    pub discord: ChannelConfig,
    #[serde(default)]
    pub whatsapp: ChannelConfig,
    #[serde(default)]
    pub slack: ChannelConfig,
    #[serde(default)]
    pub signal: ChannelConfig,
    #[serde(default)]
    pub google_chat: ChannelConfig,
    #[serde(default)]
    pub imessage: ChannelConfig,
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

/// Individual channel configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: Option<String>,
    /// Secondary token (Slack app-level token `xapp-...`)
    #[serde(default)]
    pub app_token: Option<String>,
    /// Allowlisted user IDs (Telegram user IDs, Discord user IDs, etc.)
    #[serde(default)]
    pub allowed_users: Vec<i64>,
    /// Allowlisted phone numbers for WhatsApp (E.164 format: "+15551234567")
    #[serde(default)]
    pub allowed_phones: Vec<String>,
    /// String-based user IDs (Slack `U12345678`, future channels)
    #[serde(default)]
    pub allowed_ids: Vec<String>,
    /// When the bot should respond: "all", "dm_only", or "mention" (default)
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Restrict bot to specific channel IDs. Empty = all channels. DMs always pass.
    #[serde(default)]
    pub allowed_channels: Vec<String>,
}

/// Voice processing configuration (STT + TTS)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Enable speech-to-text transcription
    #[serde(default = "default_true")]
    pub stt_enabled: bool,

    /// Enable text-to-speech replies
    #[serde(default)]
    pub tts_enabled: bool,

    /// TTS voice name (default: "ash")
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,

    /// TTS model (default: "gpt-4o-mini-tts")
    #[serde(default = "default_tts_model")]
    pub tts_model: String,

    /// STT provider config (runtime - from providers.stt.*)
    /// Not serialized to config file
    #[serde(skip, default)]
    pub stt_provider: Option<ProviderConfig>,

    /// TTS provider config (runtime - from providers.tts.*)
    /// Not serialized to config file
    #[serde(skip, default)]
    pub tts_provider: Option<ProviderConfig>,
}

fn default_true() -> bool {
    true
}
fn default_tts_voice() -> String {
    "ash".to_string()
}
fn default_tts_model() -> String {
    "gpt-4o-mini-tts".to_string()
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_enabled: true,
            tts_enabled: false,
            tts_voice: default_tts_voice(),
            tts_model: default_tts_model(),
            stt_provider: None,
            tts_provider: None,
        }
    }
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
}

fn default_approval_policy() -> String {
    "ask".to_string()
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

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            approval_policy: default_approval_policy(),
            max_concurrent: default_max_concurrent(),
            context_limit: default_context_limit(),
            max_tokens: default_max_tokens(),
        }
    }
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

    /// Named custom OpenAI-compatible providers (e.g. [providers.custom.ollama])
    #[serde(default, deserialize_with = "deserialize_custom_providers")]
    pub custom: Option<BTreeMap<String, ProviderConfig>>,

    /// Google Gemini configuration
    #[serde(default)]
    pub gemini: Option<ProviderConfig>,

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

    /// Get a specific custom provider by name
    pub fn custom_by_name(&self, name: &str) -> Option<&ProviderConfig> {
        self.custom.as_ref()?.get(name)
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

    // If it has "enabled", "api_key", "base_url", "default_model", or "models" at top level,
    // it's old flat format — wrap as "default"
    if value.get("enabled").is_some()
        || value.get("api_key").is_some()
        || value.get("base_url").is_some()
        || value.get("default_model").is_some()
        || value.get("models").is_some()
    {
        let config: ProviderConfig = value.try_into().map_err(de::Error::custom)?;
        let mut map = BTreeMap::new();
        map.insert("default".to_string(), config);
        return Ok(Some(map));
    }

    // Otherwise parse as named map
    let map: BTreeMap<String, ProviderConfig> = value.try_into().map_err(de::Error::custom)?;
    Ok(if map.is_empty() { None } else { Some(map) })
}

/// Fallback provider configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FallbackProviderConfig {
    /// Enable fallback
    #[serde(default)]
    pub enabled: bool,

    /// Fallback provider type
    #[serde(default)]
    pub provider: Option<String>,
}

/// STT (Speech-to-Text) provider configurations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SttProviders {
    /// Groq STT configuration
    #[serde(default)]
    pub groq: Option<ProviderConfig>,
}

/// TTS (Text-to-Speech) provider configurations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TtsProviders {
    /// OpenAI TTS configuration
    #[serde(default)]
    pub openai: Option<ProviderConfig>,
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

/// Canonical base directory: `~/.opencrabs/`
///
/// All OpenCrabs data lives here: config, database, history, brain workspace.
pub fn opencrabs_home() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let p = home.join(".opencrabs");
    if !p.exists() {
        let _ = std::fs::create_dir_all(&p);
    }
    p
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

    let path = keys_path();

    let mut doc: toml::Value = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let parts: Vec<&str> = section.split('.').collect();
    let mut current = doc
        .as_table_mut()
        .context("keys.toml root is not a table")?;

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
    current.insert(key.to_string(), toml::Value::String(value.to_string()));

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
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
    // Merge each provider's api_key if present in keys
    if let Some(k) = keys.anthropic
        && let Some(key) = k.api_key
    {
        let entry = base.anthropic.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(k) = keys.openai
        && let Some(key) = k.api_key
    {
        let entry = base.openai.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(k) = keys.openrouter
        && let Some(key) = k.api_key
    {
        let entry = base.openrouter.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(k) = keys.minimax
        && let Some(key) = k.api_key
    {
        let entry = base.minimax.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(k) = keys.gemini
        && let Some(key) = k.api_key
    {
        let entry = base.gemini.get_or_insert_with(ProviderConfig::default);
        entry.api_key = Some(key);
    }
    if let Some(custom_keys) = keys.custom {
        let base_customs = base.custom.get_or_insert_with(BTreeMap::new);
        for (name, key_cfg) in custom_keys {
            if let Some(key) = key_cfg.api_key {
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

    // WhatsApp
    if let Some(ref token) = keys.whatsapp.token
        && !token.is_empty()
    {
        base.whatsapp.token = Some(token.clone());
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
            gateway: GatewayConfig::default(),
            channels: ChannelsConfig::default(),
            voice: VoiceConfig::default(),
            agent: AgentConfig::default(),
            a2a: A2aConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from default locations
    ///
    /// Priority (lowest to highest):
    /// 1. Default values
    /// 2. System config: ~/.opencrabs/config.toml
    /// 3. Local config: ./opencrabs.toml
    /// 4. Environment variables
    pub fn load() -> Result<Self> {
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

        // 3. Load API keys from keys.toml (overrides config.toml keys)
        if let Ok(keys) = load_keys_from_file() {
            config.providers = merge_provider_keys(config.providers, keys.providers);
            config.channels = merge_channel_keys(config.channels, keys.channels);
            // Merge A2A API key from keys.toml
            if let Some(a2a_keys) = keys.a2a
                && let Some(key) = a2a_keys.api_key
                && !key.is_empty()
            {
                config.a2a.api_key = Some(key);
            }
        }

        // 4. Apply environment variable overrides
        config = Self::apply_env_overrides(config)?;

        // Expand tilde in database path (TOML doesn't expand ~)
        config.database.path = expand_tilde(&config.database.path);

        tracing::debug!("Configuration loaded successfully");
        Ok(config)
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
            gateway: overlay.gateway,
            channels: overlay.channels,
            voice: overlay.voice,
            agent: overlay.agent,
            a2a: overlay.a2a,
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
    /// `section` is a dotted path like "agent" or "voice".
    /// `key` is the field name inside that section.
    /// `value` is the TOML-serialisable value.
    pub fn write_key(section: &str, key: &str, value: &str) -> Result<()> {
        // Sanitize: trim whitespace/newlines that may leak from TUI input
        let value = value.trim();

        let path =
            Self::system_config_path().unwrap_or_else(|| opencrabs_home().join("config.toml"));

        // Read existing TOML or start fresh
        let mut doc: toml::Value = if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        // Navigate/create the section table (supports dotted paths like "channels.slack")
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
        Self::backup_config(&path, 5);

        let toml_str = toml::to_string_pretty(&doc)?;
        fs::write(&path, toml_str)?;
        tracing::info!("Wrote config key [{section}].{key} = {value}");
        Ok(())
    }

    /// Write a string array to a dotted config section.
    /// e.g. `write_array("channels.slack", "allowed_ids", &["U123"])` →
    /// `[channels.slack] allowed_ids = ["U123"]`
    pub fn write_array(section: &str, key: &str, values: &[String]) -> Result<()> {
        let path =
            Self::system_config_path().unwrap_or_else(|| opencrabs_home().join("config.toml"));

        let mut doc: toml::Value = if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
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
        Self::backup_config(&path, 5);
        let toml_str = toml::to_string_pretty(&doc)?;
        fs::write(&path, toml_str)?;
        tracing::info!(
            "Wrote config array [{section}].{key} ({} items)",
            values.len()
        );
        Ok(())
    }

    /// Write an array of integers to config.toml under `[section]`, creating
    /// intermediate tables as needed.
    ///
    /// e.g. `write_i64_array("channels.telegram", "allowed_users", &[123456])` →
    /// `[channels.telegram] allowed_users = [123456]`
    pub fn write_i64_array(section: &str, key: &str, values: &[i64]) -> Result<()> {
        let path =
            Self::system_config_path().unwrap_or_else(|| opencrabs_home().join("config.toml"));

        let mut doc: toml::Value = if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
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

        let arr = values.iter().map(|v| toml::Value::Integer(*v)).collect();
        current.insert(key.to_string(), toml::Value::Array(arr));

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Self::backup_config(&path, 5);
        let toml_str = toml::to_string_pretty(&doc)?;
        fs::write(&path, toml_str)?;
        tracing::info!(
            "Wrote config i64 array [{section}].{key} ({} items)",
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

    /// Rotate config backups before writing.
    ///
    /// Keeps up to `max_backups` copies named `config.toml.backup1` (newest)
    /// through `config.toml.backupN` (oldest). Oldest is deleted when limit is
    /// exceeded. Silently ignores errors — backup failure must never block a
    /// config write.
    fn backup_config(path: &Path, max_backups: usize) {
        // Only back up if the file actually exists
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

        // Rotate existing backups: N → N+1 (delete oldest if over limit)
        for i in (1..=max_backups).rev() {
            let src = parent.join(format!("{stem}.backup{i}"));
            if i == max_backups {
                // Drop the oldest backup
                let _ = fs::remove_file(&src);
            } else {
                let dst = parent.join(format!("{stem}.backup{}", i + 1));
                if src.exists() {
                    let _ = fs::rename(&src, &dst);
                }
            }
        }

        // Copy current config → backup1
        let backup1 = parent.join(format!("{stem}.backup1"));
        if let Err(e) = fs::copy(path, &backup1) {
            tracing::warn!("Failed to back up config before write: {e}");
        } else {
            tracing::debug!("Config backed up to {}", backup1.display());
        }
    }

    /// Save configuration to a file
    pub fn save(&self, path: &Path) -> Result<()> {
        let toml_string =
            toml::to_string_pretty(self).context("Failed to serialize config to TOML")?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {:?}", parent))?;
        }

        // Back up before overwriting
        Self::backup_config(path, 5);

        fs::write(path, toml_string)
            .with_context(|| format!("Failed to write config file: {:?}", path))?;

        tracing::info!("Configuration saved to: {:?}", path);
        Ok(())
    }
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
        assert_eq!(agent.approval_policy, "ask");
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
        assert_eq!(config.agent.approval_policy, "ask");
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
pub fn resolve_provider_from_config(config: &Config) -> (&str, &str) {
    // Check new dedicated providers first
    if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
        let model = config
            .providers
            .minimax
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("Minimax", model);
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
    if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
        let model = config
            .providers
            .gemini
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("Google Gemini", model);
    }
    if let Some((name, cfg)) = config.providers.active_custom() {
        let model = cfg.default_model.as_deref().unwrap_or("default");
        // Return a static string for the provider name; we can't return the dynamic name
        // from a borrowed reference easily, so we use "Custom" with the model
        let _ = name; // name is available but we return a static str
        return ("Custom", model);
    }
    // Default - nothing configured
    ("Not configured", "N/A")
}
