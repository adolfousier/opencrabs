//! Tests for onboarding field-level navigation improvements (issue #43).
//!
//! Covers Shift+Tab (BackTab) backward navigation and Ctrl/Alt+Backspace
//! field clearing across all provider auth and channel setup screens.

use crate::tui::onboarding::{
    AuthField, DiscordField, OnboardingStep, OnboardingWizard, SlackField, TelegramField,
    TrelloField,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ── Helpers ────────────────────────────────────────────────────

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn ctrl_backspace() -> KeyEvent {
    KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)
}

fn alt_backspace() -> KeyEvent {
    KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT)
}

fn clean_wizard() -> OnboardingWizard {
    let mut w = OnboardingWizard::new();
    w.selected_provider = 0;
    w.api_key_input = String::new();
    w.custom_base_url = String::new();
    w.custom_model = String::new();
    w
}

// ── is_clear_field via public behaviour ────────────────────────

#[test]
fn ctrl_backspace_clears_api_key_field() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.auth_field = AuthField::ApiKey;
    w.api_key_input = "sk-some-long-key-here".to_string();

    w.handle_key(ctrl_backspace());
    assert!(
        w.api_key_input.is_empty(),
        "Ctrl+Backspace should clear the field"
    );
}

#[test]
fn alt_backspace_clears_api_key_field() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.auth_field = AuthField::ApiKey;
    w.api_key_input = "sk-some-long-key-here".to_string();

    w.handle_key(alt_backspace());
    assert!(
        w.api_key_input.is_empty(),
        "Alt+Backspace should clear the field"
    );
}

#[test]
fn plain_backspace_deletes_single_char() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.auth_field = AuthField::ApiKey;
    w.api_key_input = "abc".to_string();

    w.handle_key(key(KeyCode::Backspace));
    assert_eq!(
        w.api_key_input, "ab",
        "Plain backspace should delete one char"
    );
}

// ── Provider auth: BackTab backward navigation ─────────────────

#[test]
fn backtab_custom_provider_full_reverse_chain() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.selected_provider = 6; // Custom OpenAI-Compatible
    w.auth_field = AuthField::CustomContextWindow;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.auth_field, AuthField::CustomModel);

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.auth_field, AuthField::CustomApiKey);

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.auth_field, AuthField::CustomBaseUrl);

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.auth_field, AuthField::CustomName);

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.auth_field, AuthField::Provider);
}

// ── Provider auth: Ctrl+Backspace on custom fields ─────────────

#[test]
fn ctrl_backspace_clears_custom_name() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.selected_provider = 6;
    w.auth_field = AuthField::CustomName;
    w.custom_provider_name = "My Provider".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.custom_provider_name.is_empty());
}

#[test]
fn ctrl_backspace_clears_custom_base_url() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.selected_provider = 6;
    w.auth_field = AuthField::CustomBaseUrl;
    w.custom_base_url = "https://api.example.com".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.custom_base_url.is_empty());
}

#[test]
fn ctrl_backspace_clears_custom_api_key() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.selected_provider = 6;
    w.auth_field = AuthField::CustomApiKey;
    w.api_key_input = "sk-custom-key".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.api_key_input.is_empty());
}

#[test]
fn ctrl_backspace_clears_custom_model() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.selected_provider = 6;
    w.auth_field = AuthField::CustomModel;
    w.custom_model = "gpt-4o-mini".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.custom_model.is_empty());
}

#[test]
fn ctrl_backspace_clears_custom_context_window() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.selected_provider = 6;
    w.auth_field = AuthField::CustomContextWindow;
    w.custom_context_window = "128000".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.custom_context_window.is_empty());
}

// ── Telegram: BackTab backward navigation ──────────────────────

#[test]
fn telegram_backtab_userid_to_bottoken() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TelegramSetup;
    w.telegram_field = TelegramField::UserID;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.telegram_field, TelegramField::BotToken);
}

#[test]
fn telegram_backtab_respondto_to_userid() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TelegramSetup;
    w.telegram_field = TelegramField::RespondTo;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.telegram_field, TelegramField::UserID);
}

// ── Telegram: Ctrl+Backspace clears fields ─────────────────────

#[test]
fn telegram_ctrl_backspace_clears_bot_token() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TelegramSetup;
    w.telegram_field = TelegramField::BotToken;
    w.telegram_token_input = "123456:ABC-DEF".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.telegram_token_input.is_empty());
}

#[test]
fn telegram_ctrl_backspace_clears_user_id() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TelegramSetup;
    w.telegram_field = TelegramField::UserID;
    w.telegram_user_id_input = "12345678".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.telegram_user_id_input.is_empty());
}

// ── Discord: BackTab backward navigation ───────────────────────

#[test]
fn discord_backtab_channelid_to_bottoken() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::DiscordSetup;
    w.discord_field = DiscordField::ChannelID;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.discord_field, DiscordField::BotToken);
}

#[test]
fn discord_backtab_allowedlist_to_channelid() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::DiscordSetup;
    w.discord_field = DiscordField::AllowedList;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.discord_field, DiscordField::ChannelID);
}

#[test]
fn discord_backtab_respondto_to_allowedlist() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::DiscordSetup;
    w.discord_field = DiscordField::RespondTo;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.discord_field, DiscordField::AllowedList);
}

