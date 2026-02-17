//! Configuration types, defaults, loading, and validation.

use super::crabrace::CrabraceConfig;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
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
    pub signal: ChannelConfig,
    #[serde(default)]
    pub google_chat: ChannelConfig,
    #[serde(default)]
    pub imessage: ChannelConfig,
}

/// Individual channel configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: Option<String>,
    /// Allowlisted user IDs (Telegram user IDs, Discord user IDs, etc.)
    #[serde(default)]
    pub allowed_users: Vec<i64>,
    /// Allowlisted phone numbers for WhatsApp (E.164 format: "+15551234567")
    #[serde(default)]
    pub allowed_phones: Vec<String>,
}

/// Voice processing configuration (STT + TTS)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Enable speech-to-text transcription (Groq Whisper)
    #[serde(default = "default_true")]
    pub stt_enabled: bool,

    /// Enable text-to-speech replies (OpenAI TTS)
    #[serde(default)]
    pub tts_enabled: bool,

    /// TTS voice name (default: "ash")
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,

    /// TTS model (default: "gpt-4o-mini-tts")
    #[serde(default = "default_tts_model")]
    pub tts_model: String,

    /// Groq API key for STT (loaded from GROQ_API_KEY env var)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groq_api_key: Option<String>,
}

fn default_true() -> bool { true }
fn default_tts_voice() -> String { "ash".to_string() }
fn default_tts_model() -> String { "gpt-4o-mini-tts".to_string() }

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_enabled: true,
            tts_enabled: false,
            tts_voice: default_tts_voice(),
            tts_model: default_tts_model(),
            groq_api_key: None,
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
}

fn default_approval_policy() -> String {
    "ask".to_string()
}

fn default_max_concurrent() -> u32 {
    4
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            approval_policy: default_approval_policy(),
            max_concurrent: default_max_concurrent(),
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

    /// OpenAI configuration
    #[serde(default)]
    pub openai: Option<ProviderConfig>,

    /// Qwen/DashScope configuration
    #[serde(default)]
    pub qwen: Option<QwenProviderConfig>,

    /// Google Gemini configuration
    #[serde(default)]
    pub gemini: Option<ProviderConfig>,

    /// AWS Bedrock configuration
    #[serde(default)]
    pub bedrock: Option<ProviderConfig>,

    /// Azure OpenAI configuration
    #[serde(default)]
    pub azure: Option<ProviderConfig>,

    /// VertexAI configuration
    #[serde(default)]
    pub vertex: Option<ProviderConfig>,
}

