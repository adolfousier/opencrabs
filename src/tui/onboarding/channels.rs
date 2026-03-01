use crossterm::event::{KeyCode, KeyEvent};

use super::types::*;
use super::wizard::OnboardingWizard;

impl OnboardingWizard {
    pub(super) fn handle_channels_key(&mut self, event: KeyEvent) -> WizardAction {
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
                            self.reset_whatsapp_state();
                            self.detect_existing_whatsapp_phone();
                            // If already paired, drop the user on the phone field so they
                            // can confirm or change the number without re-scanning QR.
                            // Do NOT mark whatsapp_connected=true — no live client this session.
                            // They can always BackTab to Connection and re-scan if needed.
                            let session_db = crate::config::opencrabs_home()
                                .join("whatsapp")
                                .join("session.db");
                            if session_db.exists() {
                                self.whatsapp_field = WhatsAppField::PhoneAllowlist;
                            } else {
                                self.whatsapp_field = WhatsAppField::Connection;
                            }
                        }
                        3 => {
                            self.step = OnboardingStep::SlackSetup;
                            self.slack_field = SlackField::BotToken;
                            self.channel_test_status = ChannelTestStatus::Idle;
                            self.detect_existing_slack_tokens();
                            self.detect_existing_slack_channel_id();
                            self.detect_existing_slack_allowed_list();
                        }
                        4 => {
                            self.step = OnboardingStep::TrelloSetup;
                            self.trello_field = TrelloField::ApiKey;
                            self.channel_test_status = ChannelTestStatus::Idle;
                            self.detect_existing_trello_credentials();
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
    pub(super) fn is_telegram_enabled(&self) -> bool {
        self.channel_toggles.first().is_some_and(|t| t.1)
    }

    /// Check if Discord channel is enabled (index 1 in channel_toggles)
    pub(super) fn is_discord_enabled(&self) -> bool {
        self.channel_toggles.get(1).is_some_and(|t| t.1)
    }

    /// Check if WhatsApp channel is enabled (index 2 in channel_toggles)
    pub(super) fn is_whatsapp_enabled(&self) -> bool {
        self.channel_toggles.get(2).is_some_and(|t| t.1)
    }

    /// Check if Slack channel is enabled (index 3 in channel_toggles)
    pub(super) fn is_slack_enabled(&self) -> bool {
        self.channel_toggles.get(3).is_some_and(|t| t.1)
    }

    /// Check if Trello channel is enabled (index 4 in channel_toggles)
    pub(super) fn is_trello_enabled(&self) -> bool {
        self.channel_toggles.get(4).is_some_and(|t| t.1)
    }

    pub(super) fn handle_telegram_setup_key(&mut self, event: KeyEvent) -> WizardAction {
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

    pub(super) fn handle_discord_setup_key(&mut self, event: KeyEvent) -> WizardAction {
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

    pub(super) fn handle_whatsapp_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        // Navigation keys always work regardless of test status — user must be able
        // to go back, re-scan, or skip at any point.
        let is_nav = matches!(
            event.code,
            KeyCode::BackTab | KeyCode::Tab | KeyCode::Char('s') | KeyCode::Char('S')
        );
        if is_nav {
            // Clear any test status so navigation doesn't get intercepted below
            self.channel_test_status = ChannelTestStatus::Idle;
        }

        // Handle test status for Enter/result display — only on PhoneAllowlist field
        if self.whatsapp_field == WhatsAppField::PhoneAllowlist && !is_nav {
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
                        return WizardAction::TestWhatsApp;
                    }
                }
                ChannelTestStatus::Testing => {
                    // Block only Enter while test is in-flight; navigation already handled above
                    if event.code == KeyCode::Enter {
                        return WizardAction::None;
                    }
                }
                ChannelTestStatus::Idle => {}
            }
        }

