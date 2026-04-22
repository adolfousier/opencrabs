use super::types::*;

/// Main onboarding wizard state
pub struct OnboardingWizard {
    pub step: OnboardingStep,
    pub mode: WizardMode,

    // Step 2: Provider/Auth — shared state with /models dialog
    pub ps: crate::tui::provider_selector::ProviderSelectorState,
    pub auth_field: AuthField,

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

    /// Cursor position for whichever channel text field is currently active
    pub channel_input_cursor: usize,

    /// Channel test connection status
    pub channel_test_status: ChannelTestStatus,

    /// Step 6: Voice Setup
    pub voice_field: VoiceField,
    /// Which STT provider is selected
    pub stt_provider: SttProvider,
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
    /// Which TTS provider is selected
    pub tts_provider: TtsProvider,
    /// Index into PIPER_VOICES (0=ryan, 1=amy, etc.)
    pub selected_tts_voice: usize,
    /// Download progress for Piper voice (0.0 - 1.0), None if not downloading
    pub tts_voice_download_progress: Option<f64>,
    /// Download error message
    pub tts_voice_download_error: Option<String>,
    /// Whether the selected Piper voice is downloaded and ready
    pub tts_voice_downloaded: bool,

    // OpenAI-compatible STT
    pub stt_openai_compat_base_url: String,
    pub stt_openai_compat_model: String,
    pub stt_openai_compat_key_input: String,
    // Voicebox STT
    pub stt_voicebox_base_url: String,
    // OpenAI-compatible TTS
    pub tts_openai_compat_base_url: String,
    pub tts_openai_compat_model: String,
    pub tts_openai_compat_voice: String,
    pub tts_openai_compat_key_input: String,
    // Voicebox TTS
    pub tts_voicebox_base_url: String,
    pub tts_voicebox_profile_id: String,
    pub tts_voicebox_engine: String,

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
    /// Whether the user has started editing each field (arrow key or char input).
    /// First backspace on an untouched template clears the entire field.
    pub brain_me_edited: bool,
    pub brain_agent_edited: bool,
    pub brain_generating: bool,
    pub brain_generated: bool,
    pub brain_error: Option<String>,
    /// Auto-format preview state
    pub preview_shown: bool,
    pub formatted_about_me: String,
    pub formatted_about_agent: String,
    pub generated_soul: Option<String>,
    pub generated_identity: Option<String>,
    pub generated_user: Option<String>,
    pub generated_agents: Option<String>,
    pub generated_tools: Option<String>,
    pub generated_memory: Option<String>,

