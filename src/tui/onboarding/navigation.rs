use super::types::*;
use super::wizard::OnboardingWizard;

impl OnboardingWizard {
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
                    && (self.custom_base_url.is_empty()
                        || self.custom_model.is_empty()
                        || self.custom_provider_name.is_empty())
                {
                    self.error_message = Some(
                        "Base URL, model name, and provider name are required for custom provider"
                            .to_string(),
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
            | OnboardingStep::SlackSetup
            | OnboardingStep::TrelloSetup => {
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
            | OnboardingStep::SlackSetup
            | OnboardingStep::TrelloSetup => {
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
}
