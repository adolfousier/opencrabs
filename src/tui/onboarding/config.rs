use std::path::PathBuf;

use crate::config::Config;

use super::types::*;
use super::wizard::OnboardingWizard;
use crate::tui::provider_selector::CUSTOM_PROVIDER_IDX;

/// Try to write a config key, collecting errors into a Vec for later reporting.
macro_rules! try_write {
    ($errors:expr, $section:expr, $key:expr, $val:expr) => {
        if let Err(e) = Config::write_key($section, $key, $val) {
            tracing::warn!("Failed to write {}.{}: {}", $section, $key, e);
            $errors.push(format!("{}.{}", $section, $key));
        }
    };
}

/// Try to write a config array, collecting errors into a Vec for later reporting.
macro_rules! try_write_array {
    ($errors:expr, $section:expr, $key:expr, $val:expr) => {
        if let Err(e) = Config::write_array($section, $key, $val) {
            tracing::warn!("Failed to write {}.{}: {}", $section, $key, e);
            $errors.push(format!("{}.{}", $section, $key));
        }
    };
}

impl OnboardingWizard {
    /// Ensure config.toml and keys.toml exist in the workspace directory
    pub(super) fn ensure_config_files(&mut self) -> Result<(), String> {
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
            let config_content = include_str!("../../../config.toml.example");
            std::fs::write(&config_path, config_content)
                .map_err(|e| format!("Failed to write config.toml: {}", e))?;
            tracing::info!("Created config.toml at {:?}", config_path);
        }

        // Create keys.toml if it doesn't exist (copy from embedded example)
        if !keys_path.exists() {
            let keys_content = include_str!("../../../keys.toml.example");
            std::fs::write(&keys_path, keys_content)
                .map_err(|e| format!("Failed to write keys.toml: {}", e))?;
            tracing::info!("Created keys.toml at {:?}", keys_path);
        }

        // Create usage_pricing.toml if it doesn't exist
        let pricing_path = workspace_path.join("usage_pricing.toml");
        if !pricing_path.exists() {
            let pricing_content = include_str!("../../../usage_pricing.toml.example");
            std::fs::write(&pricing_path, pricing_content)
                .map_err(|e| format!("Failed to write usage_pricing.toml: {}", e))?;
            tracing::info!("Created usage_pricing.toml at {:?}", pricing_path);
        }

        // Reload models for the selected provider from the newly created config
        self.ps.reload_config_models();

