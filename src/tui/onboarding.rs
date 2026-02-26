//! Onboarding Wizard
//!
//! A 7-step TUI-based onboarding wizard for first-time OpenCrabs users.
//! Handles mode selection, provider/auth setup, workspace, gateway,
//! channels, daemon installation, and health check.

use crate::config::{Config, ProviderConfig};
use chrono::Local;

/// Sentinel value stored in api_key_input when a key was loaded from config.
/// The actual key is never held in memory — this just signals "key exists".
const EXISTING_KEY_SENTINEL: &str = "__EXISTING_KEY__";
use crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;

/// Provider definitions
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        name: "Anthropic Claude",
        models: &[], // Fetched from API
        key_label: "Setup Token",
        help_lines: &[
            "Claude Max / Code: run 'claude setup-token'",
            "Or paste API key from console.anthropic.com",
        ],
    },
    ProviderInfo {
        name: "OpenAI",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from platform.openai.com"],
    },
    ProviderInfo {
        name: "Google Gemini",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from aistudio.google.com"],
    },
    ProviderInfo {
        name: "OpenRouter",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from openrouter.ai/keys"],
    },
    ProviderInfo {
        name: "Minimax",
        models: &[], // Loaded from config.toml at runtime
        key_label: "API Key",
        help_lines: &["Get key from platform.minimax.io"],
    },
    ProviderInfo {
        name: "Custom OpenAI-Compatible",
        models: &[],
        key_label: "API Key",
        help_lines: &["Enter your own API endpoint"],
    },
];

pub struct ProviderInfo {
    pub name: &'static str,
    pub models: &'static [&'static str],
    pub key_label: &'static str,
    pub help_lines: &'static [&'static str],
}

/// Channel definitions for the unified Channels step.
/// Index mapping: 0=Telegram, 1=Discord, 2=WhatsApp, 3=Slack, 4=Signal, 5=Google Chat, 6=iMessage
pub const CHANNEL_NAMES: &[(&str, &str)] = &[
    ("Telegram", "Bot token (via @BotFather)"),
    ("Discord", "Bot token (via Developer Portal)"),
    ("WhatsApp", "QR code pairing"),
    ("Slack", "Socket Mode (bot + app tokens)"),
    ("Signal", "Coming soon"),
    ("Google Chat", "Coming soon"),
    ("iMessage", "Coming soon"),
];

/// Template files to seed in the workspace
const TEMPLATE_FILES: &[(&str, &str)] = &[
    (
        "SOUL.md",
        include_str!("../docs/reference/templates/SOUL.md"),
    ),
    (
        "IDENTITY.md",
        include_str!("../docs/reference/templates/IDENTITY.md"),
    ),
    (
        "USER.md",
        include_str!("../docs/reference/templates/USER.md"),
    ),
    (
        "AGENTS.md",
        include_str!("../docs/reference/templates/AGENTS.md"),
    ),
    (
        "TOOLS.md",
        include_str!("../docs/reference/templates/TOOLS.md"),
    ),
    (
        "MEMORY.md",
        include_str!("../docs/reference/templates/MEMORY.md"),
    ),
];

/// Current step in the onboarding wizard
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    ModeSelect,
    Workspace,
    ProviderAuth,
    Channels,
    TelegramSetup,
    DiscordSetup,
    WhatsAppSetup,
    SlackSetup,
    Gateway,
    VoiceSetup,
    Daemon,
    HealthCheck,
    BrainSetup,
    Complete,
}

impl OnboardingStep {
    /// Step number (1-based)
    pub fn number(&self) -> usize {
        match self {
            Self::ModeSelect => 1,
            Self::Workspace => 2,
            Self::ProviderAuth => 3,
            Self::Channels => 4,
            Self::TelegramSetup => 4, // sub-step of Channels
            Self::DiscordSetup => 4,  // sub-step of Channels
            Self::WhatsAppSetup => 4, // sub-step of Channels
            Self::SlackSetup => 4,    // sub-step of Channels
            Self::Gateway => 5,
            Self::VoiceSetup => 6,
            Self::Daemon => 7,
            Self::HealthCheck => 8,
            Self::BrainSetup => 9,
            Self::Complete => 10,
        }
    }

    /// Total number of steps (excluding Complete)
    pub fn total() -> usize {
        9
    }

    /// Step title
    pub fn title(&self) -> &'static str {
        match self {
            Self::ModeSelect => "Pick Your Vibe",
            Self::Workspace => "Home Base",
            Self::ProviderAuth => "Brain Fuel",
            Self::Channels => "Chat Me Anywhere",
            Self::TelegramSetup => "Telegram Bot",
            Self::DiscordSetup => "Discord Bot",
            Self::WhatsAppSetup => "WhatsApp",
            Self::SlackSetup => "Slack Bot",
            Self::Gateway => "API Gateway",
            Self::VoiceSetup => "Voice Superpowers",
            Self::Daemon => "Always On",
            Self::HealthCheck => "Vibe Check",
            Self::BrainSetup => "Make It Yours",
            Self::Complete => "Let's Go!",
        }
    }

    /// Step subtitle
    pub fn subtitle(&self) -> &'static str {
        match self {
            Self::ModeSelect => "Quick and easy or full control — your call",
            Self::Workspace => "Where my brain lives on disk",
            Self::ProviderAuth => "Pick your AI model and drop your key",
            Self::Channels => "Chat with me from your phone — Telegram, WhatsApp, whatever",
            Self::TelegramSetup => "Hook up your Telegram bot token",
            Self::DiscordSetup => "Hook up your Discord bot token",
            Self::WhatsAppSetup => "Scan the QR code with your phone",
            Self::SlackSetup => "Hook up your Slack bot and app tokens",
            Self::Gateway => "Open up an HTTP API if you want one",
            Self::VoiceSetup => "Talk to me, literally",
            Self::Daemon => "Keep me running in the background",
            Self::HealthCheck => "Making sure everything's wired up right",
            Self::BrainSetup => "Make me yours, drop some context so I actually get you",
            Self::Complete => "You're all set — let's build something cool",
        }
    }
}

/// Wizard mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardMode {
    QuickStart,
    Advanced,
}

/// Health check status for individual checks
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Pending,
    Running,
    Pass,
    Fail(String),
}

/// Which field is being actively edited in ProviderAuth step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthField {
    Provider,
    ApiKey,
    Model,
    CustomName,
    CustomBaseUrl,
    CustomApiKey,
    CustomModel,
}

/// Which field is focused in DiscordSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordField {
    BotToken,
    ChannelID,
    AllowedList,
}

/// Which field is focused in SlackSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackField {
    BotToken,
    AppToken,
    ChannelID,
    AllowedList,
}

/// Which field is focused in TelegramSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramField {
    BotToken,
    UserID,
}

/// Which field is focused in WhatsAppSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhatsAppField {
    Connection,
    PhoneAllowlist,
}

/// Channel test connection status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelTestStatus {
    Idle,
    Testing,
    Success,
    Failed(String),
}

/// Which field is focused in VoiceSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceField {
    GroqApiKey,
    TtsToggle,
}

/// Which text area is focused in BrainSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrainField {
    AboutMe,
    AboutAgent,
}

/// Main onboarding wizard state
pub struct OnboardingWizard {
    pub step: OnboardingStep,
    pub mode: WizardMode,

    // Step 2: Provider/Auth
    pub selected_provider: usize,
    pub api_key_input: String,
    pub api_key_cursor: usize,
    pub selected_model: usize,
    pub auth_field: AuthField,
    pub custom_provider_name: String,
    pub custom_base_url: String,
    pub custom_model: String,
    /// Models fetched live from provider API (overrides static list when non-empty)
    pub fetched_models: Vec<String>,
    pub models_fetching: bool,
    /// Models from config.toml (used when API fetch not available)
    pub config_models: Vec<String>,

    /// Step 4: Workspace
    pub workspace_path: String,
    pub seed_templates: bool,

    /// Step 4: Gateway
    pub gateway_port: String,
    pub gateway_bind: String,
    /// 0=Token, 1=None
    pub gateway_auth: usize,

    /// Step 5: Channels
    pub channel_toggles: Vec<(String, bool)>,

    /// Step 5b: Telegram Setup (shown when Telegram is enabled)
    pub telegram_field: TelegramField,
    pub telegram_token_input: String,
    pub telegram_user_id_input: String,

    /// Discord Setup (shown when Discord is enabled)
    pub discord_field: DiscordField,
    pub discord_token_input: String,
    pub discord_channel_id_input: String,
    pub discord_allowed_list_input: String,

    /// WhatsApp Setup (shown when WhatsApp is enabled)
    pub whatsapp_field: WhatsAppField,
    pub whatsapp_qr_text: Option<String>,
    pub whatsapp_connecting: bool,
    pub whatsapp_connected: bool,
    pub whatsapp_error: Option<String>,
    pub whatsapp_phone_input: String,

    /// Slack Setup (shown when Slack is enabled)
    pub slack_field: SlackField,
    pub slack_bot_token_input: String,
    pub slack_app_token_input: String,
    pub slack_channel_id_input: String,
    pub slack_allowed_list_input: String,

    /// Channel test connection status
    pub channel_test_status: ChannelTestStatus,

    /// Step 6: Voice Setup
    pub voice_field: VoiceField,
    pub groq_api_key_input: String,
    pub tts_enabled: bool,

    /// Step 7: Daemon
    pub install_daemon: bool,

    /// Step 7: Health check
    pub health_results: Vec<(String, HealthStatus)>,
    pub health_running: bool,
    pub health_complete: bool,

    /// Step 8: Brain Setup
    pub brain_field: BrainField,
    pub about_me: String,
    pub about_opencrabs: String,
    /// Original values loaded from workspace brain files (for change detection)
    pub original_about_me: String,
    pub original_about_opencrabs: String,
    pub brain_generating: bool,
    pub brain_generated: bool,
    pub brain_error: Option<String>,
    pub generated_soul: Option<String>,
    pub generated_identity: Option<String>,
    pub generated_user: Option<String>,
    pub generated_agents: Option<String>,
    pub generated_tools: Option<String>,
    pub generated_memory: Option<String>,

    /// Model filter (live search in model list)
    pub model_filter: String,

    /// Navigation
    pub focused_field: usize,
    pub error_message: Option<String>,
}

impl Default for OnboardingWizard {
    fn default() -> Self {
        Self::new()
    }
}