/// Individual provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Qwen-specific provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QwenProviderConfig {
    /// Provider enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// API key (for DashScope cloud)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// API base URL override
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Default model to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,

    /// Tool call parser: "hermes" or "openai" (default: hermes for local, openai for cloud)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_parser: Option<String>,

    /// Enable Qwen3 thinking mode
    #[serde(default)]
    pub enable_thinking: bool,

    /// Thinking budget tokens (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<u32>,

    /// DashScope region: "intl" (Singapore) or "cn" (Beijing)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
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
            && system_config_path.exists() {
                tracing::debug!("Loading system config from: {:?}", system_config_path);
                config = Self::merge_from_file(config, &system_config_path)?;
            }

        // 2. Try to load local config
        let local_config_path = Self::local_config_path();
        if local_config_path.exists() {
            tracing::debug!("Loading local config from: {:?}", local_config_path);
            config = Self::merge_from_file(config, &local_config_path)?;
        }

        // 3. Apply environment variable overrides
        config = Self::apply_env_overrides(config)?;

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

        tracing::debug!("Configuration loaded successfully from custom path");
        Ok(config)
    }

    /// Get the system config path: ~/.opencrabs/config.toml
    ///
    /// Falls back to legacy `~/.config/opencrabs/config.toml` for migration.
    pub fn system_config_path() -> Option<PathBuf> {
        let primary = Some(opencrabs_home().join("config.toml"));
        // Check legacy XDG path for migration
        let legacy = dirs::config_dir().map(|dir| dir.join("opencrabs").join("config.toml"));
        if primary.as_ref().is_some_and(|p| p.exists()) {
            return primary;
        }
        if legacy.as_ref().is_some_and(|p| p.exists()) {
            return legacy;
        }
        primary
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

        // Groq API key for voice STT
        if let Ok(groq_key) = std::env::var("GROQ_API_KEY") {
            config.voice.groq_api_key = Some(groq_key);
        }

        // Provider API keys from environment
        Self::load_provider_api_keys(&mut config)?;

        Ok(config)
    }

    /// Load provider API keys from environment variables
    fn load_provider_api_keys(config: &mut Self) -> Result<()> {
        // Anthropic
        // ANTHROPIC_MAX_SETUP_TOKEN takes priority over ANTHROPIC_API_KEY (OAuth token for Claude Max)
        if let Ok(api_key) = std::env::var("ANTHROPIC_MAX_SETUP_TOKEN")
            .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        {
            let provider = config.providers.anthropic.get_or_insert(ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
            });
            provider.api_key = Some(api_key);
        }

        // ANTHROPIC_MAX_MODEL sets the default model for Anthropic
        if let Ok(model) = std::env::var("ANTHROPIC_MAX_MODEL") {
            let provider = config.providers.anthropic.get_or_insert(ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
            });
            provider.default_model = Some(model);
        }

        // OpenAI — ONLY set api_key on an EXISTING provider config.
        // Do NOT auto-create an OpenAI text provider just from OPENAI_API_KEY,
        // because that key may only be for TTS (gpt-4o-mini-tts).
        // Users who want OpenAI for text must explicitly configure it in config.toml.
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY")
            && let Some(provider) = config.providers.openai.as_mut()
        {
            provider.api_key = Some(api_key);
        }

        // OpenAI base URL (for LM Studio, Ollama, etc.) — explicit local LLM setup
        if let Ok(base_url) = std::env::var("OPENAI_BASE_URL") {
            let provider = config.providers.openai.get_or_insert(ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
            });
            provider.base_url = Some(base_url);
        }

        // Google Gemini
        if let Ok(api_key) = std::env::var("GEMINI_API_KEY") {
            let provider = config.providers.gemini.get_or_insert(ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
            });
            provider.api_key = Some(api_key);
        }

        // AWS credentials are typically loaded via AWS SDK default chain
        // Azure uses AZURE_OPENAI_KEY and AZURE_OPENAI_ENDPOINT
        if let Ok(api_key) = std::env::var("AZURE_OPENAI_KEY") {
            let provider = config.providers.azure.get_or_insert(ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
            });
            provider.api_key = Some(api_key);
        }

        if let Ok(endpoint) = std::env::var("AZURE_OPENAI_ENDPOINT") {
            let provider = config.providers.azure.get_or_insert(ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
            });
            provider.base_url = Some(endpoint);
        }

        // Qwen/DashScope
        if let Ok(api_key) = std::env::var("DASHSCOPE_API_KEY") {
            let provider = config.providers.qwen.get_or_insert(QwenProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
                tool_parser: None,
                enable_thinking: false,
                thinking_budget: None,
                region: None,
            });
            provider.api_key = Some(api_key);
        }

        // Qwen base URL (for vLLM, LM Studio, etc.)
        if let Ok(base_url) = std::env::var("QWEN_BASE_URL") {
            let provider = config.providers.qwen.get_or_insert(QwenProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
                tool_parser: None,
                enable_thinking: false,
                thinking_budget: None,
                region: None,
            });
            provider.base_url = Some(base_url);
        }

        // Qwen thinking mode
        if let Ok(thinking) = std::env::var("QWEN_ENABLE_THINKING") {
            let provider = config.providers.qwen.get_or_insert(QwenProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
                tool_parser: None,
                enable_thinking: false,
                thinking_budget: None,
                region: None,
            });
            provider.enable_thinking = thinking.parse().unwrap_or(false);
        }

        Ok(())
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
        let path = Self::system_config_path()
            .unwrap_or_else(|| opencrabs_home().join("config.toml"));

        // Read existing TOML or start fresh
        let mut doc: toml::Value = if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        // Navigate/create the section table
        let table = doc.as_table_mut()
            .context("config root is not a table")?;

        let section_table = if let Some(existing) = table.get_mut(section) {
            existing.as_table_mut().context("section is not a table")?
        } else {
            table.insert(section.to_string(), toml::Value::Table(toml::map::Map::new()));
            table.get_mut(section)
                .context("just-inserted section missing")?
                .as_table_mut()
                .context("just-inserted section is not a table")?
        };

        // Parse the value — try integer, float, bool, then fall back to string
        let parsed: toml::Value = if let Ok(v) = value.parse::<i64>() {
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

    /// Validate configuration
    /// Check if any provider has an API key configured (from env, keyring, or config).
    pub fn has_any_api_key(&self) -> bool {
        let has_anthropic = self.providers.anthropic.as_ref()
            .is_some_and(|p| p.api_key.is_some());
        let has_openai = self.providers.openai.as_ref()
            .is_some_and(|p| p.api_key.is_some());
        let has_gemini = self.providers.gemini.as_ref()
            .is_some_and(|p| p.api_key.is_some());
        let has_qwen = self.providers.qwen.as_ref()
            .is_some_and(|p| p.api_key.is_some());

        has_anthropic || has_openai || has_gemini || has_qwen
    }

    pub fn validate(&self) -> Result<()> {
        tracing::debug!("Validating configuration...");

        // Validate database path parent directory exists
        if let Some(parent) = self.database.path.parent()
            && !parent.exists() {
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
        assert!(providers.azure.is_none());
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
                m.insert("approval_policy".to_string(), toml::Value::String("auto-session".to_string()));
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
