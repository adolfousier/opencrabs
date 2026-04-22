use crossterm::event::{KeyCode, KeyEvent};

use super::helpers::handle_text_input;
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
            KeyCode::Char(' ') if self.focused_field < count => {
                let name = &self.channel_toggles[self.focused_field].0;
                let new_val = !self.channel_toggles[self.focused_field].1;
                tracing::debug!("[channels] toggled '{}' → {}", name, new_val);
                self.channel_toggles[self.focused_field].1 = new_val;
            }
            KeyCode::Enter => {
                if self.focused_field >= count {
                    // "Continue" button — advance past channels
                    tracing::debug!("[channels] Continue pressed, advancing");
                    self.next_step();
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
                            self.detect_existing_respond_to();
                        }
                        1 => {
                            self.step = OnboardingStep::DiscordSetup;
                            self.discord_field = DiscordField::BotToken;
                            self.channel_test_status = ChannelTestStatus::Idle;
                            self.detect_existing_discord_token();
                            self.detect_existing_discord_channel_id();
                            self.detect_existing_discord_allowed_list();
                            self.detect_existing_respond_to();
                        }
                        2 => {
                            self.step = OnboardingStep::WhatsAppSetup;
                            self.reset_whatsapp_state();
                            self.detect_existing_whatsapp_phone();
                            // Always start on Connection field so the user can see
                            // the reset option (R) when already paired, or scan QR
                            // when not paired. Tab advances to PhoneAllowlist.
                            self.whatsapp_field = WhatsAppField::Connection;
                        }
                        3 => {
                            self.step = OnboardingStep::SlackSetup;
                            self.slack_field = SlackField::BotToken;
                            self.channel_test_status = ChannelTestStatus::Idle;
                            self.detect_existing_slack_tokens();
                            self.detect_existing_slack_channel_id();
                            self.detect_existing_slack_allowed_list();
                            self.detect_existing_respond_to();
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
                self.next_step();
            }
            _ => {}
        }
        WizardAction::None
    }

    /// Check if Telegram channel is enabled (index 0 in channel_toggles)
    pub fn is_telegram_enabled(&self) -> bool {
        self.channel_toggles.first().is_some_and(|t| t.1)
    }

    /// Check if Discord channel is enabled (index 1 in channel_toggles)
    pub fn is_discord_enabled(&self) -> bool {
        self.channel_toggles.get(1).is_some_and(|t| t.1)
    }

    /// Check if WhatsApp channel is enabled (index 2 in channel_toggles)
    pub fn is_whatsapp_enabled(&self) -> bool {
        self.channel_toggles.get(2).is_some_and(|t| t.1)
    }

    /// Check if Slack channel is enabled (index 3 in channel_toggles)
    pub fn is_slack_enabled(&self) -> bool {
        self.channel_toggles.get(3).is_some_and(|t| t.1)
    }

    /// Check if Trello channel is enabled (index 4 in channel_toggles)
    pub fn is_trello_enabled(&self) -> bool {
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

        let existing_token = self.has_existing_telegram_token();
        let existing_uid = self.has_existing_telegram_user_id();
        match self.telegram_field {
            TelegramField::BotToken => {
                if handle_text_input(
                    &event,
                    &mut self.telegram_token_input,
                    &mut self.channel_input_cursor,
                    existing_token,
                    None,
                ) {
                    // consumed
                } else {
                    match event.code {
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.telegram_field = TelegramField::UserID;
                            self.channel_input_cursor = self.telegram_user_id_input.len();
                        }
                        _ => {}
                    }
                }
            }
            TelegramField::UserID => {
                if handle_text_input(
                    &event,
                    &mut self.telegram_user_id_input,
                    &mut self.channel_input_cursor,
                    existing_uid,
                    Some(|c: char| c.is_ascii_digit()),
                ) {
                    // consumed
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.telegram_field = TelegramField::BotToken;
                            self.channel_input_cursor = self.telegram_token_input.len();
                        }
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.telegram_field = TelegramField::RespondTo;
                        }
                        _ => {}
                    }
                }
            }
            TelegramField::RespondTo => match event.code {
                KeyCode::Left | KeyCode::Char('h') => {
                    self.telegram_respond_to = self.telegram_respond_to.saturating_sub(1);
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                    self.telegram_respond_to = (self.telegram_respond_to + 1).min(2);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.telegram_field = TelegramField::UserID;
                }
                KeyCode::Enter => {
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

        let existing_d_token = self.has_existing_discord_token();
        let existing_d_ch = self.has_existing_discord_channel_id();
        let existing_d_al = self.has_existing_discord_allowed_list();
        match self.discord_field {
            DiscordField::BotToken => {
                if handle_text_input(
                    &event,
                    &mut self.discord_token_input,
                    &mut self.channel_input_cursor,
                    existing_d_token,
                    None,
                ) {
                    // consumed
                } else {
                    match event.code {
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.discord_field = DiscordField::ChannelID;
                            self.channel_input_cursor = self.discord_channel_id_input.len();
                        }
                        _ => {}
                    }
                }
            }
            DiscordField::ChannelID => {
                if handle_text_input(
                    &event,
                    &mut self.discord_channel_id_input,
                    &mut self.channel_input_cursor,
                    existing_d_ch,
                    None,
                ) {
                    // consumed
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.discord_field = DiscordField::BotToken;
                            self.channel_input_cursor = self.discord_token_input.len();
                        }
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.discord_field = DiscordField::AllowedList;
                            self.channel_input_cursor = self.discord_allowed_list_input.len();
                        }
                        _ => {}
                    }
                }
            }
            DiscordField::AllowedList => {
                if handle_text_input(
                    &event,
                    &mut self.discord_allowed_list_input,
                    &mut self.channel_input_cursor,
                    existing_d_al,
                    Some(|c: char| c.is_ascii_digit()),
                ) {
                    // consumed
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.discord_field = DiscordField::ChannelID;
                            self.channel_input_cursor = self.discord_channel_id_input.len();
                        }
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.discord_field = DiscordField::RespondTo;
                        }
                        _ => {}
                    }
                }
            }
            DiscordField::RespondTo => match event.code {
                KeyCode::Left | KeyCode::Char('h') => {
                    self.discord_respond_to = self.discord_respond_to.saturating_sub(1);
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                    self.discord_respond_to = (self.discord_respond_to + 1).min(2);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.discord_field = DiscordField::AllowedList;
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
            KeyCode::BackTab
                | KeyCode::Up
                | KeyCode::Tab
                | KeyCode::Down
                | KeyCode::Char('s')
                | KeyCode::Char('S')
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
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    if !self.whatsapp_connecting {
                        // Delete session.db to force fresh QR pairing
                        let wa_dir = crate::config::opencrabs_home().join("whatsapp");
                        let _ = std::fs::remove_file(wa_dir.join("session.db"));
                        let _ = std::fs::remove_file(wa_dir.join("session.db-wal"));
                        let _ = std::fs::remove_file(wa_dir.join("session.db-shm"));
                        self.reset_whatsapp_state();
                        self.whatsapp_connecting = true;
                        WizardAction::WhatsAppConnect
                    } else {
                        WizardAction::None
                    }
                }
                KeyCode::Tab | KeyCode::Down => {
                    self.whatsapp_field = WhatsAppField::PhoneAllowlist;
                    WizardAction::None
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.next_step();
                    WizardAction::None
                }
                _ => WizardAction::None,
            },
            WhatsAppField::PhoneAllowlist => {
                let existing_phone = self.has_existing_whatsapp_phone();
                if handle_text_input(
                    &event,
                    &mut self.whatsapp_phone_input,
                    &mut self.channel_input_cursor,
                    existing_phone,
                    Some(|c: char| c.is_ascii_digit() || c == '+' || c == '-' || c == ' '),
                ) {
                    WizardAction::None
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.whatsapp_field = WhatsAppField::Connection;
                            self.whatsapp_connected = false;
                            self.whatsapp_connecting = false;
                            WizardAction::None
                        }
                        KeyCode::Tab | KeyCode::Down => {
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
                    }
                }
            }
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

        let existing_t_ak = self.has_existing_trello_api_key();
        let existing_t_at = self.has_existing_trello_api_token();
        let existing_t_bd = self.has_existing_trello_board_id();
        let existing_t_au = self.has_existing_trello_allowed_users();
        match self.trello_field {
            TrelloField::ApiKey => {
                if handle_text_input(
                    &event,
                    &mut self.trello_api_key_input,
                    &mut self.channel_input_cursor,
                    existing_t_ak,
                    None,
                ) {
                } else {
                    match event.code {
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.trello_field = TrelloField::ApiToken;
                            self.channel_input_cursor = self.trello_api_token_input.len();
                        }
                        _ => {}
                    }
                }
            }
            TrelloField::ApiToken => {
                if handle_text_input(
                    &event,
                    &mut self.trello_api_token_input,
                    &mut self.channel_input_cursor,
                    existing_t_at,
                    None,
                ) {
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.trello_field = TrelloField::ApiKey;
                            self.channel_input_cursor = self.trello_api_key_input.len();
                        }
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.trello_field = TrelloField::BoardId;
                            self.channel_input_cursor = self.trello_board_id_input.len();
                        }
                        _ => {}
                    }
                }
            }
            TrelloField::BoardId => {
                if handle_text_input(
                    &event,
                    &mut self.trello_board_id_input,
                    &mut self.channel_input_cursor,
                    existing_t_bd,
                    None,
                ) {
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.trello_field = TrelloField::ApiToken;
                            self.channel_input_cursor = self.trello_api_token_input.len();
                        }
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.trello_field = TrelloField::AllowedUsers;
                            self.channel_input_cursor = self.trello_allowed_users_input.len();
                        }
                        _ => {}
                    }
                }
            }
            TrelloField::AllowedUsers => {
                if handle_text_input(
                    &event,
                    &mut self.trello_allowed_users_input,
                    &mut self.channel_input_cursor,
                    existing_t_au,
                    None,
                ) {
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.trello_field = TrelloField::BoardId;
                            self.channel_input_cursor = self.trello_board_id_input.len();
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
                    }
                }
            }
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

        let existing_s_bot = self.has_existing_slack_bot_token();
        let existing_s_app = self.has_existing_slack_app_token();
        let existing_s_ch = self.has_existing_slack_channel_id();
        let existing_s_al = self.has_existing_slack_allowed_list();
        match self.slack_field {
            SlackField::BotToken => {
                if handle_text_input(
                    &event,
                    &mut self.slack_bot_token_input,
                    &mut self.channel_input_cursor,
                    existing_s_bot,
                    None,
                ) {
                } else {
                    match event.code {
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.slack_field = SlackField::AppToken;
                            self.channel_input_cursor = self.slack_app_token_input.len();
                        }
                        _ => {}
                    }
                }
            }
            SlackField::AppToken => {
                if handle_text_input(
                    &event,
                    &mut self.slack_app_token_input,
                    &mut self.channel_input_cursor,
                    existing_s_app,
                    None,
                ) {
                } else {
                    match event.code {
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.slack_field = SlackField::ChannelID;
                            self.channel_input_cursor = self.slack_channel_id_input.len();
                        }
                        KeyCode::BackTab | KeyCode::Up => {
                            self.slack_field = SlackField::BotToken;
                            self.channel_input_cursor = self.slack_bot_token_input.len();
                        }
                        _ => {}
                    }
                }
            }
            SlackField::ChannelID => {
                if handle_text_input(
                    &event,
                    &mut self.slack_channel_id_input,
                    &mut self.channel_input_cursor,
                    existing_s_ch,
                    None,
                ) {
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.slack_field = SlackField::AppToken;
                            self.channel_input_cursor = self.slack_app_token_input.len();
                        }
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.slack_field = SlackField::AllowedList;
                            self.channel_input_cursor = self.slack_allowed_list_input.len();
                        }
                        _ => {}
                    }
                }
            }
            SlackField::AllowedList => {
                if handle_text_input(
                    &event,
                    &mut self.slack_allowed_list_input,
                    &mut self.channel_input_cursor,
                    existing_s_al,
                    None,
                ) {
                } else {
                    match event.code {
                        KeyCode::BackTab | KeyCode::Up => {
                            self.slack_field = SlackField::ChannelID;
                            self.channel_input_cursor = self.slack_channel_id_input.len();
                        }
                        KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                            self.slack_field = SlackField::RespondTo;
                        }
                        _ => {}
                    }
                }
            }
            SlackField::RespondTo => match event.code {
                KeyCode::Left | KeyCode::Char('h') => {
                    self.slack_respond_to = self.slack_respond_to.saturating_sub(1);
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                    self.slack_respond_to = (self.slack_respond_to + 1).min(2);
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.slack_field = SlackField::AllowedList;
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