    /// GitHub Copilot device flow state
    pub github_user_code: Option<String>,
    pub github_device_flow_status: GitHubDeviceFlowStatus,

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
        let default_workspace = crate::config::opencrabs_home();

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
                } else if config.providers.zhipu.as_ref().is_some_and(|p| p.enabled) {
                    (
                        6,
                        EXISTING_KEY_SENTINEL.to_string(),
                        String::new(),
                        String::new(),
                    )
                } else if config
                    .providers
                    .claude_cli
                    .as_ref()
                    .is_some_and(|p| p.enabled)
                {
                    (
                        crate::tui::provider_selector::index_of_provider("claude-cli").unwrap_or(0),
                        String::new(),
                        String::new(),
                        String::new(),
                    )
                } else if config
                    .providers
                    .opencode_cli
                    .as_ref()
                    .is_some_and(|p| p.enabled)
                {
                    (
                        crate::tui::provider_selector::index_of_provider("opencode-cli")
                            .unwrap_or(0),
                        String::new(),
                        String::new(),
                        String::new(),
                    )
                } else if config.providers.qwen.as_ref().is_some_and(|p| p.enabled) {
                    (
                        crate::tui::provider_selector::index_of_provider("qwen").unwrap_or(0),
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
                    // Map to CUSTOM_INSTANCES_START+ for existing custom providers
                    use crate::tui::provider_selector::{
                        CUSTOM_INSTANCES_START, CUSTOM_PROVIDER_IDX,
                    };
                    let idx = config
                        .providers
                        .custom
                        .as_ref()
                        .and_then(|m| {
                            m.keys()
                                .position(|k| k == name)
                                .map(|pos| CUSTOM_INSTANCES_START + pos)
                        })
                        .unwrap_or(CUSTOM_PROVIDER_IDX);
                    (idx, EXISTING_KEY_SENTINEL.to_string(), base, model)
                } else {
                    (0, String::new(), String::new(), String::new())
                }
            } else {
                (0, String::new(), String::new(), String::new())
            };

        let ps = crate::tui::provider_selector::ProviderSelectorState {
            selected_provider,
            api_key_input,
            api_key_cursor: 0,
            selected_model: 0,
            custom_name: custom_provider_name_init.unwrap_or_default(),
            base_url: custom_base_url,
            custom_model,
            context_window: String::new(),
            models: Vec::new(),
            models_fetching: false,
            config_models,
            custom_names: existing_config
                .as_ref()
                .and_then(|c| c.providers.custom.as_ref())
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default(),
            zhipu_endpoint_type: 0, // default to API mode
            model_filter: String::new(),
            ..Default::default()
        };

        let mut wizard = Self {
            step: OnboardingStep::ModeSelect,
            mode: WizardMode::QuickStart,

            ps,
            auth_field: AuthField::Provider,

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

            channel_input_cursor: 0,
            channel_test_status: ChannelTestStatus::Idle,

            voice_field: VoiceField::SttModeSelect,
            stt_provider: SttProvider::Off,
            groq_api_key_input: String::new(),
            selected_local_stt_model: 0,
            stt_model_download_progress: None,
            stt_model_download_error: None,
            stt_model_downloaded: false,
            tts_enabled: false,
            tts_provider: TtsProvider::Off,
            selected_tts_voice: 0,
            tts_voice_download_progress: None,
            tts_voice_download_error: None,
            tts_voice_downloaded: false,
            // OpenAI-compatible STT
            stt_openai_compat_base_url: String::new(),
            stt_openai_compat_model: "whisper-1".to_string(),
            stt_openai_compat_key_input: String::new(),
            // Voicebox STT
            stt_voicebox_base_url: "http://localhost:8000".to_string(),
            // OpenAI-compatible TTS
            tts_openai_compat_base_url: String::new(),
            tts_openai_compat_model: "tts-1".to_string(),
            tts_openai_compat_voice: "alloy".to_string(),
            tts_openai_compat_key_input: String::new(),
            // Voicebox TTS
            tts_voicebox_base_url: String::new(),
            tts_voicebox_profile_id: String::new(),
            tts_voicebox_engine: String::new(),

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
            brain_me_edited: false,
            brain_agent_edited: false,
            brain_generating: false,
            brain_generated: false,
            brain_error: None,
            preview_shown: false,
            formatted_about_me: String::new(),
            formatted_about_agent: String::new(),
            generated_soul: None,
            generated_identity: None,
            generated_user: None,
            generated_agents: None,
            generated_tools: None,
            generated_memory: None,

            github_user_code: None,
            github_device_flow_status: GitHubDeviceFlowStatus::Idle,

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
            wizard.ps.selected_provider = 0; // Anthropic
            if let Some(model) = &config
                .providers
                .anthropic
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.ps.custom_model = model.clone();
            }
        } else if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
            wizard.ps.selected_provider = 1; // OpenAI
            if let Some(base_url) = &config
                .providers
                .openai
                .as_ref()
                .and_then(|p| p.base_url.clone())
            {
                wizard.ps.base_url = base_url.clone();
            }
            if let Some(model) = &config
                .providers
                .openai
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.ps.custom_model = model.clone();
            }
        } else if config.providers.github.as_ref().is_some_and(|p| p.enabled) {
            wizard.ps.selected_provider = 2; // GitHub Copilot
            if let Some(model) = &config
                .providers
                .github
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.ps.custom_model = model.clone();
            }
        } else if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
            wizard.ps.selected_provider = 3; // Gemini
        } else if config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            wizard.ps.selected_provider = 4; // OpenRouter
            if let Some(model) = &config
                .providers
                .openrouter
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.ps.custom_model = model.clone();
            }
        } else if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
            wizard.ps.selected_provider = 5; // Minimax
            if let Some(model) = &config
                .providers
                .minimax
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.ps.custom_model = model.clone();
            }
        } else if config.providers.zhipu.as_ref().is_some_and(|p| p.enabled) {
            wizard.ps.selected_provider = 6; // z.ai GLM
            if let Some(model) = &config
                .providers
                .zhipu
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.ps.custom_model = model.clone();
            }
        } else if config
            .providers
            .claude_cli
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            wizard.ps.selected_provider = 7; // Claude CLI
            if let Some(model) = &config
                .providers
                .claude_cli
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.ps.custom_model = model.clone();
            }
        } else if config
            .providers
            .opencode_cli
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            wizard.ps.selected_provider = 8; // OpenCode CLI
            if let Some(model) = &config
                .providers
                .opencode_cli
                .as_ref()
                .and_then(|p| p.default_model.clone())
            {
                wizard.ps.custom_model = model.clone();
            }
        }

        // Detect if we have an existing API key for the selected provider
        wizard.ps.detect_existing_key();
        wizard.ps.reload_config_models();
        wizard.ps.resolve_selected_model_index();

        // Load channel toggles (indices match CHANNEL_NAMES order)
        wizard.channel_toggles[0].1 = config.channels.telegram.enabled; // Telegram
        wizard.channel_toggles[1].1 = config.channels.discord.enabled; // Discord
        wizard.channel_toggles[2].1 = config.channels.whatsapp.enabled; // WhatsApp
        wizard.channel_toggles[3].1 = config.channels.slack.enabled; // Slack
        wizard.channel_toggles[4].1 = config.channels.trello.enabled; // Trello

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

        // Load voice settings
        let vc = config.voice_config();
        // Initial mapping from legacy VoiceConfig (Groq/Local only)
        wizard.stt_provider = if !vc.stt_enabled {
            SttProvider::Off
        } else {
            match vc.stt_mode {
                crate::config::SttMode::Api => SttProvider::Groq,
                crate::config::SttMode::Local => SttProvider::Local,
            }
        };
        wizard.tts_enabled = vc.tts_enabled;
        wizard.tts_provider = if !vc.tts_enabled {
            TtsProvider::Off
        } else {
            match vc.tts_mode {
                crate::config::TtsMode::Api => TtsProvider::OpenAi,
                crate::config::TtsMode::Local => TtsProvider::Local,
            }
        };

        // If Local was saved but the capability isn't available, reset to Off
        if wizard.stt_provider == SttProvider::Local
            && !crate::channels::voice::local_stt_available()
        {
            wizard.stt_provider = SttProvider::Off;
        }
        if wizard.tts_provider == TtsProvider::Local
            && !crate::channels::voice::local_tts_available()
        {
            wizard.tts_provider = TtsProvider::Off;
            wizard.tts_enabled = false;
        }

        wizard.detect_existing_groq_key();

        // Load STT provider from config
        if let Some(stt) = config.providers.stt.as_ref() {
            if stt.voicebox.as_ref().is_some_and(|v| v.enabled) {
                wizard.stt_provider = SttProvider::Voicebox;
                wizard.stt_voicebox_base_url = stt.voicebox.as_ref().unwrap().base_url.clone();
            } else if stt.openai_compatible.as_ref().is_some_and(|v| v.enabled) {
                wizard.stt_provider = SttProvider::OpenAiCompatible;
                let oc = stt.openai_compatible.as_ref().unwrap();
                wizard.stt_openai_compat_base_url = oc.base_url.clone().unwrap_or_default();
                wizard.stt_openai_compat_model =
                    oc.model.clone().unwrap_or_else(|| "whisper-1".to_string());
                wizard.stt_openai_compat_key_input =
                    super::types::EXISTING_KEY_SENTINEL.to_string();
            } else if stt.local.as_ref().is_some_and(|l| l.enabled) {
                wizard.stt_provider = SttProvider::Local;
            } else if stt.groq.as_ref().is_some_and(|g| g.enabled) {
                wizard.stt_provider = SttProvider::Groq;
            } else {
                wizard.stt_provider = SttProvider::Off;
            }
        }

        // Load TTS provider from config
        if let Some(tts) = config.providers.tts.as_ref() {
            if tts.voicebox.as_ref().is_some_and(|v| v.enabled) {
                wizard.tts_provider = TtsProvider::Voicebox;
                wizard.tts_enabled = true;
                let vb = tts.voicebox.as_ref().unwrap();
                wizard.tts_voicebox_base_url = vb.base_url.clone();
                wizard.tts_voicebox_profile_id = vb.profile_id.clone();
                wizard.tts_voicebox_engine = vb.engine.clone();
                wizard.tts_voicebox_engine = vb.engine.clone();
            } else if tts.openai_compatible.as_ref().is_some_and(|v| v.enabled) {
                wizard.tts_provider = TtsProvider::OpenAiCompatible;
                wizard.tts_enabled = true;
                let oc = tts.openai_compatible.as_ref().unwrap();
                wizard.tts_openai_compat_base_url = oc.base_url.clone().unwrap_or_default();
                wizard.tts_openai_compat_model =
                    oc.model.clone().unwrap_or_else(|| "tts-1".to_string());
                wizard.tts_openai_compat_voice =
                    oc.voice.clone().unwrap_or_else(|| "alloy".to_string());
                wizard.tts_openai_compat_key_input =
                    super::types::EXISTING_KEY_SENTINEL.to_string();
            } else if tts.local.as_ref().is_some_and(|l| l.enabled) {
                wizard.tts_provider = TtsProvider::Local;
                wizard.tts_enabled = true;
            } else if tts.openai.as_ref().is_some_and(|o| o.enabled) {
                wizard.tts_provider = TtsProvider::OpenAi;
                wizard.tts_enabled = true;
            } else {
                wizard.tts_provider = TtsProvider::Off;
                wizard.tts_enabled = false;
            }
        }

        // Reset Local STT/TTS to Off if capability isn't available (after config loading)
        if wizard.stt_provider == SttProvider::Local
            && !crate::channels::voice::local_stt_available()
        {
            wizard.stt_provider = SttProvider::Off;
        }
        if wizard.tts_provider == TtsProvider::Local
            && !crate::channels::voice::local_tts_available()
        {
            wizard.tts_provider = TtsProvider::Off;
            wizard.tts_enabled = false;
        }

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

        // Mark existing channel data with sentinels so apply_config() won't overwrite.
        // When the user enters a channel sub-step, detect_existing_* re-checks and
        // keeps the sentinel; if they clear + re-enter data the sentinel is replaced.
        use super::types::EXISTING_KEY_SENTINEL;
        let sentinel = || EXISTING_KEY_SENTINEL.to_string();

        if config
            .channels
            .telegram
            .token
            .as_ref()
            .is_some_and(|t| !t.is_empty())
        {
            wizard.telegram_token_input = sentinel();
        }
        // Telegram user ID is NOT a secret — show the real stored value so the
        // user can see what's configured instead of a misleading "exists"
        // sentinel that makes them think an ID materialised from nowhere.
        if let Some(first) = config.channels.telegram.allowed_users.first() {
            wizard.telegram_user_id_input = first.clone();
        }
        if config
            .channels
            .discord
            .token
            .as_ref()
            .is_some_and(|t| !t.is_empty())
        {
            wizard.discord_token_input = sentinel();
        }
        if !config.channels.discord.allowed_channels.is_empty() {
            wizard.discord_channel_id_input = sentinel();
        }
        if !config.channels.discord.allowed_users.is_empty() {
            wizard.discord_allowed_list_input = sentinel();
        }
        if config
            .channels
            .slack
            .token
            .as_ref()
            .is_some_and(|t| !t.is_empty())
        {
            wizard.slack_bot_token_input = sentinel();
        }
        if config
            .channels
            .slack
            .app_token
            .as_ref()
            .is_some_and(|t| !t.is_empty())
        {
            wizard.slack_app_token_input = sentinel();
        }
        if !config.channels.slack.allowed_channels.is_empty() {
            wizard.slack_channel_id_input = sentinel();
        }
        if !config.channels.slack.allowed_users.is_empty() {
            wizard.slack_allowed_list_input = sentinel();
        }
        // Trello
        if config
            .channels
            .trello
            .app_token
            .as_ref()
            .is_some_and(|t| !t.is_empty())
        {
            wizard.trello_api_key_input = sentinel();
        }
        if config
            .channels
            .trello
            .token
            .as_ref()
            .is_some_and(|t| !t.is_empty())
        {
            wizard.trello_api_token_input = sentinel();
        }
        if !config.channels.trello.board_ids.is_empty() {
            wizard.trello_board_id_input = sentinel();
        }
        if !config.channels.trello.allowed_users.is_empty() {
            wizard.trello_allowed_users_input = sentinel();
        }
        // WhatsApp
        if !config.channels.whatsapp.allowed_phones.is_empty() {
            wizard.whatsapp_phone_input = sentinel();
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
}