impl OnboardingWizard {
    /// Create a new wizard with default state
    /// Loads existing config if available to pre-fill settings
    pub fn new() -> Self {
        let default_workspace = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(".opencrabs");

        // config_models loaded on demand per provider via reload_config_models()
        let config_models = Vec::new();

        // Try to load existing config to pre-fill settings
        let existing_config = crate::config::Config::load().ok();

        // Detect existing enabled provider
        let (selected_provider, api_key_input, custom_base_url, custom_model) =
            if let Some(ref config) = existing_config {
                // Find first enabled provider
                if config
                    .providers
                    .anthropic
                    .as_ref()
                    .is_some_and(|p| p.enabled)
                {
                    (
                        0,
                        EXISTING_KEY_SENTINEL.to_string(),
                        String::new(),
                        String::new(),
                    )
                } else if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
                    (
                        1,
                        EXISTING_KEY_SENTINEL.to_string(),
                        String::new(),
                        String::new(),
                    )
                } else if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
                    (
                        2,
                        EXISTING_KEY_SENTINEL.to_string(),
                        String::new(),
                        String::new(),
                    )
                } else if config
                    .providers
                    .openrouter
                    .as_ref()
                    .is_some_and(|p| p.enabled)
                {
                    (
                        3,
                        EXISTING_KEY_SENTINEL.to_string(),
                        String::new(),
                        String::new(),
                    )
                } else if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
                    (
                        4,
                        EXISTING_KEY_SENTINEL.to_string(),
                        String::new(),
                        String::new(),
                    )
                } else if let Some((_name, c)) = config.providers.active_custom() {
                    let base = c.base_url.clone().unwrap_or_default();
                    let model = c.default_model.clone().unwrap_or_default();
                    (5, EXISTING_KEY_SENTINEL.to_string(), base, model)
                } else {
                    (0, String::new(), String::new(), String::new())
                }
            } else {
                (0, String::new(), String::new(), String::new())
            };

        // Pre-fill gateway settings from existing config
        let gateway_port = existing_config
            .as_ref()
            .map(|c| c.gateway.port.to_string())
            .unwrap_or_else(|| "18789".to_string());
        let gateway_bind = existing_config
            .as_ref()
            .map(|c| c.gateway.bind.clone())
            .unwrap_or_else(|| "127.0.0.1".to_string());

        let mut wizard = Self {
            step: OnboardingStep::ModeSelect,
            mode: WizardMode::QuickStart,

            selected_provider,
            api_key_input,
            api_key_cursor: 0,
            selected_model: 0,
            auth_field: AuthField::Provider,
            custom_provider_name: "default".to_string(),
            custom_base_url,
            custom_model,
            fetched_models: Vec::new(),
            models_fetching: false,
            config_models,

            workspace_path: default_workspace.to_string_lossy().to_string(),
            seed_templates: true,

            gateway_port,
            gateway_bind,
            gateway_auth: 0,

            channel_toggles: CHANNEL_NAMES
                .iter()
                .map(|(name, _desc)| (name.to_string(), false))
                .collect(),

            telegram_field: TelegramField::BotToken,
            telegram_token_input: String::new(),
            telegram_user_id_input: String::new(),

            discord_field: DiscordField::BotToken,
            discord_token_input: String::new(),
            discord_channel_id_input: String::new(),
            discord_allowed_list_input: String::new(),

            whatsapp_field: WhatsAppField::Connection,
            whatsapp_qr_text: None,
            whatsapp_connecting: false,
            whatsapp_connected: false,
            whatsapp_error: None,
            whatsapp_phone_input: String::new(),

            slack_field: SlackField::BotToken,
            slack_bot_token_input: String::new(),
            slack_app_token_input: String::new(),
            slack_channel_id_input: String::new(),
            slack_allowed_list_input: String::new(),

            channel_test_status: ChannelTestStatus::Idle,

            voice_field: VoiceField::GroqApiKey,
            groq_api_key_input: String::new(),
            tts_enabled: false,

            install_daemon: false,

            health_results: Vec::new(),
            health_running: false,
            health_complete: false,

            brain_field: BrainField::AboutMe,
            about_me: String::new(),
            about_opencrabs: String::new(),
            original_about_me: String::new(),
            original_about_opencrabs: String::new(),
            brain_generating: false,
            brain_generated: false,
            brain_error: None,
            generated_soul: None,
            generated_identity: None,
            generated_user: None,
            generated_agents: None,
            generated_tools: None,
            generated_memory: None,

            model_filter: String::new(),
            focused_field: 0,
            error_message: None,
        };

        // Load existing brain files from workspace if available
        let workspace = std::path::Path::new(&wizard.workspace_path);
        if let Ok(content) = std::fs::read_to_string(workspace.join("USER.md")) {
            let truncated = Self::truncate_preview(&content, 200);
            wizard.about_me = truncated.clone();
            wizard.original_about_me = truncated;
        }
        if let Ok(content) = std::fs::read_to_string(workspace.join("IDENTITY.md")) {
            let truncated = Self::truncate_preview(&content, 200);
            wizard.about_opencrabs = truncated.clone();
            wizard.original_about_opencrabs = truncated;
        }

        wizard
    }

    /// Create a wizard with existing config.toml values as defaults
    pub fn from_config(config: &Config) -> Self {
        let mut wizard = Self::new();

        // Determine which provider is configured and set selected_provider
        if config
            .providers
            .anthropic
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            wizard.selected_provider = 0; // Anthropic
            if let Some(model) = &config
                .providers
                .anthropic
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.custom_model = model.clone();
            }
        } else if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
            wizard.selected_provider = 4; // Minimax
            if let Some(model) = &config
                .providers
                .minimax
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.custom_model = model.clone();
            }
        } else if config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            wizard.selected_provider = 3; // OpenRouter - fetches from API
            if let Some(model) = &config
                .providers
                .openrouter
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.custom_model = model.clone();
            }
        } else if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
            // Custom OpenAI-compatible
            wizard.selected_provider = 5;
            if let Some(base_url) = &config
                .providers
                .openai
                .as_ref()
                .and_then(|p| p.base_url.clone())
            {
                wizard.custom_base_url = base_url.clone();
            }
            if let Some(model) = &config
                .providers
                .openai
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.custom_model = model.clone();
            }
        } else if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
            wizard.selected_provider = 2; // Gemini
        }

        // Detect if we have an existing API key for the selected provider
        wizard.detect_existing_key();
        wizard.reload_config_models();

        // Load gateway settings
        wizard.gateway_port = config.gateway.port.to_string();
        wizard.gateway_bind = config.gateway.bind.clone();
        wizard.gateway_auth = if config.gateway.auth_mode == "none" {
            1
        } else {
            0
        };

        // Load channel toggles (indices match CHANNEL_NAMES order)
        wizard.channel_toggles[0].1 = config.channels.telegram.enabled; // Telegram
        wizard.channel_toggles[1].1 = config.channels.discord.enabled; // Discord
        wizard.channel_toggles[2].1 = config.channels.whatsapp.enabled; // WhatsApp
        wizard.channel_toggles[3].1 = config.channels.slack.enabled; // Slack
        wizard.channel_toggles[4].1 = config.channels.signal.enabled; // Signal
        wizard.channel_toggles[5].1 = config.channels.google_chat.enabled; // Google Chat
        wizard.channel_toggles[6].1 = config.channels.imessage.enabled; // iMessage

        // Load voice settings
        wizard.tts_enabled = config.voice.tts_enabled;
        wizard.detect_existing_groq_key();

        // Jump directly to provider auth step since config exists
        wizard.step = OnboardingStep::ProviderAuth;
        wizard.auth_field = AuthField::Provider;

        wizard
    }

    /// Get provider info for currently selected provider
    pub fn current_provider(&self) -> &ProviderInfo {
        &PROVIDERS[self.selected_provider]
    }

    /// Check if the current provider is the "Custom" option
    pub fn is_custom_provider(&self) -> bool {
        self.selected_provider == PROVIDERS.len() - 1
    }

    /// Reload config_models for the currently selected provider.
    /// Tries config.toml first, falls back to config.toml.example defaults.
    fn reload_config_models(&mut self) {
        self.config_models.clear();
        // Try live config first
        if let Ok(config) = crate::config::Config::load() {
            match self.selected_provider {
                4 => {
                    if let Some(p) = &config.providers.minimax
                        && !p.models.is_empty()
                    {
                        self.config_models = p.models.clone();
                        return;
                    }
                }
                5 => {
                    if let Some((_name, p)) = config.providers.active_custom()
                        && !p.models.is_empty()
                    {
                        self.config_models = p.models.clone();
                        return;
                    }
                }
                _ => return,
            }
        }
        // Fall back to embedded config.toml.example
        self.config_models = Self::load_default_models(self.selected_provider);
    }

    /// All model names for the current provider (fetched or config or static fallback)
    pub fn all_model_names(&self) -> Vec<&str> {
        if !self.fetched_models.is_empty() {
            self.fetched_models.iter().map(|s| s.as_str()).collect()
        } else if !self.config_models.is_empty() {
            self.config_models.iter().map(|s| s.as_str()).collect()
        } else {
            self.current_provider().models.to_vec()
        }
    }

    /// Model names filtered by `model_filter` (case-insensitive substring match).
    /// Returns all models when filter is empty.
    pub fn filtered_model_names(&self) -> Vec<&str> {
        let all = self.all_model_names();
        if self.model_filter.is_empty() {
            all
        } else {
            let q = self.model_filter.to_lowercase();
            all.into_iter()
                .filter(|m| m.to_lowercase().contains(&q))
                .collect()
        }
    }

    /// Number of models available after applying the current filter
    pub fn model_count(&self) -> usize {
        self.filtered_model_names().len()
    }

    /// Get the selected model name (resolves through filter)
    pub fn selected_model_name(&self) -> &str {
        let filtered = self.filtered_model_names();
        if let Some(name) = filtered.get(self.selected_model) {
            name
        } else {
            // fallback: first unfiltered model
            self.all_model_names().first().copied().unwrap_or("default")
        }
    }

    /// Whether the current provider supports live model fetching
    pub fn supports_model_fetch(&self) -> bool {
        matches!(self.selected_provider, 0 | 1 | 3) // Anthropic, OpenAI, OpenRouter
    }

    /// Load default models from embedded config.toml.example for MiniMax and Custom
    fn load_default_models(provider_index: usize) -> Vec<String> {
        // Parse the embedded config.toml.example to extract default models for a specific provider
        let config_content = include_str!("../../config.toml.example");
        let mut models = Vec::new();

        if let Ok(config) = config_content.parse::<toml::Value>()
            && let Some(providers) = config.get("providers")
        {
            match provider_index {
                4 => {
                    // Minimax only
                    if let Some(minimax) = providers.get("minimax")
                        && let Some(models_arr) = minimax.get("models").and_then(|m| m.as_array())
                    {
                        for model in models_arr {
                            if let Some(model_str) = model.as_str() {
                                models.push(model_str.to_string());
                            }
                        }
                    }
                }
                5 => {
                    // Custom providers only
                    if let Some(custom) = providers.get("custom")
                        && let Some(custom_table) = custom.as_table()
                    {
                        for (_name, entry) in custom_table {
                            if let Some(models_arr) = entry.get("models").and_then(|m| m.as_array())
                            {
                                for model in models_arr {
                                    if let Some(model_str) = model.as_str()
                                        && !models.contains(&model_str.to_string())
                                    {
                                        models.push(model_str.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        tracing::debug!(
            "Loaded {} default models from config.toml.example for provider {}",
            models.len(),
            provider_index
        );
        models
    }

    /// Whether the current api_key_input holds a pre-existing key (from env/keyring)
    pub fn has_existing_key(&self) -> bool {
        self.api_key_input == EXISTING_KEY_SENTINEL
    }

    /// Try to load an existing API key for the currently selected provider.
    /// Checks keys.toml (merged into config) for the API key. If found, sets sentinel.
    pub fn detect_existing_key(&mut self) {
        // Helper: true when the provider has a non-empty API key
        fn has_nonempty_key(p: Option<&ProviderConfig>) -> bool {
            p.and_then(|p| p.api_key.as_ref())
                .is_some_and(|k| !k.is_empty())
        }

        if let Ok(config) = crate::config::Config::load() {
            let has_key = match self.selected_provider {
                0 => has_nonempty_key(config.providers.anthropic.as_ref()),
                1 => has_nonempty_key(config.providers.openai.as_ref()),
                2 => has_nonempty_key(config.providers.gemini.as_ref()),
                3 => has_nonempty_key(config.providers.openrouter.as_ref()),
                4 => has_nonempty_key(config.providers.minimax.as_ref()),
                5 => {
                    // Custom provider - also load base_url, model, and name
                    if let Some((name, c)) = config.providers.active_custom() {
                        if c.api_key.as_ref().is_some_and(|k| !k.is_empty()) {
                            self.custom_provider_name = name.to_string();
                            self.custom_base_url = c.base_url.clone().unwrap_or_default();
                            self.custom_model = c.default_model.clone().unwrap_or_default();
                            self.api_key_input = EXISTING_KEY_SENTINEL.to_string();
                        }
                        c.base_url.as_ref().is_some_and(|u| !u.is_empty())
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if has_key {
                self.api_key_input = EXISTING_KEY_SENTINEL.to_string();
                self.api_key_cursor = 0;
            }
        }
    }

    /// Advance to the next step
    pub fn next_step(&mut self) {
        self.error_message = None;
        self.focused_field = 0;

        match self.step {
            OnboardingStep::ModeSelect => {
                self.step = OnboardingStep::Workspace;
            }
            OnboardingStep::Workspace => {
                // Create config files in the workspace directory
                if let Err(e) = self.ensure_config_files() {
                    self.error_message = Some(format!("Failed to create config files: {}", e));
                    return;
                }
                self.step = OnboardingStep::ProviderAuth;
                self.auth_field = AuthField::Provider;
                self.detect_existing_key();
            }
            OnboardingStep::ProviderAuth => {
                // Validate API key is provided
                if self.api_key_input.is_empty() && !self.is_custom_provider() {
                    self.error_message = Some("API key is required".to_string());
                    return;
                }
                if self.is_custom_provider()
                    && (self.custom_base_url.is_empty() || self.custom_model.is_empty())
                {
                    self.error_message = Some(
                        "Base URL and model name are required for custom provider".to_string(),
                    );
                    return;
                }
                // QuickStart: skip channels, go straight to gateway
                if self.mode == WizardMode::QuickStart {
                    self.step = OnboardingStep::Gateway;
                } else {
                    tracing::debug!("[next_step] ProviderAuth → Channels");
                    self.step = OnboardingStep::Channels;
                    self.focused_field = 0;
                }
            }
            OnboardingStep::Channels => {
                // Handled by handle_channels_key — Enter on focused channel or Continue
                self.step = OnboardingStep::Gateway;
            }
            OnboardingStep::TelegramSetup
            | OnboardingStep::DiscordSetup
            | OnboardingStep::WhatsAppSetup
            | OnboardingStep::SlackSetup => {
                // Return to channel list after completing a channel setup
                self.step = OnboardingStep::Channels;
            }
            OnboardingStep::Gateway => {
                // QuickStart: skip voice, go straight to daemon
                if self.mode == WizardMode::QuickStart {
                    self.step = OnboardingStep::Daemon;
                } else {
                    self.step = OnboardingStep::VoiceSetup;
                    self.voice_field = VoiceField::GroqApiKey;
                    self.detect_existing_groq_key();
                }
            }
            OnboardingStep::VoiceSetup => {
                self.step = OnboardingStep::Daemon;
            }
            OnboardingStep::Daemon => {
                self.step = OnboardingStep::HealthCheck;
                self.start_health_check();
            }
            OnboardingStep::HealthCheck => {
                self.step = OnboardingStep::BrainSetup;
                self.brain_field = BrainField::AboutMe;
            }
            OnboardingStep::BrainSetup => {
                if self.brain_generated || self.brain_error.is_some() {
                    self.step = OnboardingStep::Complete;
                }
                // Otherwise wait for generation to finish or user to trigger it
            }
            OnboardingStep::Complete => {
                // Already complete
            }
        }
    }

    /// Go back to the previous step
    pub fn prev_step(&mut self) -> bool {
        self.error_message = None;
        self.focused_field = 0;

        match self.step {
            OnboardingStep::ModeSelect => {
                // Can't go back further — return true to signal "cancel wizard"
                return true;
            }
            OnboardingStep::Workspace => {
                self.step = OnboardingStep::ModeSelect;
            }
            OnboardingStep::ProviderAuth => {
                self.step = OnboardingStep::Workspace;
            }
            OnboardingStep::Channels => {
                self.step = OnboardingStep::ProviderAuth;
                self.auth_field = AuthField::Provider;
            }
            OnboardingStep::TelegramSetup => {
                self.step = OnboardingStep::Channels;
            }
            OnboardingStep::DiscordSetup
            | OnboardingStep::WhatsAppSetup
            | OnboardingStep::SlackSetup => {
                self.step = OnboardingStep::Channels;
            }
            OnboardingStep::Gateway => {
                if self.mode == WizardMode::QuickStart {
                    self.step = OnboardingStep::ProviderAuth;
                    self.auth_field = AuthField::Provider;
                } else {
                    self.step = OnboardingStep::Channels;
                }
            }
            OnboardingStep::VoiceSetup => {
                self.step = OnboardingStep::Gateway;
            }
            OnboardingStep::Daemon => {
                // QuickStart: go back to Gateway, Advanced: go back to VoiceSetup
                if self.mode == WizardMode::QuickStart {
                    self.step = OnboardingStep::Gateway;
                } else {
                    self.step = OnboardingStep::VoiceSetup;
                    self.voice_field = VoiceField::GroqApiKey;
                }
            }
            OnboardingStep::HealthCheck => {
                self.step = OnboardingStep::Daemon;
            }
            OnboardingStep::BrainSetup => {
                self.step = OnboardingStep::HealthCheck;
                self.brain_generating = false;
                self.brain_error = None;
            }
            OnboardingStep::Complete => {
                self.step = OnboardingStep::BrainSetup;
                self.brain_field = BrainField::AboutMe;
            }
        }
        false
    }

    /// Ensure config.toml and keys.toml exist in the workspace directory
    fn ensure_config_files(&mut self) -> Result<(), String> {
        let workspace_path = std::path::PathBuf::from(&self.workspace_path);

        // Create workspace directory if it doesn't exist
        if !workspace_path.exists() {
            std::fs::create_dir_all(&workspace_path)
                .map_err(|e| format!("Failed to create workspace directory: {}", e))?;
        }

        let config_path = workspace_path.join("config.toml");
        let keys_path = workspace_path.join("keys.toml");

        // Create config.toml if it doesn't exist (copy from embedded example)
        if !config_path.exists() {
            let config_content = include_str!("../../config.toml.example");
            std::fs::write(&config_path, config_content)
                .map_err(|e| format!("Failed to write config.toml: {}", e))?;
            tracing::info!("Created config.toml at {:?}", config_path);
        }

        // Create keys.toml if it doesn't exist (copy from embedded example)
        if !keys_path.exists() {
            let keys_content = include_str!("../../keys.toml.example");
            std::fs::write(&keys_path, keys_content)
                .map_err(|e| format!("Failed to write keys.toml: {}", e))?;
            tracing::info!("Created keys.toml at {:?}", keys_path);
        }

        // Create usage_pricing.toml if it doesn't exist
        let pricing_path = workspace_path.join("usage_pricing.toml");
        if !pricing_path.exists() {
            let pricing_content = include_str!("../../usage_pricing.toml.example");
            std::fs::write(&pricing_path, pricing_content)
                .map_err(|e| format!("Failed to write usage_pricing.toml: {}", e))?;
            tracing::info!("Created usage_pricing.toml at {:?}", pricing_path);
        }

        // Reload models for the selected provider from the newly created config
        self.reload_config_models();

        Ok(())
    }

    /// Initialize health check results
    fn start_health_check(&mut self) {
        let mut checks = vec![
            ("API Key Present".to_string(), HealthStatus::Pending),
            ("Config File".to_string(), HealthStatus::Pending),
            ("Workspace Directory".to_string(), HealthStatus::Pending),
            ("Template Files".to_string(), HealthStatus::Pending),
        ];

        // Add channel-specific checks for enabled channels
        if self.is_telegram_enabled() {
            checks.push(("Telegram Token".to_string(), HealthStatus::Pending));
            checks.push(("Telegram User ID".to_string(), HealthStatus::Pending));
        }
        if self.is_discord_enabled() {
            checks.push(("Discord Token".to_string(), HealthStatus::Pending));
            checks.push(("Discord Channel ID".to_string(), HealthStatus::Pending));
        }
        if self.is_slack_enabled() {
            checks.push(("Slack Bot Token".to_string(), HealthStatus::Pending));
            checks.push(("Slack Channel ID".to_string(), HealthStatus::Pending));
        }
        if self.is_whatsapp_enabled() {
            checks.push(("WhatsApp Connected".to_string(), HealthStatus::Pending));
        }

        self.health_results = checks;
        self.health_running = true;
        self.health_complete = false;

        // Run health checks synchronously (they're fast local checks)
        self.run_health_checks();
    }

    /// Execute all health checks
    fn run_health_checks(&mut self) {
        // Check 1: API key present
        self.health_results[0].1 = if !self.api_key_input.is_empty()
            || (self.is_custom_provider() && !self.custom_base_url.is_empty())
        {
            HealthStatus::Pass
        } else {
            HealthStatus::Fail("No API key provided".to_string())
        };

        // Check 2: Config path writable
        let config_path = crate::config::opencrabs_home().join("config.toml");
        self.health_results[1].1 = if let Some(parent) = config_path.parent() {
            if parent.exists() || std::fs::create_dir_all(parent).is_ok() {
                HealthStatus::Pass
            } else {
                HealthStatus::Fail(format!("Cannot create {}", parent.display()))
            }
        } else {
            HealthStatus::Fail("Invalid config path".to_string())
        };

        // Check 3: Workspace directory
        let workspace = PathBuf::from(&self.workspace_path);
        self.health_results[2].1 =
            if workspace.exists() || std::fs::create_dir_all(&workspace).is_ok() {
                HealthStatus::Pass
            } else {
                HealthStatus::Fail(format!("Cannot create {}", workspace.display()))
            };

        // Check 4: Template files available (they're compiled in, always present)
        self.health_results[3].1 = HealthStatus::Pass;

        // Channel checks (by name, since indices depend on which channels are enabled)
        for i in 0..self.health_results.len() {
            let name = self.health_results[i].0.clone();
            self.health_results[i].1 = match name.as_str() {
                "Telegram Token" => {
                    if !self.telegram_token_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No token provided".to_string())
                    }
                }
                "Telegram User ID" => {
                    if !self.telegram_user_id_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No user ID — bot won't know who to talk to".to_string())
                    }
                }
                "Discord Token" => {
                    if !self.discord_token_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No token provided".to_string())
                    }
                }
                "Discord Channel ID" => {
                    if !self.discord_channel_id_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail(
                            "No channel ID — bot won't know where to post".to_string(),
                        )
                    }
                }
                "Slack Bot Token" => {
                    if !self.slack_bot_token_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No bot token provided".to_string())
                    }
                }
                "Slack Channel ID" => {
                    if !self.slack_channel_id_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail(
                            "No channel ID — bot won't know where to post".to_string(),
                        )
                    }
                }
                "WhatsApp Connected" => {
                    if self.whatsapp_connected {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("Not paired — scan QR code to connect".to_string())
                    }
                }
                _ => continue, // Already set above
            };
        }

        self.health_running = false;
        self.health_complete = true;
    }

    /// Check if all health checks passed
    pub fn all_health_passed(&self) -> bool {
        self.health_complete
            && self
                .health_results
                .iter()
                .all(|(_, s)| matches!(s, HealthStatus::Pass))
    }

    /// Handle key events for the current step
    /// Returns `WizardAction` indicating what the app should do
    pub fn handle_key(&mut self, event: KeyEvent) -> WizardAction {
        // Global: Escape goes back (but if model filter is active, clear it first)
        if event.code == KeyCode::Esc {
            if !self.model_filter.is_empty() {
                self.model_filter.clear();
                self.selected_model = 0;
                return WizardAction::None;
            }
            if self.prev_step() {
                return WizardAction::Cancel;
            }
            return WizardAction::None;
        }

        match self.step {
            OnboardingStep::ModeSelect => self.handle_mode_select_key(event),
            OnboardingStep::ProviderAuth => self.handle_provider_auth_key(event),
            OnboardingStep::Workspace => self.handle_workspace_key(event),
            OnboardingStep::Gateway => self.handle_gateway_key(event),
            OnboardingStep::Channels => self.handle_channels_key(event),
            OnboardingStep::TelegramSetup => self.handle_telegram_setup_key(event),
            OnboardingStep::DiscordSetup => self.handle_discord_setup_key(event),
            OnboardingStep::WhatsAppSetup => self.handle_whatsapp_setup_key(event),
            OnboardingStep::SlackSetup => self.handle_slack_setup_key(event),
            OnboardingStep::VoiceSetup => self.handle_voice_setup_key(event),
            OnboardingStep::Daemon => self.handle_daemon_key(event),
            OnboardingStep::HealthCheck => self.handle_health_check_key(event),
            OnboardingStep::BrainSetup => self.handle_brain_setup_key(event),
            OnboardingStep::Complete => WizardAction::Complete,
        }
    }

    /// Handle paste event - inserts text at current cursor position
    pub fn handle_paste(&mut self, text: &str) {
        // Sanitize pasted text: take first line only, strip \r\n and whitespace
        let clean = text.split(['\r', '\n']).next().unwrap_or("").trim();
        if clean.is_empty() {
            return;
        }

        // Dispatch paste based on current step first, then auth_field
        match self.step {
            OnboardingStep::TelegramSetup => {
                tracing::debug!(
                    "[paste] Telegram pasted ({} chars) field={:?}",
                    clean.len(),
                    self.telegram_field
                );
                match self.telegram_field {
                    TelegramField::BotToken => {
                        if self.has_existing_telegram_token() {
                            self.telegram_token_input.clear();
                        }
                        self.telegram_token_input.push_str(clean);
                    }
                    TelegramField::UserID => {
                        // Only accept digits for user ID paste
                        let digits: String = clean.chars().filter(|c| c.is_ascii_digit()).collect();
                        if !digits.is_empty() {
                            if self.has_existing_telegram_user_id() {
                                self.telegram_user_id_input.clear();
                            }
                            self.telegram_user_id_input.push_str(&digits);
                        }
                    }
                }
            }
            OnboardingStep::DiscordSetup => {
                tracing::debug!(
                    "[paste] Discord pasted ({} chars) field={:?}",
                    clean.len(),
                    self.discord_field
                );
                match self.discord_field {
                    DiscordField::BotToken => {
                        if self.has_existing_discord_token() {
                            self.discord_token_input.clear();
                        }
                        self.discord_token_input.push_str(clean);
                    }
                    DiscordField::ChannelID => {
                        if self.has_existing_discord_channel_id() {
                            self.discord_channel_id_input.clear();
                        }
                        self.discord_channel_id_input.push_str(clean);
                    }
                    DiscordField::AllowedList => {
                        let digits: String = clean.chars().filter(|c| c.is_ascii_digit()).collect();
                        if !digits.is_empty() {
                            if self.has_existing_discord_allowed_list() {
                                self.discord_allowed_list_input.clear();
                            }
                            self.discord_allowed_list_input.push_str(&digits);
                        }
                    }
                }
            }
            OnboardingStep::SlackSetup => {
                tracing::debug!(
                    "[paste] Slack pasted ({} chars) field={:?}",
                    clean.len(),
                    self.slack_field
                );
                match self.slack_field {
                    SlackField::BotToken => {
                        if self.has_existing_slack_bot_token() {
                            self.slack_bot_token_input.clear();
                        }
                        self.slack_bot_token_input.push_str(clean);
                    }
                    SlackField::AppToken => {
                        if self.has_existing_slack_app_token() {
                            self.slack_app_token_input.clear();
                        }
                        self.slack_app_token_input.push_str(clean);
                    }
                    SlackField::ChannelID => {
                        if self.has_existing_slack_channel_id() {
                            self.slack_channel_id_input.clear();
                        }
                        self.slack_channel_id_input.push_str(clean);
                    }
                    SlackField::AllowedList => {
                        if self.has_existing_slack_allowed_list() {
                            self.slack_allowed_list_input.clear();
                        }
                        self.slack_allowed_list_input.push_str(clean);
                    }
                }
            }
            OnboardingStep::WhatsAppSetup => {
                if self.whatsapp_field == WhatsAppField::PhoneAllowlist {
                    // Accept digits, +, - for phone number
                    let phone: String = clean
                        .chars()
                        .filter(|c| c.is_ascii_digit() || *c == '+' || *c == '-')
                        .collect();
                    if !phone.is_empty() {
                        if self.has_existing_whatsapp_phone() {
                            self.whatsapp_phone_input.clear();
                        }
                        self.whatsapp_phone_input.push_str(&phone);
                    }
                }
            }
            OnboardingStep::VoiceSetup => {
                tracing::debug!("[paste] Groq API key pasted ({} chars)", clean.len());
                if self.has_existing_groq_key() {
                    self.groq_api_key_input.clear();
                }
                self.groq_api_key_input.push_str(clean);
            }
            OnboardingStep::ProviderAuth => match self.auth_field {
                AuthField::ApiKey | AuthField::CustomApiKey => {
                    if self.has_existing_key() {
                        self.api_key_input.clear();
                    }
                    self.api_key_input.push_str(clean);
                    self.api_key_cursor = self.api_key_input.len();
                }
                AuthField::CustomName => {
                    self.custom_provider_name.push_str(clean);
                }
                AuthField::CustomBaseUrl => {
                    self.custom_base_url.push_str(clean);
                }
                AuthField::CustomModel => {
                    self.custom_model.push_str(clean);
                }
                _ => {}
            },
            _ => {}
        }
    }

    // --- Step-specific key handlers ---

    fn handle_mode_select_key(&mut self, event: KeyEvent) -> WizardAction {
        match event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.mode = WizardMode::QuickStart;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.mode = WizardMode::Advanced;
            }
            KeyCode::Char('1') => {
                self.mode = WizardMode::QuickStart;
            }
            KeyCode::Char('2') => {
                self.mode = WizardMode::Advanced;
            }
            KeyCode::Enter => {
                self.next_step();
                // If entering ProviderAuth with existing key detected, pre-fetch models
                if self.step == OnboardingStep::ProviderAuth
                    && self.has_existing_key()
                    && self.supports_model_fetch()
                {
                    return WizardAction::FetchModels;
                }
            }
            _ => {}
        }
        WizardAction::None
    }

    fn handle_provider_auth_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.auth_field {
            AuthField::Provider => match event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.selected_provider = self.selected_provider.saturating_sub(1);
                    self.selected_model = 0;
                    self.model_filter.clear();
                    self.api_key_input.clear();
                    self.fetched_models.clear();
                    self.config_models.clear();
                    self.reload_config_models();
                    self.detect_existing_key();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selected_provider = (self.selected_provider + 1).min(PROVIDERS.len() - 1);
                    self.selected_model = 0;
                    self.model_filter.clear();
                    self.api_key_input.clear();
                    self.fetched_models.clear();
                    self.config_models.clear();
                    self.reload_config_models();
                    self.detect_existing_key();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.detect_existing_key();
                    if self.is_custom_provider() {
                        self.auth_field = AuthField::CustomName;
                    } else {
                        self.auth_field = AuthField::ApiKey;
                    }
                }
                _ => {}
            },
            AuthField::ApiKey => match event.code {
                KeyCode::Char(c) => {
                    // If existing key is loaded and user starts typing, clear it (replace mode)
                    if self.has_existing_key() {
                        self.api_key_input.clear();
                    }
                    self.api_key_input.push(c);
                    self.api_key_cursor = self.api_key_input.len();
                }
                KeyCode::Backspace => {
                    // If existing key sentinel, clear entirely on backspace
                    if self.has_existing_key() {
                        self.api_key_input.clear();
                    } else {
                        self.api_key_input.pop();
                    }
                    self.api_key_cursor = self.api_key_input.len();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.auth_field = AuthField::Model;
                    // Fetch live models when we have a key and provider supports it
                    if self.supports_model_fetch()
                        && (!self.api_key_input.is_empty() || self.has_existing_key())
                    {
                        self.fetched_models.clear();
                        self.selected_model = 0;
                        return WizardAction::FetchModels;
                    }
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::Provider;
                }
                _ => {}
            },
            AuthField::Model => match event.code {
                KeyCode::Up => {
                    self.selected_model = self.selected_model.saturating_sub(1);
                }
                KeyCode::Down => {
                    let count = self.model_count();
                    if count > 0 {
                        self.selected_model = (self.selected_model + 1).min(count - 1);
                    }
                }
                KeyCode::Char(c) if event.modifiers.is_empty() => {
                    self.model_filter.push(c);
                    self.selected_model = 0; // reset selection on filter change
                }
                KeyCode::Backspace => {
                    if self.model_filter.is_empty() {
                        self.auth_field = AuthField::ApiKey;
                    } else {
                        self.model_filter.pop();
                        self.selected_model = 0;
                    }
                }
                KeyCode::Enter => {
                    self.next_step();
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::ApiKey;
                    self.model_filter.clear();
                    self.selected_model = 0;
                }
                KeyCode::Tab => {
                    self.next_step();
                }
                _ => {}
            },
            AuthField::CustomName => match event.code {
                KeyCode::Char(c) => {
                    self.custom_provider_name.push(c);
                }
                KeyCode::Backspace => {
                    self.custom_provider_name.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    // Default to "default" if empty, always lowercase for config consistency
                    if self.custom_provider_name.is_empty() {
                        self.custom_provider_name = "default".to_string();
                    } else {
                        self.custom_provider_name = self.custom_provider_name.to_lowercase();
                    }
                    self.auth_field = AuthField::CustomBaseUrl;
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::Provider;
                }
                _ => {}
            },
            AuthField::CustomBaseUrl => match event.code {
                KeyCode::Char(c) => {
                    self.custom_base_url.push(c);
                }
                KeyCode::Backspace => {
                    self.custom_base_url.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.auth_field = AuthField::CustomApiKey;
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::CustomName;
                }
                _ => {}
            },
            AuthField::CustomApiKey => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_key() {
                        self.api_key_input.clear();
                    }
                    self.api_key_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_key() {
                        self.api_key_input.clear();
                    } else {
                        self.api_key_input.pop();
                    }
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.auth_field = AuthField::CustomModel;
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::CustomBaseUrl;
                }
                _ => {}
            },
            AuthField::CustomModel => match event.code {
                KeyCode::Char(c) => {
                    self.custom_model.push(c);
                }
                KeyCode::Backspace => {
                    self.custom_model.pop();
                }
                KeyCode::Enter => {
                    self.next_step();
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::CustomApiKey;
                }
                KeyCode::Tab => {
                    self.next_step();
                }
                _ => {}
            },
        }
        WizardAction::None
    }

    fn handle_workspace_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.focused_field {
            0 => {
                // Editing workspace path
                match event.code {
                    KeyCode::Char(c) => {
                        self.workspace_path.push(c);
                    }
                    KeyCode::Backspace => {
                        self.workspace_path.pop();
                    }
                    KeyCode::Tab => {
                        self.focused_field = 1;
                    }
                    KeyCode::Enter => {
                        self.workspace_path = self.workspace_path.trim().to_string();
                        self.next_step();
                        return self.maybe_fetch_models();
                    }
                    _ => {}
                }
            }
            1 => {
                // Seed templates toggle
                match event.code {
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        self.seed_templates = !self.seed_templates;
                    }
                    KeyCode::Tab => {
                        self.focused_field = 2;
                    }
                    KeyCode::BackTab => {
                        self.focused_field = 0;
                    }
                    _ => {}
                }
            }
            _ => {
                // "Next" button
                match event.code {
                    KeyCode::Enter => {
                        self.next_step();
                        return self.maybe_fetch_models();
                    }
                    KeyCode::BackTab => {
                        self.focused_field = 1;
                    }
                    _ => {}
                }
            }
        }
        WizardAction::None
    }

    /// If we just entered ProviderAuth with an existing key, trigger model fetch
    fn maybe_fetch_models(&self) -> WizardAction {
        if self.step == OnboardingStep::ProviderAuth
            && self.has_existing_key()
            && self.supports_model_fetch()
        {
            WizardAction::FetchModels
        } else {
            WizardAction::None
        }
    }

    fn handle_gateway_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.focused_field {
            0 => {
                // Port
                match event.code {
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        self.gateway_port.push(c);
                    }
                    KeyCode::Backspace => {
                        self.gateway_port.pop();
                    }
                    KeyCode::Tab | KeyCode::Enter => {
                        self.focused_field = 1;
                    }
                    _ => {}
                }
            }
            1 => {
                // Bind address
                match event.code {
                    KeyCode::Char(c) => {
                        self.gateway_bind.push(c);
                    }
                    KeyCode::Backspace => {
                        self.gateway_bind.pop();
                    }
                    KeyCode::Tab | KeyCode::Enter => {
                        self.focused_field = 2;
                    }
                    KeyCode::BackTab => {
                        self.focused_field = 0;
                    }
                    _ => {}
                }
            }
            2 => {
                // Auth mode
                match event.code {
                    KeyCode::Up | KeyCode::Down | KeyCode::Char(' ') => {
                        self.gateway_auth = if self.gateway_auth == 0 { 1 } else { 0 };
                    }
                    KeyCode::Enter => {
                        self.next_step();
                    }
                    KeyCode::BackTab => {
                        self.focused_field = 1;
                    }
                    _ => {}
                }
            }
            _ => {
                if event.code == KeyCode::Enter {
                    self.next_step();
                }
            }
        }
        WizardAction::None
    }

    fn handle_channels_key(&mut self, event: KeyEvent) -> WizardAction {
        // Extra item at the bottom: "Continue" (index == channel count)
        let count = self.channel_toggles.len();
        let total = count + 1; // channels + Continue button
        match event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.focused_field = self.focused_field.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.focused_field = (self.focused_field + 1).min(total.saturating_sub(1));
            }
            KeyCode::Char(' ') => {
                if self.focused_field < count {
                    let name = &self.channel_toggles[self.focused_field].0;
                    let new_val = !self.channel_toggles[self.focused_field].1;
                    tracing::debug!("[channels] toggled '{}' → {}", name, new_val);
                    self.channel_toggles[self.focused_field].1 = new_val;
                }
            }
            KeyCode::Enter => {
                if self.focused_field >= count {
                    // "Continue" button — advance past channels
                    tracing::debug!("[channels] Continue pressed, advancing to Gateway");
                    self.step = OnboardingStep::Gateway;
                } else if self.focused_field < count && self.channel_toggles[self.focused_field].1 {
                    // Enter on an enabled channel — open its setup screen
                    let idx = self.focused_field;
                    tracing::debug!("[channels] Enter on enabled channel idx={}", idx);
                    match idx {
                        0 => {
                            self.step = OnboardingStep::TelegramSetup;
                            self.telegram_field = TelegramField::BotToken;
                            self.channel_test_status = ChannelTestStatus::Idle;
                            self.detect_existing_telegram_token();
                            self.detect_existing_telegram_user_id();
                        }
                        1 => {
                            self.step = OnboardingStep::DiscordSetup;
                            self.discord_field = DiscordField::BotToken;
                            self.channel_test_status = ChannelTestStatus::Idle;
                            self.detect_existing_discord_token();
                            self.detect_existing_discord_channel_id();
                            self.detect_existing_discord_allowed_list();
                        }
                        2 => {
                            self.step = OnboardingStep::WhatsAppSetup;
                            self.whatsapp_field = WhatsAppField::Connection;
                            self.reset_whatsapp_state();
                            self.detect_existing_whatsapp_phone();
                        }
                        3 => {
                            self.step = OnboardingStep::SlackSetup;
                            self.slack_field = SlackField::BotToken;
                            self.channel_test_status = ChannelTestStatus::Idle;
                            self.detect_existing_slack_tokens();
                            self.detect_existing_slack_channel_id();
                            self.detect_existing_slack_allowed_list();
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Tab => {
                // Tab also advances past channels
                self.step = OnboardingStep::Gateway;
            }
            _ => {}
        }
        WizardAction::None
    }

    /// Check if Telegram channel is enabled (index 0 in channel_toggles)
    fn is_telegram_enabled(&self) -> bool {
        self.channel_toggles.first().is_some_and(|t| t.1)
    }

    /// Check if Discord channel is enabled (index 1 in channel_toggles)
    fn is_discord_enabled(&self) -> bool {
        self.channel_toggles.get(1).is_some_and(|t| t.1)
    }

    /// Check if WhatsApp channel is enabled (index 2 in channel_toggles)
    fn is_whatsapp_enabled(&self) -> bool {
        self.channel_toggles.get(2).is_some_and(|t| t.1)
    }

    /// Check if Slack channel is enabled (index 3 in channel_toggles)
    fn is_slack_enabled(&self) -> bool {
        self.channel_toggles.get(3).is_some_and(|t| t.1)
    }

    /// Detect existing Discord bot token from keys.toml
    fn detect_existing_discord_token(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && config
                .channels
                .discord
                .token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
        {
            self.discord_token_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if discord token holds a pre-existing value
    pub fn has_existing_discord_token(&self) -> bool {
        self.discord_token_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Discord channel ID from config.toml
    fn detect_existing_discord_channel_id(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.discord.allowed_channels.is_empty()
        {
            self.discord_channel_id_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if discord channel ID holds a pre-existing value
    pub fn has_existing_discord_channel_id(&self) -> bool {
        self.discord_channel_id_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Discord allowed users from config.toml
    fn detect_existing_discord_allowed_list(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.discord.allowed_users.is_empty()
        {
            self.discord_allowed_list_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if Discord allowed list holds a pre-existing value
    pub fn has_existing_discord_allowed_list(&self) -> bool {
        self.discord_allowed_list_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Slack tokens from keys.toml
    fn detect_existing_slack_tokens(&mut self) {
        if let Ok(config) = crate::config::Config::load() {
            if config
                .channels
                .slack
                .token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
            {
                self.slack_bot_token_input = EXISTING_KEY_SENTINEL.to_string();
            }
            if config
                .channels
                .slack
                .app_token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
            {
                self.slack_app_token_input = EXISTING_KEY_SENTINEL.to_string();
            }
        }
    }

    /// Check if slack bot token holds a pre-existing value
    pub fn has_existing_slack_bot_token(&self) -> bool {
        self.slack_bot_token_input == EXISTING_KEY_SENTINEL
    }

    /// Check if slack app token holds a pre-existing value
    pub fn has_existing_slack_app_token(&self) -> bool {
        self.slack_app_token_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Slack channel ID from config.toml
    fn detect_existing_slack_channel_id(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.slack.allowed_channels.is_empty()
        {
            self.slack_channel_id_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if slack channel ID holds a pre-existing value
    pub fn has_existing_slack_channel_id(&self) -> bool {
        self.slack_channel_id_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Slack allowed IDs from config.toml
    fn detect_existing_slack_allowed_list(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.slack.allowed_ids.is_empty()
        {
            self.slack_allowed_list_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if Slack allowed list holds a pre-existing value
    pub fn has_existing_slack_allowed_list(&self) -> bool {
        self.slack_allowed_list_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Telegram bot token from keys.toml
    fn detect_existing_telegram_token(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && config
                .channels
                .telegram
                .token
                .as_ref()
                .is_some_and(|t| !t.is_empty())
        {
            self.telegram_token_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if telegram token holds a pre-existing value
    pub fn has_existing_telegram_token(&self) -> bool {
        self.telegram_token_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Telegram user ID from config.toml
    fn detect_existing_telegram_user_id(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.telegram.allowed_users.is_empty()
        {
            self.telegram_user_id_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if telegram user ID holds a pre-existing value
    pub fn has_existing_telegram_user_id(&self) -> bool {
        self.telegram_user_id_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing WhatsApp allowed phones from config.toml
    fn detect_existing_whatsapp_phone(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && !config.channels.whatsapp.allowed_phones.is_empty()
        {
            self.whatsapp_phone_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if WhatsApp phone holds a pre-existing value
    pub fn has_existing_whatsapp_phone(&self) -> bool {
        self.whatsapp_phone_input == EXISTING_KEY_SENTINEL
    }

    /// Detect existing Groq API key from keys.toml
    fn detect_existing_groq_key(&mut self) {
        if let Ok(config) = crate::config::Config::load()
            && config
                .providers
                .stt
                .as_ref()
                .and_then(|s| s.groq.as_ref())
                .and_then(|p| p.api_key.as_ref())
                .is_some_and(|k| !k.is_empty())
        {
            self.groq_api_key_input = EXISTING_KEY_SENTINEL.to_string();
        }
    }

    /// Check if groq key holds a pre-existing value
    pub fn has_existing_groq_key(&self) -> bool {
        self.groq_api_key_input == EXISTING_KEY_SENTINEL
    }

    fn handle_telegram_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        // Handle test status interactions first
        match &self.channel_test_status {
            ChannelTestStatus::Success => {
                if event.code == KeyCode::Enter {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    self.next_step();
                    return WizardAction::None;
                }
            }
            ChannelTestStatus::Failed(_) => {
                if event.code == KeyCode::Enter {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    return WizardAction::TestTelegram;
                }
                if matches!(event.code, KeyCode::Char('s') | KeyCode::Char('S')) {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    self.next_step();
                    return WizardAction::None;
                }
            }
            ChannelTestStatus::Testing => return WizardAction::None,
            ChannelTestStatus::Idle => {}
        }

        match self.telegram_field {
            TelegramField::BotToken => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_telegram_token() {
                        self.telegram_token_input.clear();
                    }
                    self.telegram_token_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_telegram_token() {
                        self.telegram_token_input.clear();
                    } else {
                        self.telegram_token_input.pop();
                    }
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.telegram_field = TelegramField::UserID;
                }
                _ => {}
            },
            TelegramField::UserID => match event.code {
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    if self.has_existing_telegram_user_id() {
                        self.telegram_user_id_input.clear();
                    }
                    self.telegram_user_id_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_telegram_user_id() {
                        self.telegram_user_id_input.clear();
                    } else {
                        self.telegram_user_id_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.telegram_field = TelegramField::BotToken;
                }
                KeyCode::Enter => {
                    // If both token and user ID are provided, test the connection
                    let has_token = !self.telegram_token_input.is_empty();
                    let has_user_id = !self.telegram_user_id_input.is_empty();
                    if has_token && has_user_id {
                        return WizardAction::TestTelegram;
                    }
                    self.next_step();
                }
                _ => {}
            },
        }
        WizardAction::None
    }

    fn handle_discord_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        // Handle test status interactions first
        match &self.channel_test_status {
            ChannelTestStatus::Success => {
                if event.code == KeyCode::Enter {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    self.next_step();
                    return WizardAction::None;
                }
            }
            ChannelTestStatus::Failed(_) => {
                if event.code == KeyCode::Enter {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    return WizardAction::TestDiscord;
                }
                if matches!(event.code, KeyCode::Char('s') | KeyCode::Char('S')) {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    self.next_step();
                    return WizardAction::None;
                }
            }
            ChannelTestStatus::Testing => return WizardAction::None,
            ChannelTestStatus::Idle => {}
        }

        match self.discord_field {
            DiscordField::BotToken => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_discord_token() {
                        self.discord_token_input.clear();
                    }
                    self.discord_token_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_discord_token() {
                        self.discord_token_input.clear();
                    } else {
                        self.discord_token_input.pop();
                    }
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.discord_field = DiscordField::ChannelID;
                }
                _ => {}
            },
            DiscordField::ChannelID => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_discord_channel_id() {
                        self.discord_channel_id_input.clear();
                    }
                    self.discord_channel_id_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_discord_channel_id() {
                        self.discord_channel_id_input.clear();
                    } else {
                        self.discord_channel_id_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.discord_field = DiscordField::BotToken;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.discord_field = DiscordField::AllowedList;
                }
                _ => {}
            },
            DiscordField::AllowedList => match event.code {
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    if self.has_existing_discord_allowed_list() {
                        self.discord_allowed_list_input.clear();
                    }
                    self.discord_allowed_list_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_discord_allowed_list() {
                        self.discord_allowed_list_input.clear();
                    } else {
                        self.discord_allowed_list_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.discord_field = DiscordField::ChannelID;
                }
                KeyCode::Enter => {
                    let has_token = !self.discord_token_input.is_empty();
                    let has_channel = !self.discord_channel_id_input.is_empty();
                    if has_token && has_channel {
                        return WizardAction::TestDiscord;
                    }
                    self.next_step();
                }
                _ => {}
            },
        }
        WizardAction::None
    }

    fn handle_whatsapp_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.whatsapp_field {
            WhatsAppField::Connection => match event.code {
                KeyCode::Enter => {
                    if self.whatsapp_connected {
                        // Connected — move to phone field
                        self.whatsapp_field = WhatsAppField::PhoneAllowlist;
                        WizardAction::None
                    } else if !self.whatsapp_connecting {
                        // Start connection
                        self.whatsapp_connecting = true;
                        self.whatsapp_error = None;
                        WizardAction::WhatsAppConnect
                    } else {
                        WizardAction::None // already connecting, wait
                    }
                }
                KeyCode::Tab => {
                    self.whatsapp_field = WhatsAppField::PhoneAllowlist;
                    WizardAction::None
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    // Skip — advance without connecting
                    self.next_step();
                    WizardAction::None
                }
                _ => WizardAction::None,
            },
            WhatsAppField::PhoneAllowlist => match event.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '+' || c == '-' || c == ' ' => {
                    if self.has_existing_whatsapp_phone() {
                        self.whatsapp_phone_input.clear();
                    }
                    self.whatsapp_phone_input.push(c);
                    WizardAction::None
                }
                KeyCode::Backspace => {
                    if self.has_existing_whatsapp_phone() {
                        self.whatsapp_phone_input.clear();
                    } else {
                        self.whatsapp_phone_input.pop();
                    }
                    WizardAction::None
                }
                KeyCode::BackTab => {
                    self.whatsapp_field = WhatsAppField::Connection;
                    WizardAction::None
                }
                KeyCode::Enter => {
                    self.next_step();
                    WizardAction::None
                }
                _ => WizardAction::None,
            },
        }
    }

    /// Reset WhatsApp pairing state (for entering/re-entering the setup step)
    fn reset_whatsapp_state(&mut self) {
        self.whatsapp_qr_text = None;
        self.whatsapp_connecting = false;
        self.whatsapp_connected = false;
        self.whatsapp_error = None;
    }

    /// Called by app when a QR code is received from the pairing flow
    pub fn set_whatsapp_qr(&mut self, qr_data: &str) {
        self.whatsapp_qr_text = crate::brain::tools::whatsapp_connect::render_qr_unicode(qr_data);
        self.whatsapp_connecting = true;
    }

    /// Called by app when WhatsApp is successfully paired
    pub fn set_whatsapp_connected(&mut self) {
        self.whatsapp_connected = true;
        self.whatsapp_connecting = false;
    }

    /// Called by app when WhatsApp connection fails
    pub fn set_whatsapp_error(&mut self, err: String) {
        self.whatsapp_error = Some(err);
        self.whatsapp_connecting = false;
    }

    fn handle_slack_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        // Handle test status interactions first
        match &self.channel_test_status {
            ChannelTestStatus::Success => {
                if event.code == KeyCode::Enter {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    self.next_step();
                    return WizardAction::None;
                }
            }
            ChannelTestStatus::Failed(_) => {
                if event.code == KeyCode::Enter {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    return WizardAction::TestSlack;
                }
                if matches!(event.code, KeyCode::Char('s') | KeyCode::Char('S')) {
                    self.channel_test_status = ChannelTestStatus::Idle;
                    self.next_step();
                    return WizardAction::None;
                }
            }
            ChannelTestStatus::Testing => return WizardAction::None,
            ChannelTestStatus::Idle => {}
        }

        match self.slack_field {
            SlackField::BotToken => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_slack_bot_token() {
                        self.slack_bot_token_input.clear();
                    }
                    self.slack_bot_token_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_slack_bot_token() {
                        self.slack_bot_token_input.clear();
                    } else {
                        self.slack_bot_token_input.pop();
                    }
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.slack_field = SlackField::AppToken;
                }
                _ => {}
            },
            SlackField::AppToken => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_slack_app_token() {
                        self.slack_app_token_input.clear();
                    }
                    self.slack_app_token_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_slack_app_token() {
                        self.slack_app_token_input.clear();
                    } else {
                        self.slack_app_token_input.pop();
                    }
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.slack_field = SlackField::ChannelID;
                }
                KeyCode::BackTab => {
                    self.slack_field = SlackField::BotToken;
                }
                _ => {}
            },
            SlackField::ChannelID => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_slack_channel_id() {
                        self.slack_channel_id_input.clear();
                    }
                    self.slack_channel_id_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_slack_channel_id() {
                        self.slack_channel_id_input.clear();
                    } else {
                        self.slack_channel_id_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.slack_field = SlackField::AppToken;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.slack_field = SlackField::AllowedList;
                }
                _ => {}
            },
            SlackField::AllowedList => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_slack_allowed_list() {
                        self.slack_allowed_list_input.clear();
                    }
                    self.slack_allowed_list_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_slack_allowed_list() {
                        self.slack_allowed_list_input.clear();
                    } else {
                        self.slack_allowed_list_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.slack_field = SlackField::ChannelID;
                }
                KeyCode::Enter => {
                    let has_token = !self.slack_bot_token_input.is_empty();
                    let has_channel = !self.slack_channel_id_input.is_empty();
                    if has_token && has_channel {
                        return WizardAction::TestSlack;
                    }
                    self.next_step();
                }
                _ => {}
            },
        }
        WizardAction::None
    }

    fn handle_voice_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.voice_field {
            VoiceField::GroqApiKey => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_groq_key() {
                        self.groq_api_key_input.clear();
                    }
                    self.groq_api_key_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_groq_key() {
                        self.groq_api_key_input.clear();
                    } else {
                        self.groq_api_key_input.pop();
                    }
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.voice_field = VoiceField::TtsToggle;
                }
                _ => {}
            },
            VoiceField::TtsToggle => match event.code {
                KeyCode::Char(' ') | KeyCode::Up | KeyCode::Down => {
                    self.tts_enabled = !self.tts_enabled;
                }
                KeyCode::BackTab => {
                    self.voice_field = VoiceField::GroqApiKey;
                }
                KeyCode::Enter => {
                    self.next_step();
                }
                _ => {}
            },
        }
        WizardAction::None
    }

    fn handle_daemon_key(&mut self, event: KeyEvent) -> WizardAction {
        match event.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Char(' ') => {
                self.install_daemon = !self.install_daemon;
            }
            KeyCode::Enter => {
                self.next_step();
            }
            _ => {}
        }
        WizardAction::None
    }

    fn handle_health_check_key(&mut self, event: KeyEvent) -> WizardAction {
        match event.code {
            KeyCode::Enter => {
                if self.health_complete {
                    self.next_step();
                    return WizardAction::None;
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                // Re-run health checks
                self.start_health_check();
            }
            _ => {}
        }
        WizardAction::None
    }

    fn handle_brain_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        // Don't accept input while generating
        if self.brain_generating {
            return WizardAction::None;
        }

        // If already generated or errored, Enter advances
        if self.brain_generated || self.brain_error.is_some() {
            if event.code == KeyCode::Enter {
                self.next_step();
                return WizardAction::Complete;
            }
            return WizardAction::None;
        }

        match event.code {
            KeyCode::Esc => {
                // Esc always skips
                self.step = OnboardingStep::Complete;
                return WizardAction::Complete;
            }
            KeyCode::Tab => {
                self.brain_field = match self.brain_field {
                    BrainField::AboutMe => BrainField::AboutAgent,
                    BrainField::AboutAgent => BrainField::AboutMe,
                };
            }
            KeyCode::BackTab => {
                self.brain_field = match self.brain_field {
                    BrainField::AboutMe => BrainField::AboutAgent,
                    BrainField::AboutAgent => BrainField::AboutMe,
                };
            }
            KeyCode::Enter => {
                if self.brain_field == BrainField::AboutAgent {
                    if self.about_me.is_empty() && self.about_opencrabs.is_empty() {
                        // Nothing to work with — skip straight to Complete
                        self.step = OnboardingStep::Complete;
                        return WizardAction::Complete;
                    }
                    // If inputs unchanged from loaded values, skip without regenerating
                    if !self.brain_inputs_changed() && !self.original_about_me.is_empty() {
                        self.step = OnboardingStep::Complete;
                        return WizardAction::Complete;
                    }
                    // Inputs changed or new — trigger generation
                    return WizardAction::GenerateBrain;
                }
                // Enter on AboutMe moves to AboutAgent
                self.brain_field = BrainField::AboutAgent;
            }
            KeyCode::Char(c) => {
                self.active_brain_field_mut().push(c);
            }
            KeyCode::Backspace => {
                self.active_brain_field_mut().pop();
            }
            _ => {}
        }
        WizardAction::None
    }

    /// Get mutable reference to the currently focused brain text area
    fn active_brain_field_mut(&mut self) -> &mut String {
        match self.brain_field {
            BrainField::AboutMe => &mut self.about_me,
            BrainField::AboutAgent => &mut self.about_opencrabs,
        }
    }

    /// Whether brain inputs have been modified since loading from file
    fn brain_inputs_changed(&self) -> bool {
        self.about_me != self.original_about_me
            || self.about_opencrabs != self.original_about_opencrabs
    }

    /// Truncate file content to first N chars for preview in the wizard
    fn truncate_preview(content: &str, max_chars: usize) -> String {
        let trimmed = content.trim();
        if trimmed.len() <= max_chars {
            trimmed.to_string()
        } else {
            let truncated = &trimmed[..trimmed.floor_char_boundary(max_chars)];
            format!("{}...", truncated.trim_end())
        }
    }

    /// Build the prompt sent to the AI to generate personalized brain files.
    /// Uses existing workspace files if available, falls back to static templates.
    pub fn build_brain_prompt(&self) -> String {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let workspace = std::path::Path::new(&self.workspace_path);

        // Read current brain files from workspace, fall back to static templates
        let soul_template_static = include_str!("../docs/reference/templates/SOUL.md");
        let identity_template_static = include_str!("../docs/reference/templates/IDENTITY.md");
        let user_template_static = include_str!("../docs/reference/templates/USER.md");
        let agents_template_static = include_str!("../docs/reference/templates/AGENTS.md");
        let tools_template_static = include_str!("../docs/reference/templates/TOOLS.md");
        let memory_template_static = include_str!("../docs/reference/templates/MEMORY.md");

        let soul_template = std::fs::read_to_string(workspace.join("SOUL.md"))
            .unwrap_or_else(|_| soul_template_static.to_string());
        let identity_template = std::fs::read_to_string(workspace.join("IDENTITY.md"))
            .unwrap_or_else(|_| identity_template_static.to_string());
        let user_template = std::fs::read_to_string(workspace.join("USER.md"))
            .unwrap_or_else(|_| user_template_static.to_string());
        let agents_template = std::fs::read_to_string(workspace.join("AGENTS.md"))
            .unwrap_or_else(|_| agents_template_static.to_string());
        let tools_template = std::fs::read_to_string(workspace.join("TOOLS.md"))
            .unwrap_or_else(|_| tools_template_static.to_string());
        let memory_template = std::fs::read_to_string(workspace.join("MEMORY.md"))
            .unwrap_or_else(|_| memory_template_static.to_string());

        format!(
            r#"You are setting up a personal AI agent's brain — its entire workspace of markdown files that define who it is, who its human is, and how it operates.

The user dumped two blocks of info. One about themselves (name, role, links, projects, whatever they shared). One about how they want their agent to be (personality, vibe, behavior). Use EVERYTHING they gave you to personalize ALL six template files below.

=== ABOUT THE USER ===
{about_me}

=== ABOUT THE AGENT ===
{about_opencrabs}

=== TODAY'S DATE ===
{date}

Below are the 6 template files. Replace ALL <placeholder> tags and HTML comments with real values based on what the user provided. Keep the exact markdown structure. Fill what you can from the user's info, leave sensible defaults for anything not provided. Don't invent facts — if the user didn't mention something, use a reasonable placeholder like "TBD" or remove that line.

===TEMPLATE: SOUL.md===
{soul}

===TEMPLATE: IDENTITY.md===
{identity}

===TEMPLATE: USER.md===
{user}

===TEMPLATE: AGENTS.md===
{agents}

===TEMPLATE: TOOLS.md===
{tools}

===TEMPLATE: MEMORY.md===
{memory}

Respond with EXACTLY six sections using these delimiters. No extra text before the first delimiter or after the last section:
---SOUL---
(generated SOUL.md content)
---IDENTITY---
(generated IDENTITY.md content)
---USER---
(generated USER.md content)
---AGENTS---
(generated AGENTS.md content)
---TOOLS---
(generated TOOLS.md content)
---MEMORY---
(generated MEMORY.md content)"#,
            about_me = if self.about_me.is_empty() {
                "Not provided"
            } else {
                &self.about_me
            },
            about_opencrabs = if self.about_opencrabs.is_empty() {
                "Not provided"
            } else {
                &self.about_opencrabs
            },
            date = today,
            soul = soul_template,
            identity = identity_template,
            user = user_template,
            agents = agents_template,
            tools = tools_template,
            memory = memory_template,
        )
    }

    /// Store the generated brain content from the AI response
    pub fn apply_generated_brain(&mut self, response: &str) {
        // Parse the response into six sections using delimiters
        let delimiters = [
            "---SOUL---",
            "---IDENTITY---",
            "---USER---",
            "---AGENTS---",
            "---TOOLS---",
            "---MEMORY---",
        ];

        // Find all delimiter positions
        let positions: Vec<Option<usize>> = delimiters.iter().map(|d| response.find(d)).collect();

        // Need at least SOUL, IDENTITY, USER to consider it a success
        if positions[0].is_none() || positions[1].is_none() || positions[2].is_none() {
            self.brain_error = Some("Couldn't parse AI response — using defaults".to_string());
            self.brain_generating = false;
            return;
        }

        // Extract content between delimiters
        // Build ordered list of (delimiter_index, position) sorted by position
        let mut ordered: Vec<(usize, usize)> = positions
            .iter()
            .enumerate()
            .filter_map(|(i, pos)| pos.map(|p| (i, p)))
            .collect();
        ordered.sort_by_key(|(_, pos)| *pos);

        for (idx, &(delim_idx, pos)) in ordered.iter().enumerate() {
            let start = pos + delimiters[delim_idx].len();
            let end = if idx + 1 < ordered.len() {
                ordered[idx + 1].1
            } else {
                response.len()
            };
            let content = response[start..end].trim();

            if !content.is_empty() {
                match delim_idx {
                    0 => self.generated_soul = Some(content.to_string()),
                    1 => self.generated_identity = Some(content.to_string()),
                    2 => self.generated_user = Some(content.to_string()),
                    3 => self.generated_agents = Some(content.to_string()),
                    4 => self.generated_tools = Some(content.to_string()),
                    5 => self.generated_memory = Some(content.to_string()),
                    _ => {}
                }
            }
        }

        self.brain_generated = true;
        self.brain_generating = false;
    }

    /// Apply wizard configuration — creates config.toml, stores API key, seeds workspace
    /// Merges with existing config to preserve settings not modified in wizard
    pub fn apply_config(&self) -> Result<(), String> {
        // Groq key for STT/TTS
        let groq_key = if !self.groq_api_key_input.is_empty() && !self.has_existing_groq_key() {
            Some(self.groq_api_key_input.clone())
        } else {
            None
        };

        // Write config.toml via merge (write_key) — never overwrite entire file
        // Disable all providers first, then enable selected one
        let all_provider_sections = [
            "providers.anthropic",
            "providers.openai",
            "providers.gemini",
            "providers.openrouter",
            "providers.minimax",
        ];
        for section in &all_provider_sections {
            let _ = Config::write_key(section, "enabled", "false");
        }
        // Disable all custom providers
        if let Ok(config) = Config::load()
            && let Some(customs) = &config.providers.custom
        {
            for name in customs.keys() {
                let section = format!("providers.custom.{}", name);
                let _ = Config::write_key(&section, "enabled", "false");
            }
        }

        // Enable + configure the selected provider
        let custom_section;
        let section = match self.selected_provider {
            0 => "providers.anthropic",
            1 => "providers.openai",
            2 => "providers.gemini",
            3 => "providers.openrouter",
            4 => "providers.minimax",
            5 => {
                custom_section = format!("providers.custom.{}", self.custom_provider_name);
                &custom_section
            }
            _ => {
                custom_section = format!("providers.custom.{}", self.custom_provider_name);
                &custom_section
            }
        };
        let _ = Config::write_key(section, "enabled", "true");
        let model = self.selected_model_name().to_string();
        if !model.is_empty() {
            let _ = Config::write_key(section, "default_model", &model);
        }

        // Write base_url for providers that need it
        match self.selected_provider {
            3 => {
                let _ = Config::write_key(
                    section,
                    "base_url",
                    "https://openrouter.ai/api/v1/chat/completions",
                );
            }
            4 => {
                let _ = Config::write_key(section, "base_url", "https://api.minimax.io/v1");
            }
            5 => {
                if !self.custom_base_url.is_empty() {
                    let _ = Config::write_key(section, "base_url", &self.custom_base_url);
                }
                if !self.custom_model.is_empty() {
                    let _ = Config::write_key(section, "default_model", &self.custom_model);
                }
            }
            _ => {}
        }

        // Write models array for providers that have static model lists
        if !self.config_models.is_empty() && matches!(self.selected_provider, 4 | 5) {
            let _ = Config::write_array(section, "models", &self.config_models);
        }

        // Gateway config
        let _ = Config::write_key("gateway", "port", &self.gateway_port);
        let _ = Config::write_key("gateway", "bind", &self.gateway_bind);
        let _ = Config::write_key(
            "gateway",
            "auth_mode",
            if self.gateway_auth == 0 {
                "token"
            } else {
                "none"
            },
        );

        // Channel enabled flags (from channel_toggles: 0=Telegram, 1=Discord, 2=WhatsApp, 3=Slack)
        let _ = Config::write_key(
            "channels.telegram",
            "enabled",
            &self.is_telegram_enabled().to_string(),
        );
        let _ = Config::write_key(
            "channels.discord",
            "enabled",
            &self.is_discord_enabled().to_string(),
        );
        let _ = Config::write_key(
            "channels.whatsapp",
            "enabled",
            &self.channel_toggles.get(2).is_some_and(|t| t.1).to_string(),
        );
        let _ = Config::write_key(
            "channels.slack",
            "enabled",
            &self.is_slack_enabled().to_string(),
        );

        // Voice config
        let groq_key_exists = !self.groq_api_key_input.is_empty() || self.has_existing_groq_key();
        let _ = Config::write_key("voice", "stt_enabled", &groq_key_exists.to_string());
        let _ = Config::write_key("voice", "tts_enabled", &self.tts_enabled.to_string());

        // STT provider
        if !self.groq_api_key_input.is_empty() || self.has_existing_groq_key() {
            let _ = Config::write_key("providers.stt.groq", "enabled", "true");
            let _ = Config::write_key(
                "providers.stt.groq",
                "default_model",
                "whisper-large-v3-turbo",
            );
        }

        // TTS provider
        if self.tts_enabled && groq_key_exists {
            let _ = Config::write_key("providers.tts.openai", "enabled", "true");
            let _ = Config::write_key("providers.tts.openai", "default_model", "gpt-4o-mini-tts");
        }

        // Save API key to keys.toml via merge — never overwrite
        if !self.has_existing_key()
            && !self.api_key_input.is_empty()
            && let Err(e) = crate::config::write_secret_key(section, "api_key", &self.api_key_input)
        {
            tracing::warn!("Failed to save API key to keys.toml: {}", e);
        }

        // Save STT/TTS keys to keys.toml
        if let Some(ref groq_key) = groq_key
            && let Err(e) =
                crate::config::write_secret_key("providers.stt.groq", "api_key", groq_key)
        {
            tracing::warn!("Failed to save Groq key to keys.toml: {}", e);
        }
        if self.tts_enabled
            && let Some(ref groq_key) = groq_key
            && let Err(e) =
                crate::config::write_secret_key("providers.tts.openai", "api_key", groq_key)
        {
            tracing::warn!("Failed to save TTS key to keys.toml: {}", e);
        }

        // Persist channel tokens to keys.toml (if new)
        if !self.telegram_token_input.is_empty()
            && !self.has_existing_telegram_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.telegram",
                "token",
                &self.telegram_token_input,
            )
        {
            tracing::warn!("Failed to save Telegram token to keys.toml: {}", e);
        }
        if !self.discord_token_input.is_empty()
            && !self.has_existing_discord_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.discord",
                "token",
                &self.discord_token_input,
            )
        {
            tracing::warn!("Failed to save Discord token to keys.toml: {}", e);
        }
        if !self.slack_bot_token_input.is_empty()
            && !self.has_existing_slack_bot_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.slack",
                "token",
                &self.slack_bot_token_input,
            )
        {
            tracing::warn!("Failed to save Slack bot token to keys.toml: {}", e);
        }
        if !self.slack_app_token_input.is_empty()
            && !self.has_existing_slack_app_token()
            && let Err(e) = crate::config::write_secret_key(
                "channels.slack",
                "app_token",
                &self.slack_app_token_input,
            )
        {
            tracing::warn!("Failed to save Slack app token to keys.toml: {}", e);
        }

        // Persist channel IDs/user IDs to config.toml (if new)
        if !self.telegram_user_id_input.is_empty()
            && !self.has_existing_telegram_user_id()
            && let Ok(uid) = self.telegram_user_id_input.parse::<i64>()
        {
            let _ = Config::write_i64_array("channels.telegram", "allowed_users", &[uid]);
        }
        if !self.discord_channel_id_input.is_empty() && !self.has_existing_discord_channel_id() {
            let _ = Config::write_array(
                "channels.discord",
                "allowed_channels",
                std::slice::from_ref(&self.discord_channel_id_input),
            );
        }
        if !self.slack_channel_id_input.is_empty() && !self.has_existing_slack_channel_id() {
            let _ = Config::write_array(
                "channels.slack",
                "allowed_channels",
                std::slice::from_ref(&self.slack_channel_id_input),
            );
        }
        if !self.discord_allowed_list_input.is_empty()
            && !self.has_existing_discord_allowed_list()
            && let Ok(uid) = self.discord_allowed_list_input.parse::<i64>()
        {
            let _ = Config::write_i64_array("channels.discord", "allowed_users", &[uid]);
        }
        if !self.slack_allowed_list_input.is_empty() && !self.has_existing_slack_allowed_list() {
            let _ = Config::write_array(
                "channels.slack",
                "allowed_ids",
                std::slice::from_ref(&self.slack_allowed_list_input),
            );
        }
        if !self.whatsapp_phone_input.is_empty() && !self.has_existing_whatsapp_phone() {
            let _ = Config::write_array(
                "channels.whatsapp",
                "allowed_phones",
                std::slice::from_ref(&self.whatsapp_phone_input),
            );
        }

        // Seed workspace templates (use AI-generated content when available)
        if self.seed_templates {
            let workspace = PathBuf::from(&self.workspace_path);
            std::fs::create_dir_all(&workspace)
                .map_err(|e| format!("Failed to create workspace: {}", e))?;

            for (filename, content) in TEMPLATE_FILES {
                let file_path = workspace.join(filename);
                // Use AI-generated content when available, static template as fallback
                let generated = match *filename {
                    "SOUL.md" => self.generated_soul.as_deref(),
                    "IDENTITY.md" => self.generated_identity.as_deref(),
                    "USER.md" => self.generated_user.as_deref(),
                    "AGENTS.md" => self.generated_agents.as_deref(),
                    "TOOLS.md" => self.generated_tools.as_deref(),
                    "MEMORY.md" => self.generated_memory.as_deref(),
                    _ => None,
                };
                // Write if: AI-generated (always overwrite) or file doesn't exist (seed template)
                if generated.is_some() || !file_path.exists() {
                    let final_content = generated.unwrap_or(content);
                    std::fs::write(&file_path, final_content)
                        .map_err(|e| format!("Failed to write {}: {}", filename, e))?;
                }
            }
        }

        // Install daemon if requested
        if self.install_daemon
            && let Err(e) = install_daemon_service()
        {
            tracing::warn!("Failed to install daemon: {}", e);
            // Non-fatal — don't block onboarding completion
        }

        Ok(())
    }
}

/// What the app should do after handling a wizard key event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardAction {
    /// Nothing special
    None,
    /// User cancelled the wizard (Esc from step 1)
    Cancel,
    /// Wizard completed successfully
    Complete,
    /// Trigger async AI generation of brain files
    GenerateBrain,
    /// Trigger async model list fetch from provider API
    FetchModels,
    /// Trigger async WhatsApp QR code pairing
    WhatsAppConnect,
    /// Trigger async Telegram test message
    TestTelegram,
    /// Trigger async Discord test message
    TestDiscord,
    /// Trigger async Slack test message
    TestSlack,
}

/// First-time detection: no config file AND no API keys in environment.
/// Once config.toml is written (by onboarding or manually), this returns false forever.
/// If any API key env var is set, the user has already configured auth — skip onboarding.
/// To re-run the wizard, use `opencrabs onboard`, `--onboard` flag, or `/onboard`.
pub fn is_first_time() -> bool {
    tracing::debug!("[is_first_time] checking if first time setup needed...");

    // Check if config exists
    let config_path = crate::config::opencrabs_home().join("config.toml");
    if !config_path.exists() {
        tracing::debug!("[is_first_time] no config found, need onboarding");
        return true;
    }

    // Config exists - check if any provider is actually enabled
    let config = match crate::config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(
                "[is_first_time] failed to load config: {}, need onboarding",
                e
            );
            return true;
        }
    };

    let has_enabled_provider = config
        .providers
        .anthropic
        .as_ref()
        .is_some_and(|p| p.enabled)
        || config.providers.openai.as_ref().is_some_and(|p| p.enabled)
        || config.providers.gemini.as_ref().is_some_and(|p| p.enabled)
        || config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled)
        || config.providers.minimax.as_ref().is_some_and(|p| p.enabled)
        || config.providers.active_custom().is_some();

    tracing::debug!(
        "[is_first_time] has_enabled_provider={}, result={}",
        has_enabled_provider,
        !has_enabled_provider
    );
    !has_enabled_provider
}