// ── Discord: Ctrl+Backspace clears fields ──────────────────────

#[test]
fn discord_ctrl_backspace_clears_bot_token() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::DiscordSetup;
    w.discord_field = DiscordField::BotToken;
    w.discord_token_input = "bot-token-here".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.discord_token_input.is_empty());
}

#[test]
fn discord_ctrl_backspace_clears_channel_id() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::DiscordSetup;
    w.discord_field = DiscordField::ChannelID;
    w.discord_channel_id_input = "123456789".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.discord_channel_id_input.is_empty());
}

#[test]
fn discord_ctrl_backspace_clears_allowed_list() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::DiscordSetup;
    w.discord_field = DiscordField::AllowedList;
    w.discord_allowed_list_input = "user1,user2".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.discord_allowed_list_input.is_empty());
}

// ── Slack: BackTab backward navigation ─────────────────────────

#[test]
fn slack_backtab_apptoken_to_bottoken() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::SlackSetup;
    w.slack_field = SlackField::AppToken;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.slack_field, SlackField::BotToken);
}

#[test]
fn slack_backtab_channelid_to_apptoken() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::SlackSetup;
    w.slack_field = SlackField::ChannelID;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.slack_field, SlackField::AppToken);
}

#[test]
fn slack_backtab_allowedlist_to_channelid() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::SlackSetup;
    w.slack_field = SlackField::AllowedList;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.slack_field, SlackField::ChannelID);
}

#[test]
fn slack_backtab_respondto_to_allowedlist() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::SlackSetup;
    w.slack_field = SlackField::RespondTo;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.slack_field, SlackField::AllowedList);
}

// ── Slack: Ctrl+Backspace clears fields ────────────────────────

#[test]
fn slack_ctrl_backspace_clears_bot_token() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::SlackSetup;
    w.slack_field = SlackField::BotToken;
    w.slack_bot_token_input = "xoxb-token".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.slack_bot_token_input.is_empty());
}

#[test]
fn slack_ctrl_backspace_clears_app_token() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::SlackSetup;
    w.slack_field = SlackField::AppToken;
    w.slack_app_token_input = "xapp-token".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.slack_app_token_input.is_empty());
}

#[test]
fn slack_ctrl_backspace_clears_channel_id() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::SlackSetup;
    w.slack_field = SlackField::ChannelID;
    w.slack_channel_id_input = "C01234567".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.slack_channel_id_input.is_empty());
}

#[test]
fn slack_ctrl_backspace_clears_allowed_list() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::SlackSetup;
    w.slack_field = SlackField::AllowedList;
    w.slack_allowed_list_input = "U123,U456".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.slack_allowed_list_input.is_empty());
}

// ── Trello: BackTab backward navigation ────────────────────────

#[test]
fn trello_backtab_apitoken_to_apikey() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TrelloSetup;
    w.trello_field = TrelloField::ApiToken;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.trello_field, TrelloField::ApiKey);
}

#[test]
fn trello_backtab_boardid_to_apitoken() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TrelloSetup;
    w.trello_field = TrelloField::BoardId;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.trello_field, TrelloField::ApiToken);
}

#[test]
fn trello_backtab_allowedusers_to_boardid() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TrelloSetup;
    w.trello_field = TrelloField::AllowedUsers;

    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.trello_field, TrelloField::BoardId);
}

// ── Trello: Ctrl+Backspace clears fields ───────────────────────

#[test]
fn trello_ctrl_backspace_clears_api_key() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TrelloSetup;
    w.trello_field = TrelloField::ApiKey;
    w.trello_api_key_input = "trello-key".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.trello_api_key_input.is_empty());
}

#[test]
fn trello_ctrl_backspace_clears_api_token() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TrelloSetup;
    w.trello_field = TrelloField::ApiToken;
    w.trello_api_token_input = "trello-token".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.trello_api_token_input.is_empty());
}

#[test]
fn trello_ctrl_backspace_clears_board_id() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TrelloSetup;
    w.trello_field = TrelloField::BoardId;
    w.trello_board_id_input = "board-123".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.trello_board_id_input.is_empty());
}

#[test]
fn trello_ctrl_backspace_clears_allowed_users() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TrelloSetup;
    w.trello_field = TrelloField::AllowedUsers;
    w.trello_allowed_users_input = "user-456".to_string();

    w.handle_key(ctrl_backspace());
    assert!(w.trello_allowed_users_input.is_empty());
}

// ── Alt+Backspace works same as Ctrl+Backspace ─────────────────

#[test]
fn alt_backspace_clears_telegram_token_input() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::TelegramSetup;
    w.telegram_field = TelegramField::BotToken;
    w.telegram_token_input = "123456:ABC".to_string();

    w.handle_key(alt_backspace());
    assert!(w.telegram_token_input.is_empty());
}

#[test]
fn alt_backspace_clears_discord_token_input() {
    let mut w = clean_wizard();
    w.step = OnboardingStep::DiscordSetup;
    w.discord_field = DiscordField::BotToken;
    w.discord_token_input = "discord-token".to_string();

    w.handle_key(alt_backspace());
    assert!(w.discord_token_input.is_empty());
}