        Ok(())
    }

    /// Initialize health check results
    pub fn start_health_check(&mut self) {
        // Reload config from disk so re-check picks up external changes
        if self.quick_jump
            && let Ok(config) = crate::config::Config::load()
        {
            let fresh = Self::from_config(&config);
            self.ps.api_key_input = fresh.ps.api_key_input;
            self.ps.selected_provider = fresh.ps.selected_provider;
            self.workspace_path = fresh.workspace_path;
            self.channel_toggles = fresh.channel_toggles;
            self.telegram_token_input = fresh.telegram_token_input;
            self.telegram_user_id_input = fresh.telegram_user_id_input;
            self.discord_token_input = fresh.discord_token_input;
            self.discord_channel_id_input = fresh.discord_channel_id_input;
            self.slack_bot_token_input = fresh.slack_bot_token_input;
            self.slack_app_token_input = fresh.slack_app_token_input;
            self.slack_channel_id_input = fresh.slack_channel_id_input;
            self.trello_api_key_input = fresh.trello_api_key_input;
            self.trello_api_token_input = fresh.trello_api_token_input;
            self.trello_board_id_input = fresh.trello_board_id_input;
            self.whatsapp_connected = fresh.whatsapp_connected;
            self.image_vision_enabled = fresh.image_vision_enabled;
            self.image_generation_enabled = fresh.image_generation_enabled;
            self.image_api_key_input = fresh.image_api_key_input;
        }

        let auth_label = if self.ps.is_cli() {
            "CLI Binary Found"
        } else {
            "API Key Present"
        };
        let mut checks = vec![
            (auth_label.to_string(), HealthStatus::Pending),
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
        if self.is_trello_enabled() {
            checks.push(("Trello API Key".to_string(), HealthStatus::Pending));
            checks.push(("Trello API Token".to_string(), HealthStatus::Pending));
            checks.push(("Trello Board ID".to_string(), HealthStatus::Pending));
        }
        if self.image_vision_enabled || self.image_generation_enabled {
            checks.push(("Google Image API Key".to_string(), HealthStatus::Pending));
        }

        self.health_results = checks;
        self.health_running = true;
        self.health_complete = false;
    }

    /// Resolve pending health checks (call from tick to show Pending state for one frame).
    pub fn tick_health_check(&mut self) {
        if self.health_running && !self.health_complete {
            self.run_health_checks();
        }
    }

    /// Execute all health checks
    fn run_health_checks(&mut self) {
        // Check 1: API key / CLI binary present
        self.health_results[0].1 = if self.ps.is_cli() {
            // CLI providers: check if the binary is installed
            let binary = match self.ps.provider_id() {
                "claude-cli" => "claude",
                "qwen-code-cli" => "qwen",
                _ => "opencode",
            };
            if which::which(binary).is_ok() {
                HealthStatus::Pass
            } else {
                HealthStatus::Fail(format!("'{}' CLI not found in PATH", binary))
            }
        } else if !self.ps.api_key_input.is_empty()
            || (self.ps.is_custom() && !self.ps.base_url.is_empty())
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
                "Trello API Key" => {
                    if !self.trello_api_key_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No API Key provided".to_string())
                    }
                }
                "Trello API Token" => {
                    if !self.trello_api_token_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail("No API Token provided".to_string())
                    }
                }
                "Trello Board ID" => {
                    if !self.trello_board_id_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail(
                            "No Board ID — agent won't know which board to poll".to_string(),
                        )
                    }
                }
                "Google Image API Key" => {
                    if !self.image_api_key_input.is_empty() {
                        HealthStatus::Pass
                    } else {
                        HealthStatus::Fail(
                            "No API key — vision and image generation need a Google AI key"
                                .to_string(),
                        )
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

    /// Apply wizard configuration — creates config.toml, stores API key, seeds workspace
    /// Merges with existing config to preserve settings not modified in wizard.
    ///
    /// In quick_jump mode, only writes settings relevant to the current step to avoid
    /// overwriting unrelated channel/provider settings loaded with defaults.
    pub fn apply_config(&self) -> Result<(), String> {
        // Determine which sections to write based on quick_jump + current step
        let write_provider = !self.quick_jump
            || matches!(
                self.step,
                OnboardingStep::ProviderAuth | OnboardingStep::Complete
            );
        let write_channels = !self.quick_jump
            || matches!(
                self.step,
                OnboardingStep::Channels
                    | OnboardingStep::TelegramSetup
                    | OnboardingStep::DiscordSetup
                    | OnboardingStep::WhatsAppSetup
                    | OnboardingStep::SlackSetup
                    | OnboardingStep::TrelloSetup
                    | OnboardingStep::Complete
            );
        let write_voice = !self.quick_jump
            || matches!(
                self.step,
                OnboardingStep::VoiceSetup | OnboardingStep::Complete
            );
        let write_image = !self.quick_jump
            || matches!(
                self.step,
                OnboardingStep::ImageSetup | OnboardingStep::Complete
            );

        // Groq key for STT/TTS
        let groq_key = if !self.groq_api_key_input.is_empty() && !self.has_existing_groq_key() {
            Some(self.groq_api_key_input.clone())
        } else {
            None
        };

        // Write config.toml via merge (write_key) — never overwrite entire file
        let mut write_errors: Vec<String> = Vec::new();

        // Provider settings — only when relevant step is active
        let custom_section;
        let section = if self.ps.selected_provider < CUSTOM_PROVIDER_IDX {
            let id = PROVIDERS[self.ps.selected_provider].id;
            crate::utils::providers::find_provider_meta(id)
                .map(|m| m.config_section)
                .unwrap_or("providers.anthropic")
        } else {
            custom_section = format!("providers.custom.{}", self.ps.custom_name);
            &custom_section
        };

        if write_provider {
            // Disable all providers first, then enable selected one
            {
                let all_sections = if let Ok(cfg) = Config::load() {
                    crate::utils::providers::all_config_sections(&cfg.providers)
                } else {
                    crate::utils::providers::KNOWN_PROVIDERS
                        .iter()
                        .map(|p| p.config_section.to_string())
                        .collect()
                };
                for s in &all_sections {
                    if let Err(e) = Config::write_key(s, "enabled", "false") {
                        tracing::warn!("Failed to write {}.enabled: {}", s, e);
                        write_errors.push(format!("{}.enabled", s));
                    }
                }
            }

            // Enable + configure the selected provider
            let custom_section;
            let section = if self.ps.selected_provider < CUSTOM_PROVIDER_IDX {
                let id = PROVIDERS[self.ps.selected_provider].id;
                crate::utils::providers::find_provider_meta(id)
                    .map(|m| m.config_section)
                    .unwrap_or("providers.anthropic")
            } else {
                custom_section = format!("providers.custom.{}", self.ps.custom_name);
                &custom_section
            };
            try_write!(write_errors, section, "enabled", "true");
            let model = self.ps.selected_model_name().to_string();
            if !model.is_empty() {
                try_write!(write_errors, section, "default_model", &model);
            }

            // Write base_url / extra config for providers that need it
            match self.ps.provider_id() {
                "github" => {
                    try_write!(
                        write_errors,
                        section,
                        "base_url",
                        "https://api.githubcopilot.com/chat/completions"
                    );
                }
                "openrouter" => {
                    try_write!(
                        write_errors,
                        section,
                        "base_url",
                        "https://openrouter.ai/api/v1/chat/completions"
                    );
                }
                "minimax" => {
                    try_write!(
                        write_errors,
                        section,
                        "base_url",
                        "https://api.minimax.io/v1"
                    );
                }
                "zhipu" => {
                    let endpoint_type = if self.ps.zhipu_endpoint_type == 1 {
                        "coding"
                    } else {
                        "api"
                    };
                    try_write!(write_errors, section, "endpoint_type", endpoint_type);
                }
                "" => {
                    if !self.ps.base_url.is_empty() {
                        try_write!(write_errors, section, "base_url", &self.ps.base_url);
                    }
                    if !self.ps.custom_model.is_empty() {
                        try_write!(
                            write_errors,
                            section,
                            "default_model",
                            &self.ps.custom_model
                        );
                    }
                    if !self.ps.context_window.is_empty() {
                        try_write!(
                            write_errors,
                            section,
                            "context_window",
                            &self.ps.context_window
                        );
                    }
                }
                _ => {}
            }

            // Write models array for providers that have static model lists
            if !self.ps.config_models.is_empty()
                && (matches!(self.ps.provider_id(), "github" | "minimax" | "zhipu" | "")
                    || self.ps.selected_provider >= CUSTOM_PROVIDER_IDX)
            {
                try_write_array!(write_errors, section, "models", &self.ps.config_models);
            }
            // Write enable_thinking for Qwen (thinking mode on by default)
            if self.ps.provider_id() == "qwen" {
                try_write!(write_errors, section, "enable_thinking", "true");
            }
        } // end if write_provider

        // Agent defaults — ensure these are persisted on fresh install
        // (serde defaults handle runtime, but we persist so they're visible in config.toml)
        try_write!(write_errors, "agent", "approval_policy", "auto-always");

        if write_channels {
            // Channel enabled flags (from channel_toggles: 0=Telegram, 1=Discord, 2=WhatsApp, 3=Slack)
            try_write!(
                write_errors,
                "channels.telegram",
                "enabled",
                &self.is_telegram_enabled().to_string()
            );
            try_write!(
                write_errors,
                "channels.discord",
                "enabled",
                &self.is_discord_enabled().to_string()
            );
            try_write!(
                write_errors,
                "channels.whatsapp",
                "enabled",
                &self.channel_toggles.get(2).is_some_and(|t| t.1).to_string()
            );
            try_write!(
                write_errors,
                "channels.slack",
                "enabled",
                &self.is_slack_enabled().to_string()
            );
            try_write!(
                write_errors,
                "channels.trello",
                "enabled",
                &self.is_trello_enabled().to_string()
            );

            // respond_to per channel
            let respond_to_values = ["all", "dm_only", "mention"];
            try_write!(
                write_errors,
                "channels.telegram",
                "respond_to",
                respond_to_values[self.telegram_respond_to.min(2)]
            );
            try_write!(
                write_errors,
                "channels.discord",
                "respond_to",
                respond_to_values[self.discord_respond_to.min(2)]
            );
            try_write!(
                write_errors,
                "channels.slack",
                "respond_to",
                respond_to_values[self.slack_respond_to.min(2)]
            );
        } // end if write_channels

        if write_voice {
            // Voice config (0=Off, 1=API, 2=Local for both STT and TTS)
            let is_local_stt = self.stt_mode == 2;
            let is_api_stt = self.stt_mode == 1;
            let groq_key_exists =
                !self.groq_api_key_input.is_empty() || self.has_existing_groq_key();

            // STT API provider (Groq)
            try_write!(
                write_errors,
                "providers.stt.groq",
                "enabled",
                &(is_api_stt && groq_key_exists).to_string()
            );
            if is_api_stt && groq_key_exists {
                try_write!(
                    write_errors,
                    "providers.stt.groq",
                    "default_model",
                    "whisper-large-v3-turbo"
                );
            }

            // STT local provider
            try_write!(
                write_errors,
                "providers.stt.local",
                "enabled",
                &is_local_stt.to_string()
            );
            if is_local_stt {
                #[cfg(feature = "local-stt")]
                {
                    use crate::channels::voice::local_whisper::LOCAL_MODEL_PRESETS;
                    if self.selected_local_stt_model < LOCAL_MODEL_PRESETS.len() {
                        try_write!(
                            write_errors,
                            "providers.stt.local",
                            "model",
                            LOCAL_MODEL_PRESETS[self.selected_local_stt_model].id
                        );
                    }
                }
            }

            // TTS API provider (OpenAI)
            let is_api_tts = self.tts_enabled && self.tts_mode == 1;
            let is_local_tts = self.tts_enabled && self.tts_mode == 2;
            try_write!(
                write_errors,
                "providers.tts.openai",
                "enabled",
                &is_api_tts.to_string()
            );
            if is_api_tts {
                try_write!(
                    write_errors,
                    "providers.tts.openai",
                    "default_model",
                    "gpt-4o-mini-tts"
                );
            }

            // TTS local provider (Piper)
            try_write!(
                write_errors,
                "providers.tts.local",
                "enabled",
                &is_local_tts.to_string()
            );
            if is_local_tts {
                #[cfg(feature = "local-tts")]
                {
                    use crate::channels::voice::local_tts::PIPER_VOICES;
                    if self.selected_tts_voice < PIPER_VOICES.len() {
                        try_write!(
                            write_errors,
                            "providers.tts.local",
                            "voice",
                            PIPER_VOICES[self.selected_tts_voice].id
                        );
                    }
                }
            }
        } // end if write_voice

        if write_image {
            // Image config
            let image_model = "gemini-3.1-flash-image-preview";
            if self.image_generation_enabled {
                try_write!(write_errors, "image.generation", "enabled", "true");
                try_write!(write_errors, "image.generation", "model", image_model);
            }
            if self.image_vision_enabled {
                try_write!(write_errors, "image.vision", "enabled", "true");
                try_write!(write_errors, "image.vision", "model", image_model);
            }
            // Save image API key to keys.toml (only if newly entered)
            if !self.image_api_key_input.is_empty()
                && !self.has_existing_image_key()
                && let Err(e) = crate::config::write_secret_key(
                    "providers.image.gemini",
                    "api_key",
                    &self.image_api_key_input,
                )
            {
                tracing::warn!("Failed to save image API key to keys.toml: {}", e);
            }
        } // end if write_image

        // Save API key to keys.toml via merge — never overwrite
        if write_provider
            && !self.ps.has_existing_key_sentinel()
            && !self.ps.api_key_input.is_empty()
            && let Err(e) =
                crate::config::write_secret_key(section, "api_key", &self.ps.api_key_input)
        {
            tracing::warn!("Failed to save API key to keys.toml: {}", e);
        }

        // (GitHub Copilot OAuth token is saved directly via the device flow handler)

        // Save STT/TTS keys to keys.toml
        if write_voice {
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
        } // end voice keys

        if write_channels {
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
            // Trello API Key (saved as app_token) + API Token
            if !self.trello_api_key_input.is_empty()
                && !self.has_existing_trello_api_key()
                && let Err(e) = crate::config::write_secret_key(
                    "channels.trello",
                    "app_token",
                    &self.trello_api_key_input,
                )
            {
                tracing::warn!("Failed to save Trello API Key to keys.toml: {}", e);
            }
            if !self.trello_api_token_input.is_empty()
                && !self.has_existing_trello_api_token()
                && let Err(e) = crate::config::write_secret_key(
                    "channels.trello",
                    "token",
                    &self.trello_api_token_input,
                )
            {
                tracing::warn!("Failed to save Trello API Token to keys.toml: {}", e);
            }

            // Persist channel IDs/user IDs to config.toml (if new)
            // telegram_user_id_input is never a sentinel — write whatever
            // the user left in the field (empty = "allow any user").
            if !self.telegram_user_id_input.is_empty() {
                try_write_array!(
                    write_errors,
                    "channels.telegram",
                    "allowed_users",
                    std::slice::from_ref(&self.telegram_user_id_input)
                );
            }
            if !self.discord_channel_id_input.is_empty() && !self.has_existing_discord_channel_id()
            {
                try_write_array!(
                    write_errors,
                    "channels.discord",
                    "allowed_channels",
                    std::slice::from_ref(&self.discord_channel_id_input)
                );
            }
            if !self.slack_channel_id_input.is_empty() && !self.has_existing_slack_channel_id() {
                try_write_array!(
                    write_errors,
                    "channels.slack",
                    "allowed_channels",
                    std::slice::from_ref(&self.slack_channel_id_input)
                );
            }
            if !self.discord_allowed_list_input.is_empty()
                && !self.has_existing_discord_allowed_list()
            {
                try_write_array!(
                    write_errors,
                    "channels.discord",
                    "allowed_users",
                    std::slice::from_ref(&self.discord_allowed_list_input)
                );
            }
            if !self.slack_allowed_list_input.is_empty() && !self.has_existing_slack_allowed_list()
            {
                try_write_array!(
                    write_errors,
                    "channels.slack",
                    "allowed_users",
                    std::slice::from_ref(&self.slack_allowed_list_input)
                );
            }
            if !self.whatsapp_phone_input.is_empty() && !self.has_existing_whatsapp_phone() {
                try_write_array!(
                    write_errors,
                    "channels.whatsapp",
                    "allowed_phones",
                    std::slice::from_ref(&self.whatsapp_phone_input)
                );
            }
            if !self.trello_board_id_input.is_empty() && !self.has_existing_trello_board_id() {
                let boards: Vec<String> = self
                    .trello_board_id_input
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !boards.is_empty() {
                    try_write_array!(write_errors, "channels.trello", "board_ids", &boards);
                }
            }
            if !self.trello_allowed_users_input.is_empty()
                && !self.has_existing_trello_allowed_users()
            {
                let users: Vec<String> = self
                    .trello_allowed_users_input
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !users.is_empty() {
                    try_write_array!(write_errors, "channels.trello", "allowed_users", &users);
                }
            }
        } // end if write_channels

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

        if !write_errors.is_empty() {
            tracing::error!(
                "Onboarding: failed to write {} config keys: {}",
                write_errors.len(),
                write_errors.join(", ")
            );
            return Err(format!(
                "Some settings could not be saved ({}). Check file permissions on config.toml.",
                write_errors.join(", ")
            ));
        }

        Ok(())
    }
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
ExecStart={} daemon
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

    // Enable and start the service
    std::process::Command::new("systemctl")
        .args(["--user", "enable", "opencrabs"])
        .output()
        .map_err(|e| format!("Failed to enable service: {}", e))?;

    std::process::Command::new("systemctl")
        .args(["--user", "start", "opencrabs"])
        .output()
        .map_err(|e| format!("Failed to start service: {}", e))?;

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
        <string>daemon</string>
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