/// Install the appropriate daemon service for the current platform
fn install_daemon_service() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        install_systemd_service()
    }

    #[cfg(target_os = "macos")]
    {
        install_launchagent()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("Daemon installation not supported on this platform".to_string())
    }
}

#[cfg(target_os = "linux")]
fn install_systemd_service() -> Result<(), String> {
    let service_dir = dirs::config_dir()
        .ok_or("Cannot determine config dir")?
        .parent()
        .ok_or("Cannot determine parent of config dir")?
        .join(".config")
        .join("systemd")
        .join("user");

    // Try the standard XDG path first
    let service_dir = if service_dir.exists() {
        service_dir
    } else {
        dirs::home_dir()
            .ok_or("Cannot determine home dir")?
            .join(".config")
            .join("systemd")
            .join("user")
    };

    std::fs::create_dir_all(&service_dir)
        .map_err(|e| format!("Failed to create systemd dir: {}", e))?;

    let exe_path = std::env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;

    let service_content = format!(
        r#"[Unit]
Description=OpenCrabs AI Orchestration Agent
After=network.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#,
        exe_path.display()
    );

    let service_path = service_dir.join("opencrabs.service");
    std::fs::write(&service_path, service_content)
        .map_err(|e| format!("Failed to write service file: {}", e))?;

    // Enable the service
    std::process::Command::new("systemctl")
        .args(["--user", "enable", "opencrabs"])
        .output()
        .map_err(|e| format!("Failed to enable service: {}", e))?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn install_launchagent() -> Result<(), String> {
    let agents_dir = dirs::home_dir()
        .ok_or("Cannot determine home dir")?
        .join("Library")
        .join("LaunchAgents");

    std::fs::create_dir_all(&agents_dir)
        .map_err(|e| format!("Failed to create LaunchAgents dir: {}", e))?;

    let exe_path = std::env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.opencrabs.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"#,
        exe_path.display()
    );

    let plist_path = agents_dir.join("com.opencrabs.agent.plist");
    std::fs::write(&plist_path, plist_content)
        .map_err(|e| format!("Failed to write plist: {}", e))?;

    std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("Failed to load launch agent: {}", e))?;

    Ok(())
}

