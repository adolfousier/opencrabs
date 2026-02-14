//! Configuration Module
//!
//! Handles application configuration loading, validation, and management.

pub mod crabrace;
pub mod secrets;
pub mod update;

pub use crabrace::{CrabraceConfig, CrabraceIntegration};
pub use secrets::{ProviderSecrets, SecretString};
pub use update::{ProviderUpdater, UpdateResult};

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
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("opencrabs")
        .join("opencrabs.db")
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
        }
    }
}

impl Config {
    /// Load configuration from default locations
    ///
    /// Priority (lowest to highest):
    /// 1. Default values
    /// 2. System config: ~/.config/opencrabs/config.toml
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

    /// Get the system config path: ~/.config/opencrabs/config.toml
    fn system_config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("opencrabs").join("config.toml"))
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

        // OpenAI
        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            let provider = config.providers.openai.get_or_insert(ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
            });
            provider.api_key = Some(api_key);
        }

        // OpenAI base URL (for LM Studio, Ollama, etc.)
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

    /// Save configuration to a file
    pub fn save(&self, path: &Path) -> Result<()> {
        let toml_string =
            toml::to_string_pretty(self).context("Failed to serialize config to TOML")?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {:?}", parent))?;
        }

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
}
