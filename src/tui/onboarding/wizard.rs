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
    /// Context window size in tokens for custom providers (e.g. "32000")
    pub custom_context_window: String,
    /// Models fetched live from provider API (overrides static list when non-empty)
    pub fetched_models: Vec<String>,
    pub models_fetching: bool,
    /// Models from config.toml (used when API fetch not available)
    pub config_models: Vec<String>,
    /// Cached list of existing custom provider names (for provider list)
    pub existing_custom_names: Vec<String>,

    /// Step 4: Workspace
    pub workspace_path: String,
    pub seed_templates: bool,

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

    /// respond_to selection per channel (0=all, 1=dm_only, 2=mention)
    pub telegram_respond_to: usize,
    pub discord_respond_to: usize,
    pub slack_respond_to: usize,

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
    /// 0 = Off, 1 = API (Groq), 2 = Local (whisper.cpp)
    pub stt_mode: usize,
    pub groq_api_key_input: String,
    /// Index into LOCAL_MODEL_PRESETS (0=Tiny, 1=Base, 2=Small, 3=Medium)
    pub selected_local_stt_model: usize,
    /// Download progress (0.0 - 1.0), None if not downloading
    pub stt_model_download_progress: Option<f64>,
    /// Download status message
    pub stt_model_download_error: Option<String>,
    /// Whether the selected model is downloaded and ready
    pub stt_model_downloaded: bool,
    pub tts_enabled: bool,
    /// 0 = Off, 1 = API (OpenAI), 2 = Local (Piper)
    pub tts_mode: usize,
    /// Index into PIPER_VOICES (0=ryan, 1=amy, etc.)
    pub selected_tts_voice: usize,
    /// Download progress for Piper voice (0.0 - 1.0), None if not downloading
    pub tts_voice_download_progress: Option<f64>,
    /// Download error message
    pub tts_voice_download_error: Option<String>,
    /// Whether the selected Piper voice is downloaded and ready
    pub tts_voice_downloaded: bool,

    /// Step 7: Image Setup
    pub image_field: ImageField,
    pub image_vision_enabled: bool,
    pub image_generation_enabled: bool,
    pub image_api_key_input: String,

    /// Step 8: Daemon
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

    /// Opened from chat via slash command (e.g. /doctor, /onboard:provider).
    /// Shows only the target step: no progress dots, no navigation, Enter/Esc exit.
    pub quick_jump: bool,
    /// Set by `next_step()` when `quick_jump` is true — signals the step is done
    /// and `handle_key` should return `WizardAction::Cancel`.
    pub quick_jump_done: bool,
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
                } else if config.providers.github.as_ref().is_some_and(|p| p.enabled) {
                    (
                        2,
                        EXISTING_KEY_SENTINEL.to_string(),
                        String::new(),
                        String::new(),
                    )
                } else if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
                    (
                        3,
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
                        4,
                        EXISTING_KEY_SENTINEL.to_string(),
                        String::new(),
                        String::new(),
                    )
                } else if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
                    (
                        5,
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
                    // context_window is set after wizard construction below
                    // Map to index 7+ for existing custom providers
                    let idx = config
                        .providers
                        .custom
                        .as_ref()
                        .and_then(|m| m.keys().position(|k| k == name).map(|pos| 7 + pos))
                        .unwrap_or(6);
                    (idx, EXISTING_KEY_SENTINEL.to_string(), base, model)
                } else {
                    (0, String::new(), String::new(), String::new())
                }
            } else {
                (0, String::new(), String::new(), String::new())
            };

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
            custom_context_window: String::new(),
            fetched_models: Vec::new(),
            models_fetching: false,
            config_models,
            existing_custom_names: existing_config
                .as_ref()
                .and_then(|c| c.providers.custom.as_ref())
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default(),

            workspace_path: default_workspace.to_string_lossy().to_string(),
            seed_templates: true,

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

            telegram_respond_to: 0, // all
            discord_respond_to: 2,  // mention
            slack_respond_to: 2,    // mention

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

            voice_field: VoiceField::SttModeSelect,
            stt_mode: 0,
            groq_api_key_input: String::new(),
            selected_local_stt_model: 0,
            stt_model_download_progress: None,
            stt_model_download_error: None,
            stt_model_downloaded: false,
            tts_enabled: false,
            tts_mode: 0,
            selected_tts_voice: 0,
            tts_voice_download_progress: None,
            tts_voice_download_error: None,
            tts_voice_downloaded: false,

            image_field: ImageField::VisionToggle,
            image_vision_enabled: false,
            image_generation_enabled: false,
            image_api_key_input: String::new(),

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
            quick_jump: false,
            quick_jump_done: false,
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
        } else if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
            wizard.selected_provider = 1; // OpenAI
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
        } else if config.providers.github.as_ref().is_some_and(|p| p.enabled) {
            wizard.selected_provider = 2; // GitHub Models
            if let Some(model) = &config
                .providers
                .github
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.custom_model = model.clone();
            }
        } else if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
            wizard.selected_provider = 3; // Gemini
        } else if config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            wizard.selected_provider = 4; // OpenRouter
            if let Some(model) = &config
                .providers
                .openrouter
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.custom_model = model.clone();
            }
        } else if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
            wizard.selected_provider = 5; // Minimax
            if let Some(model) = &config
                .providers
                .minimax
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.custom_model = model.clone();
            }
        }

        // Detect if we have an existing API key for the selected provider
        wizard.detect_existing_key();
        wizard.reload_config_models();

        // Load channel toggles (indices match CHANNEL_NAMES order)
        wizard.channel_toggles[0].1 = config.channels.telegram.enabled; // Telegram
        wizard.channel_toggles[1].1 = config.channels.discord.enabled; // Discord
        wizard.channel_toggles[2].1 = config.channels.whatsapp.enabled; // WhatsApp
        wizard.channel_toggles[3].1 = config.channels.slack.enabled; // Slack
        wizard.channel_toggles[4].1 = config.channels.trello.enabled; // Trello
        wizard.channel_toggles[5].1 = config.channels.signal.enabled; // Signal
        wizard.channel_toggles[6].1 = config.channels.google_chat.enabled; // Google Chat
        wizard.channel_toggles[7].1 = config.channels.imessage.enabled; // iMessage

        // Load respond_to per channel
        use crate::config::RespondTo;
        wizard.telegram_respond_to = match config.channels.telegram.respond_to {
            RespondTo::All => 0,
            RespondTo::DmOnly => 1,
            RespondTo::Mention => 2,
        };
        wizard.discord_respond_to = match config.channels.discord.respond_to {
            RespondTo::All => 0,
            RespondTo::DmOnly => 1,
            RespondTo::Mention => 2,
        };
        wizard.slack_respond_to = match config.channels.slack.respond_to {
            RespondTo::All => 0,
            RespondTo::DmOnly => 1,
            RespondTo::Mention => 2,
        };

        // Load voice settings (0=Off, 1=API, 2=Local for both STT and TTS)
        let vc = config.voice_config();
        wizard.stt_mode = if !vc.stt_enabled {
            0 // Off
        } else {
            match vc.stt_mode {
                crate::config::SttMode::Api => 1,
                crate::config::SttMode::Local => 2,
            }
        };
        wizard.tts_enabled = vc.tts_enabled;
        wizard.tts_mode = if !vc.tts_enabled {
            0 // Off
        } else {
            match vc.tts_mode {
                crate::config::TtsMode::Api => 1,
                crate::config::TtsMode::Local => 2,
            }
        };

        // If Local was saved but the capability isn't available on this machine, reset to Off
        if wizard.stt_mode == 2 && !crate::channels::voice::local_stt_available() {
            wizard.stt_mode = 0;
        }
        if wizard.tts_mode == 2 && !crate::channels::voice::local_tts_available() {
            wizard.tts_mode = 0;
            wizard.tts_enabled = false;
        }

        wizard.detect_existing_groq_key();

        // Resolve selected Piper voice index from config
        #[cfg(feature = "local-tts")]
        {
            use crate::channels::voice::local_tts::{PIPER_VOICES, piper_voice_exists};
            if let Some(idx) = PIPER_VOICES.iter().position(|v| v.id == vc.local_tts_voice) {
                wizard.selected_tts_voice = idx;
                wizard.tts_voice_downloaded = piper_voice_exists(PIPER_VOICES[idx].id);
            }
        }

        // Resolve selected local model index from config
        #[cfg(feature = "local-stt")]
        {
            use crate::channels::voice::local_whisper::{LOCAL_MODEL_PRESETS, is_model_downloaded};
            if let Some(idx) = LOCAL_MODEL_PRESETS
                .iter()
                .position(|p| p.id == vc.local_stt_model)
            {
                wizard.selected_local_stt_model = idx;
                wizard.stt_model_downloaded = is_model_downloaded(&LOCAL_MODEL_PRESETS[idx]);
            }
        }

        // Load image settings
        wizard.image_vision_enabled = config.image.vision.enabled;
        wizard.image_generation_enabled = config.image.generation.enabled;
        wizard.detect_existing_image_key();

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
        // Trello: load API Key (app_token), API Token (token), board ID
        if let Some(ref api_key) = config.channels.trello.app_token {
            wizard.trello_api_key_input = api_key.clone();
        }
        if let Some(ref token) = config.channels.trello.token {
            wizard.trello_api_token_input = token.clone();
        }
        if let Some(board_id) = config.channels.trello.board_ids.first() {
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
        // Indices >= 7 are existing custom providers — map to the Custom entry (index 6)
        &PROVIDERS[self.selected_provider.min(PROVIDERS.len() - 1)]
    }

    /// Check if the current provider is a custom option (new or existing)
    pub fn is_custom_provider(&self) -> bool {
        self.selected_provider >= PROVIDERS.len() - 1
    }

    /// Whether the current api_key_input holds a pre-existing key (from env/keyring)
    pub fn has_existing_key(&self) -> bool {
        self.api_key_input == EXISTING_KEY_SENTINEL
    }

    /// When navigating to an existing custom provider (index >= 7), load its config fields.
    /// For index 6 (new custom), clear the fields.
    pub fn load_custom_fields_if_existing(&mut self) {
        if self.selected_provider == 6 {
            // "+ New Custom Provider" — clear fields
            self.custom_provider_name.clear();
            self.custom_base_url.clear();
            self.custom_model.clear();
            self.custom_context_window.clear();
        } else if self.selected_provider >= 7 {
            let custom_idx = self.selected_provider - 7;
            if let Some(cname) = self.existing_custom_names.get(custom_idx).cloned()
                && let Ok(config) = crate::config::Config::load()
                && let Some(c) = config.providers.custom_by_name(&cname)
            {
                self.custom_provider_name = cname;
                self.custom_base_url = c.base_url.clone().unwrap_or_default();
                self.custom_model = c.default_model.clone().unwrap_or_default();
                self.custom_context_window = c
                    .context_window
                    .map(|cw| cw.to_string())
                    .unwrap_or_default();
                if c.api_key.as_ref().is_some_and(|k| !k.is_empty()) {
                    self.api_key_input = EXISTING_KEY_SENTINEL.to_string();
                }
            }
        }
    }
}