        match self.whatsapp_field {
            WhatsAppField::Connection => match event.code {
                KeyCode::Enter => {
                    if self.whatsapp_connected {
                        self.whatsapp_field = WhatsAppField::PhoneAllowlist;
                        WizardAction::None
                    } else if !self.whatsapp_connecting {
                        self.whatsapp_connecting = true;
                        self.whatsapp_error = None;
                        WizardAction::WhatsAppConnect
                    } else {
                        WizardAction::None
                    }
                }
                KeyCode::Tab => {
                    self.whatsapp_field = WhatsAppField::PhoneAllowlist;
                    WizardAction::None
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
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
                    self.whatsapp_connected = false;
                    self.whatsapp_connecting = false;
                    WizardAction::None
                }
                KeyCode::Tab => {
                    // Tab from phone field wraps back to Connection
                    self.whatsapp_field = WhatsAppField::Connection;
                    self.whatsapp_connected = false;
                    self.whatsapp_connecting = false;
                    WizardAction::None
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.next_step();
                    WizardAction::None
                }
                KeyCode::Enter => {
                    if !self.whatsapp_phone_input.is_empty() {
                        return WizardAction::TestWhatsApp;
                    }
                    self.next_step();
                    WizardAction::None
                }
                _ => WizardAction::None,
            },
        }
    }

    /// Reset WhatsApp pairing state (for entering/re-entering the setup step)
    pub(super) fn reset_whatsapp_state(&mut self) {
        self.whatsapp_qr_text = None;
        self.whatsapp_connecting = false;
        self.whatsapp_connected = false;
        self.whatsapp_error = None;
        self.channel_test_status = ChannelTestStatus::Idle; // never carry over blocking state
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
        self.whatsapp_qr_text = None; // dismiss QR popup
        self.whatsapp_field = WhatsAppField::PhoneAllowlist; // advance to phone field
        self.channel_test_status = ChannelTestStatus::Idle;
    }

    /// Called by app when WhatsApp connection fails
    pub fn set_whatsapp_error(&mut self, err: String) {
        self.whatsapp_error = Some(err);
        self.whatsapp_connecting = false;
    }

    pub(super) fn handle_trello_setup_key(&mut self, event: KeyEvent) -> WizardAction {
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
                    return WizardAction::TestTrello;
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

        match self.trello_field {
            TrelloField::ApiKey => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_trello_api_key() {
                        self.trello_api_key_input.clear();
                    }
                    self.trello_api_key_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_trello_api_key() {
                        self.trello_api_key_input.clear();
                    } else {
                        self.trello_api_key_input.pop();
                    }
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.trello_field = TrelloField::ApiToken;
                }
                _ => {}
            },
            TrelloField::ApiToken => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_trello_api_token() {
                        self.trello_api_token_input.clear();
                    }
                    self.trello_api_token_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_trello_api_token() {
                        self.trello_api_token_input.clear();
                    } else {
                        self.trello_api_token_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.trello_field = TrelloField::ApiKey;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.trello_field = TrelloField::BoardId;
                }
                _ => {}
            },
            TrelloField::BoardId => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_trello_board_id() {
                        self.trello_board_id_input.clear();
                    }
                    self.trello_board_id_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_trello_board_id() {
                        self.trello_board_id_input.clear();
                    } else {
                        self.trello_board_id_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.trello_field = TrelloField::ApiToken;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.trello_field = TrelloField::AllowedUsers;
                }
                _ => {}
            },
            TrelloField::AllowedUsers => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_trello_allowed_users() {
                        self.trello_allowed_users_input.clear();
                    }
                    self.trello_allowed_users_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_trello_allowed_users() {
                        self.trello_allowed_users_input.clear();
                    } else {
                        self.trello_allowed_users_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.trello_field = TrelloField::BoardId;
                }
                KeyCode::Enter => {
                    let has_key = !self.trello_api_key_input.is_empty();
                    let has_token = !self.trello_api_token_input.is_empty();
                    let has_board = !self.trello_board_id_input.is_empty();
                    if has_key && has_token && has_board {
                        return WizardAction::TestTrello;
                    }
                    self.next_step();
                }
                _ => {}
            },
        }
        WizardAction::None
    }

    pub(super) fn handle_slack_setup_key(&mut self, event: KeyEvent) -> WizardAction {
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
}
