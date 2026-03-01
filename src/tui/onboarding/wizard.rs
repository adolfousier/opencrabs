use std::path::PathBuf;

use super::types::*;

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

    /// Trello Setup (shown when Trello is enabled)
    pub trello_field: TrelloField,
    pub trello_api_key_input: String,
    pub trello_api_token_input: String,
    pub trello_board_id_input: String,
    pub trello_allowed_users_input: String,

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
        let mut custom_provider_name_init: Option<String> = None;
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
                } else if let Some((name, c)) = config.providers.active_custom().or_else(|| {
                    config
                        .providers
                        .custom
                        .as_ref()
                        .and_then(|m| m.iter().next())
                        .map(|(n, c)| (n.as_str(), c))
                }) {
                    let base = c.base_url.clone().unwrap_or_default();
                    let model = c.default_model.clone().unwrap_or_default();
                    custom_provider_name_init = Some(name.to_string());
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
            custom_provider_name: custom_provider_name_init.unwrap_or_default(),
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

            trello_field: TrelloField::ApiKey,
            trello_api_key_input: String::new(),
            trello_api_token_input: String::new(),
            trello_board_id_input: String::new(),
            trello_allowed_users_input: String::new(),

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
    pub fn from_config(config: &crate::config::Config) -> Self {
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
        wizard.channel_toggles[4].1 = config.channels.trello.enabled; // Trello
        wizard.channel_toggles[5].1 = config.channels.signal.enabled; // Signal
        wizard.channel_toggles[6].1 = config.channels.google_chat.enabled; // Google Chat
        wizard.channel_toggles[7].1 = config.channels.imessage.enabled; // iMessage

        // Load voice settings
        wizard.tts_enabled = config.voice.tts_enabled;
        wizard.detect_existing_groq_key();

        // Load channel tokens/IDs from config (keys.toml is merged in at load time)
        if let Some(ref token) = config.channels.telegram.token {
            wizard.telegram_token_input = token.clone();
        }
        if let Some(user_id) = config.channels.telegram.allowed_users.first() {
            wizard.telegram_user_id_input = user_id.clone();
        }
        if let Some(ref token) = config.channels.discord.token {
            wizard.discord_token_input = token.clone();
        }
        if let Some(channel_id) = config.channels.discord.allowed_channels.first() {
            wizard.discord_channel_id_input = channel_id.clone();
        }
        if let Some(ref token) = config.channels.slack.token {
            wizard.slack_bot_token_input = token.clone();
        }
        if let Some(ref app_token) = config.channels.slack.app_token {
            wizard.slack_app_token_input = app_token.clone();
        }
        if let Some(channel_id) = config.channels.slack.allowed_channels.first() {
            wizard.slack_channel_id_input = channel_id.clone();
        }
        // Trello: load API Key (app_token) + board ID (first allowed_channel)
        if let Some(ref api_key) = config.channels.trello.app_token {
            wizard.trello_api_key_input = api_key.clone();
        }
        if let Some(board_id) = config.channels.trello.allowed_channels.first() {
            wizard.trello_board_id_input = board_id.clone();
        }
        // WhatsApp: check if session.db exists (means it's paired)
        let wa_session = crate::config::opencrabs_home()
            .join("whatsapp")
            .join("session.db");
        wizard.whatsapp_connected = wa_session.exists();

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

    /// Whether the current api_key_input holds a pre-existing key (from env/keyring)
    pub fn has_existing_key(&self) -> bool {
        self.api_key_input == EXISTING_KEY_SENTINEL
    }
}