/// Fetch models from provider API. No API key needed for most providers.
/// If api_key is provided, includes it (some endpoints filter by access level).
/// Returns empty vec on failure (callers fall back to static list).
pub async fn fetch_provider_models(provider_index: usize, api_key: Option<&str>) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    // Handle Minimax specially - no /models API, must use config
    if provider_index == 4 {
        // Minimax — NO /models API endpoint, must use config.models
        if let Ok(config) = crate::config::Config::load()
            && let Some(p) = &config.providers.minimax
        {
            if !p.models.is_empty() {
                return p.models.clone();
            }
            // Fall back to default_model if no models list
            if let Some(model) = &p.default_model {
                return vec![model.clone()];
            }
        }
        // Return hardcoded defaults if no config
        return vec!["MiniMax-M2.5".to_string(), "MiniMax-M2.1".to_string()];
    }

    let client = reqwest::Client::new();

    let result = match provider_index {
        0 => {
            // Anthropic — /v1/models is public
            let mut req = client
                .get("https://api.anthropic.com/v1/models")
                .header("anthropic-version", "2023-06-01");

            // Include key if available (may show more models)
            if let Some(key) = api_key {
                if key.starts_with("sk-ant-oat") {
                    req = req
                        .header("Authorization", format!("Bearer {}", key))
                        .header("anthropic-beta", "oauth-2025-04-20");
                } else if !key.is_empty() {
                    req = req.header("x-api-key", key);
                }
            }

            req.send().await
        }
        1 => {
            // OpenAI — /v1/models
            let mut req = client.get("https://api.openai.com/v1/models");
            if let Some(key) = api_key
                && !key.is_empty()
            {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            req.send().await
        }
        3 => {
            // OpenRouter — /api/v1/models
            let mut req = client.get("https://openrouter.ai/api/v1/models");
            if let Some(key) = api_key
                && !key.is_empty()
            {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            req.send().await
        }
        _ => return Vec::new(),
    };

    match result {
        Ok(resp) if resp.status().is_success() => match resp.json::<ModelsResponse>().await {
            Ok(body) => {
                let mut models: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
                models.sort();
                models
            }
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_creation() {
        let wizard = OnboardingWizard::new();
        assert_eq!(wizard.step, OnboardingStep::ModeSelect);
        assert_eq!(wizard.mode, WizardMode::QuickStart);
        assert_eq!(wizard.channel_toggles.len(), CHANNEL_NAMES.len());
    }

    #[test]
    fn test_step_navigation() {
        let mut wizard = OnboardingWizard::new();
        wizard.api_key_input = "test-key".to_string();

        assert_eq!(wizard.step, OnboardingStep::ModeSelect);
        wizard.next_step(); // ModeSelect -> Workspace
        assert_eq!(wizard.step, OnboardingStep::Workspace);
    }

    #[test]
    fn test_advanced_mode_all_steps() {
        let mut wizard = OnboardingWizard::new();
        wizard.mode = WizardMode::Advanced;
        wizard.api_key_input = "test-key".to_string();

        wizard.next_step(); // ModeSelect -> Workspace
        assert_eq!(wizard.step, OnboardingStep::Workspace);
        wizard.next_step(); // Workspace -> ProviderAuth
        assert_eq!(wizard.step, OnboardingStep::ProviderAuth);
        wizard.next_step(); // ProviderAuth -> Channels
        assert_eq!(wizard.step, OnboardingStep::Channels);
        wizard.next_step(); // Channels -> Gateway (nothing enabled)
        assert_eq!(wizard.step, OnboardingStep::Gateway);
        wizard.next_step(); // Gateway -> VoiceSetup (Advanced)
        assert_eq!(wizard.step, OnboardingStep::VoiceSetup);
        wizard.next_step(); // VoiceSetup -> Daemon
        assert_eq!(wizard.step, OnboardingStep::Daemon);
        wizard.next_step(); // Daemon -> HealthCheck
        assert_eq!(wizard.step, OnboardingStep::HealthCheck);
    }

    #[test]
    fn test_channels_telegram_goes_to_telegram_setup() {
        let mut wizard = clean_wizard();
        wizard.mode = WizardMode::Advanced;
        wizard.step = OnboardingStep::Channels;

        // Enable Telegram in channel toggles
        wizard.channel_toggles[0].1 = true;

        // Enter Telegram setup (focus on Telegram, press Enter)
        wizard.focused_field = 0;
        wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(wizard.step, OnboardingStep::TelegramSetup);

        // Complete Telegram → back to Channels
        wizard.next_step();
        assert_eq!(wizard.step, OnboardingStep::Channels);

        // Continue to Gateway
        wizard.focused_field = wizard.channel_toggles.len();
        wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(wizard.step, OnboardingStep::Gateway);
    }

    #[test]
    fn test_channels_whatsapp_skips_to_gateway() {
        let mut wizard = OnboardingWizard::new();
        wizard.mode = WizardMode::Advanced;
        wizard.api_key_input = "test-key".to_string();

        wizard.next_step(); // ModeSelect -> Workspace
        wizard.next_step(); // Workspace -> ProviderAuth
        wizard.next_step(); // ProviderAuth -> Channels

        // Enable WhatsApp only (no token sub-step)
        wizard.channel_toggles[2].1 = true;
        wizard.next_step(); // Channels -> Gateway (WhatsApp has no sub-step)
        assert_eq!(wizard.step, OnboardingStep::Gateway);
        // Verify channel_toggles WhatsApp is enabled
        assert!(wizard.channel_toggles[2].1);
    }

    #[test]
    fn test_channels_full_chain_telegram_discord_slack() {
        let mut wizard = clean_wizard();
        wizard.mode = WizardMode::Advanced;
        wizard.step = OnboardingStep::Channels;

        // Enable all three token-based channels
        wizard.channel_toggles[0].1 = true; // Telegram
        wizard.channel_toggles[1].1 = true; // Discord
        wizard.channel_toggles[3].1 = true; // Slack

        // Enter Telegram setup
        wizard.focused_field = 0;
        wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(wizard.step, OnboardingStep::TelegramSetup);

        // Complete Telegram → back to Channels
        wizard.next_step();
        assert_eq!(wizard.step, OnboardingStep::Channels);

        // Enter Discord setup
        wizard.focused_field = 1;
        wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(wizard.step, OnboardingStep::DiscordSetup);

        // Complete Discord → back to Channels
        wizard.next_step();
        assert_eq!(wizard.step, OnboardingStep::Channels);

        // Enter Slack setup
        wizard.focused_field = 3;
        wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(wizard.step, OnboardingStep::SlackSetup);

        // Complete Slack → back to Channels
        wizard.next_step();
        assert_eq!(wizard.step, OnboardingStep::Channels);

        // Continue to Gateway
        wizard.focused_field = wizard.channel_toggles.len();
        wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(wizard.step, OnboardingStep::Gateway);
    }

    #[test]
    fn test_voice_setup_defaults() {
        let wizard = OnboardingWizard::new();
        assert!(wizard.groq_api_key_input.is_empty());
        assert!(!wizard.tts_enabled);
        assert_eq!(wizard.voice_field, VoiceField::GroqApiKey);
    }

    #[test]
    fn test_step_numbers() {
        assert_eq!(OnboardingStep::ModeSelect.number(), 1);
        assert_eq!(OnboardingStep::Channels.number(), 4);
        assert_eq!(OnboardingStep::TelegramSetup.number(), 4); // sub-step of Channels
        assert_eq!(OnboardingStep::Gateway.number(), 5);
        assert_eq!(OnboardingStep::VoiceSetup.number(), 6);
        assert_eq!(OnboardingStep::HealthCheck.number(), 8);
        assert_eq!(OnboardingStep::BrainSetup.number(), 9);
        assert_eq!(OnboardingStep::total(), 9);
    }

    #[test]
    fn test_prev_step_cancel() {
        let mut wizard = OnboardingWizard::new();
        // Going back from step 1 signals cancel
        assert!(wizard.prev_step());
    }

    #[test]
    fn test_provider_auth_defaults() {
        let wizard = clean_wizard();
        assert_eq!(wizard.selected_provider, 0);
        assert_eq!(wizard.auth_field, AuthField::Provider);
        assert!(wizard.api_key_input.is_empty());
        assert_eq!(wizard.selected_model, 0);
        // First provider is Anthropic Claude
        assert_eq!(PROVIDERS[wizard.selected_provider].name, "Anthropic Claude");
        assert!(PROVIDERS[wizard.selected_provider].help_lines.is_empty() == false);
    }

    #[test]
    fn test_channel_toggles_default_off() {
        let wizard = OnboardingWizard::new();
        assert_eq!(wizard.channel_toggles.len(), CHANNEL_NAMES.len());
        // All channels default to disabled
        for (name, enabled) in &wizard.channel_toggles {
            assert!(!enabled, "Channel {} should default to disabled", name);
        }
        // Verify all expected channels are present
        let toggle_names: Vec<&str> = wizard
            .channel_toggles
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert!(toggle_names.contains(&"Telegram"));
        assert!(toggle_names.contains(&"Discord"));
        assert!(toggle_names.contains(&"iMessage"));
    }

    /// Create a wizard with clean defaults (no config auto-detection).
    /// `OnboardingWizard::new()` loads existing config from disk, which
    /// pollutes provider/brain fields when a real config exists.
    fn clean_wizard() -> OnboardingWizard {
        let mut w = OnboardingWizard::new();
        w.selected_provider = 0;
        w.api_key_input = String::new();
        w.custom_base_url = String::new();
        w.custom_model = String::new();
        w.about_me = String::new();
        w.about_opencrabs = String::new();
        w.original_about_me = String::new();
        w.original_about_opencrabs = String::new();
        w
    }

    // ── handle_key tests ──

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::empty())
    }

    #[test]
    fn test_handle_key_mode_select_up_down() {
        let mut wizard = OnboardingWizard::new();
        assert_eq!(wizard.mode, WizardMode::QuickStart);

        wizard.handle_key(key(KeyCode::Down));
        assert_eq!(wizard.mode, WizardMode::Advanced);

        wizard.handle_key(key(KeyCode::Up));
        assert_eq!(wizard.mode, WizardMode::QuickStart);
    }

    #[test]
    fn test_handle_key_mode_select_number_keys() {
        let mut wizard = OnboardingWizard::new();

        wizard.handle_key(key(KeyCode::Char('2')));
        assert_eq!(wizard.mode, WizardMode::Advanced);

        wizard.handle_key(key(KeyCode::Char('1')));
        assert_eq!(wizard.mode, WizardMode::QuickStart);
    }

    #[test]
    fn test_handle_key_mode_select_enter_advances() {
        let mut wizard = OnboardingWizard::new();
        let action = wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(action, WizardAction::None);
        assert_eq!(wizard.step, OnboardingStep::Workspace);
    }

    #[test]
    fn test_handle_key_escape_from_step1_cancels() {
        let mut wizard = OnboardingWizard::new();
        let action = wizard.handle_key(key(KeyCode::Esc));
        assert_eq!(action, WizardAction::Cancel);
    }

    #[test]
    fn test_handle_key_escape_from_step2_goes_back() {
        let mut wizard = OnboardingWizard::new();
        wizard.handle_key(key(KeyCode::Enter)); // ModeSelect -> Workspace
        assert_eq!(wizard.step, OnboardingStep::Workspace);

        let action = wizard.handle_key(key(KeyCode::Esc));
        assert_eq!(action, WizardAction::None);
        assert_eq!(wizard.step, OnboardingStep::ModeSelect);
    }

    #[test]
    fn test_handle_key_provider_navigation() {
        let mut wizard = clean_wizard();
        wizard.step = OnboardingStep::ProviderAuth;
        wizard.auth_field = AuthField::Provider;
        assert_eq!(wizard.selected_provider, 0);

        wizard.handle_key(key(KeyCode::Down));
        assert_eq!(wizard.selected_provider, 1);

        wizard.handle_key(key(KeyCode::Up));
        assert_eq!(wizard.selected_provider, 0);

        // Can't go below 0
        wizard.handle_key(key(KeyCode::Up));
        assert_eq!(wizard.selected_provider, 0);
    }

    #[test]
    fn test_handle_key_api_key_typing() {
        let mut wizard = OnboardingWizard::new();
        wizard.step = OnboardingStep::ProviderAuth;
        wizard.auth_field = AuthField::Provider;

        // Enter to select provider -> goes to ApiKey field
        wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(wizard.auth_field, AuthField::ApiKey);

        // Type a key
        wizard.handle_key(key(KeyCode::Char('s')));
        wizard.handle_key(key(KeyCode::Char('k')));
        assert_eq!(wizard.api_key_input, "sk");

        // Backspace
        wizard.handle_key(key(KeyCode::Backspace));
        assert_eq!(wizard.api_key_input, "s");
    }

    #[test]
    fn test_handle_key_provider_auth_field_flow() {
        let mut wizard = OnboardingWizard::new();
        wizard.step = OnboardingStep::ProviderAuth;
        wizard.auth_field = AuthField::Provider;
        assert_eq!(wizard.auth_field, AuthField::Provider);

        // Enter goes to ApiKey
        wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(wizard.auth_field, AuthField::ApiKey);

        // Tab goes to Model
        wizard.handle_key(key(KeyCode::Tab));
        assert_eq!(wizard.auth_field, AuthField::Model);

        // BackTab goes back to ApiKey
        wizard.handle_key(key(KeyCode::BackTab));
        assert_eq!(wizard.auth_field, AuthField::ApiKey);

        // BackTab from ApiKey goes to Provider
        wizard.handle_key(key(KeyCode::BackTab));
        assert_eq!(wizard.auth_field, AuthField::Provider);
    }

    #[test]
    fn test_handle_key_complete_step_returns_complete() {
        let mut wizard = OnboardingWizard::new();
        wizard.step = OnboardingStep::Complete;
        let action = wizard.handle_key(key(KeyCode::Enter));
        assert_eq!(action, WizardAction::Complete);
    }

    #[test]
    fn test_quickstart_skips_channels_voice() {
        let mut wizard = OnboardingWizard::new();
        wizard.mode = WizardMode::QuickStart;
        wizard.api_key_input = "test-key".to_string();

        wizard.next_step(); // ModeSelect -> Workspace
        assert_eq!(wizard.step, OnboardingStep::Workspace);
        wizard.next_step(); // Workspace -> ProviderAuth
        assert_eq!(wizard.step, OnboardingStep::ProviderAuth);
        wizard.next_step(); // ProviderAuth -> Gateway (QuickStart skips Channels)
        assert_eq!(wizard.step, OnboardingStep::Gateway);
        wizard.next_step(); // Gateway -> Daemon (QuickStart skips Voice)
        assert_eq!(wizard.step, OnboardingStep::Daemon);
    }

    #[test]
    fn test_provider_auth_validation_empty_key() {
        let mut wizard = clean_wizard();
        wizard.step = OnboardingStep::ProviderAuth;
        // api_key_input is empty
        wizard.next_step();
        // Should stay on ProviderAuth with error
        assert_eq!(wizard.step, OnboardingStep::ProviderAuth);
        assert!(wizard.error_message.is_some());
        assert!(
            wizard
                .error_message
                .as_ref()
                .map_or(false, |m| m.contains("required"))
        );
    }

    #[test]
    fn test_model_selection() {
        let mut wizard = OnboardingWizard::new();
        wizard.step = OnboardingStep::ProviderAuth;
        wizard.auth_field = AuthField::Model;
        // Set up config models for selection testing
        wizard.config_models = vec!["model-a".into(), "model-b".into(), "model-c".into()];

        assert_eq!(wizard.selected_model, 0);
        wizard.handle_key(key(KeyCode::Down));
        assert_eq!(wizard.selected_model, 1);
        wizard.handle_key(key(KeyCode::Down));
        assert_eq!(wizard.selected_model, 2);
        // Should clamp to max
        for _ in 0..20 {
            wizard.handle_key(key(KeyCode::Down));
        }
        // Provider selection wraps or stays within bounds
        assert!(wizard.selected_provider < PROVIDERS.len());
    }

    #[test]
    fn test_workspace_path_default() {
        let wizard = OnboardingWizard::new();
        // Should have a default workspace path
        assert!(!wizard.workspace_path.is_empty());
    }

    #[test]
    fn test_health_check_initial_state() {
        let wizard = OnboardingWizard::new();
        // health_results starts empty (populated on start_health_check)
        assert!(wizard.health_results.is_empty());
    }

    #[test]
    fn test_brain_setup_defaults() {
        let wizard = clean_wizard();
        assert!(wizard.about_me.is_empty());
        assert!(wizard.about_opencrabs.is_empty());
        assert_eq!(wizard.brain_field, BrainField::AboutMe);
    }

    // --- Model fetching helpers ---

    #[test]
    fn test_openrouter_provider_index() {
        // OpenRouter is index 3, Custom is last
        assert_eq!(PROVIDERS[3].name, "OpenRouter");
        assert_eq!(PROVIDERS.last().unwrap().name, "Custom OpenAI-Compatible");
    }

    #[test]
    fn test_model_count_uses_fetched_when_available() {
        let mut wizard = OnboardingWizard::new();
        // Static fallback is empty - models fetched from API
        assert_eq!(wizard.model_count(), 0);

        // After fetching
        wizard.fetched_models = vec![
            "model-a".into(),
            "model-b".into(),
            "model-c".into(),
            "model-d".into(),
        ];
        assert_eq!(wizard.model_count(), 4);
    }

    #[test]
    fn test_selected_model_name_uses_fetched() {
        let mut wizard = OnboardingWizard::new();
        // No static models - should use fetched or show placeholder
        assert!(wizard.selected_model_name().is_empty() || wizard.fetched_models.is_empty());

        wizard.fetched_models = vec!["live-model-1".into(), "live-model-2".into()];
        wizard.selected_model = 1;
        assert_eq!(wizard.selected_model_name(), "live-model-2");
    }

    #[test]
    fn test_supports_model_fetch() {
        let mut wizard = OnboardingWizard::new();
        wizard.selected_provider = 0; // Anthropic
        assert!(wizard.supports_model_fetch());
        wizard.selected_provider = 1; // OpenAI
        assert!(wizard.supports_model_fetch());
        wizard.selected_provider = 2; // Gemini
        assert!(!wizard.supports_model_fetch());
        wizard.selected_provider = 3; // OpenRouter
        assert!(wizard.supports_model_fetch());
        wizard.selected_provider = 4; // Minimax
        assert!(!wizard.supports_model_fetch());
        wizard.selected_provider = 5; // Custom
        assert!(!wizard.supports_model_fetch());
    }

    #[test]
    fn test_fetch_models_unsupported_provider_returns_empty() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(fetch_provider_models(99, None));
        assert!(result.is_empty());
    }

    // --- Live API integration tests (skipped if env var not set) ---

    #[test]
    fn test_fetch_anthropic_models_with_api_key() {
        let key = match std::env::var("ANTHROPIC_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => return, // ANTHROPIC_API_KEY not set, skip
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let models = rt.block_on(fetch_provider_models(0, Some(&key)));
        assert!(
            !models.is_empty(),
            "Anthropic should return models with API key"
        );
        // Should contain at least one claude model
        assert!(
            models.iter().any(|m| m.contains("claude")),
            "Expected claude model, got: {:?}",
            models
        );
    }

    #[test]
    fn test_fetch_anthropic_models_with_setup_token() {
        let key = match std::env::var("ANTHROPIC_MAX_SETUP_TOKEN") {
            Ok(k) if !k.is_empty() && k.starts_with("sk-ant-oat") => k,
            _ => return, // ANTHROPIC_MAX_SETUP_TOKEN not set, skip
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let models = rt.block_on(fetch_provider_models(0, Some(&key)));
        assert!(
            !models.is_empty(),
            "Anthropic should return models with setup token"
        );
        assert!(
            models.iter().any(|m| m.contains("claude")),
            "Expected claude model, got: {:?}",
            models
        );
    }

    #[test]
    fn test_fetch_openai_models_with_api_key() {
        let key = match std::env::var("OPENAI_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => return, // OPENAI_API_KEY not set, skip
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let models = rt.block_on(fetch_provider_models(1, Some(&key)));
        assert!(
            !models.is_empty(),
            "OpenAI should return models with API key"
        );
        assert!(
            models.iter().any(|m| m.contains("gpt")),
            "Expected gpt model, got: {:?}",
            models
        );
    }

    #[test]
    fn test_fetch_openrouter_models_with_api_key() {
        let key = match std::env::var("OPENROUTER_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => return, // OPENROUTER_API_KEY not set, skip
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let models = rt.block_on(fetch_provider_models(4, Some(&key)));
        assert!(!models.is_empty(), "OpenRouter should return models");
        // OpenRouter has 400+ models
        assert!(
            models.len() > 50,
            "Expected 50+ models from OpenRouter, got {}",
            models.len()
        );
    }

    #[test]
    fn test_fetch_models_bad_key_returns_empty() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Bad key should fail gracefully (empty vec, not panic)
        let models = rt.block_on(fetch_provider_models(
            0,
            Some("sk-bad-key-definitely-invalid"),
        ));
        assert!(
            models.is_empty(),
            "Bad key should return empty, got {} models",
            models.len()
        );
    }
}
