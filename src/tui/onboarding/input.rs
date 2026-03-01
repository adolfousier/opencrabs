use crossterm::event::{KeyCode, KeyEvent};

use super::types::*;
use super::wizard::OnboardingWizard;

impl OnboardingWizard {
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
            OnboardingStep::TrelloSetup => self.handle_trello_setup_key(event),
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
            OnboardingStep::TrelloSetup => {
                tracing::debug!(
                    "[paste] Trello pasted ({} chars) field={:?}",
                    clean.len(),
                    self.trello_field
                );
                match self.trello_field {
                    TrelloField::ApiKey => {
                        if self.has_existing_trello_api_key() {
                            self.trello_api_key_input.clear();
                        }
                        self.trello_api_key_input.push_str(clean);
                    }
                    TrelloField::ApiToken => {
                        if self.has_existing_trello_api_token() {
                            self.trello_api_token_input.clear();
                        }
                        self.trello_api_token_input.push_str(clean);
                    }
                    TrelloField::BoardId => {
                        if self.has_existing_trello_board_id() {
                            self.trello_board_id_input.clear();
                        }
                        self.trello_board_id_input.push_str(clean);
                    }
                    TrelloField::AllowedUsers => {
                        if self.has_existing_trello_allowed_users() {
                            self.trello_allowed_users_input.clear();
                        }
                        self.trello_allowed_users_input.push_str(clean);
                    }
                }
            }
            OnboardingStep::WhatsAppSetup
                if self.whatsapp_field == WhatsAppField::PhoneAllowlist =>
            {
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

    pub(super) fn handle_mode_select_key(&mut self, event: KeyEvent) -> WizardAction {
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

    pub(super) fn handle_provider_auth_key(&mut self, event: KeyEvent) -> WizardAction {
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
                    if self.custom_provider_name.is_empty() {
                        self.error_message =
                            Some("Enter a name identifier for this provider".to_string());
                        return WizardAction::None;
                    }
                    self.custom_provider_name = self.custom_provider_name.to_lowercase();
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

    pub(super) fn handle_workspace_key(&mut self, event: KeyEvent) -> WizardAction {
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
    pub(super) fn maybe_fetch_models(&self) -> WizardAction {
        if self.step == OnboardingStep::ProviderAuth
            && self.has_existing_key()
            && self.supports_model_fetch()
        {
            WizardAction::FetchModels
        } else {
            WizardAction::None
        }
    }
}
