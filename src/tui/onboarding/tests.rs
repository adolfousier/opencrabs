use super::*;
use crossterm::event::{KeyCode, KeyEvent};

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
    wizard.ps.api_key_input = "test-key".to_string();

    assert_eq!(wizard.step, OnboardingStep::ModeSelect);
    wizard.next_step(); // ModeSelect -> Workspace
    assert_eq!(wizard.step, OnboardingStep::Workspace);
}

#[test]
fn test_advanced_mode_all_steps() {
    let mut wizard = OnboardingWizard::new();
    wizard.mode = WizardMode::Advanced;

    wizard.next_step(); // ModeSelect -> Workspace
    assert_eq!(wizard.step, OnboardingStep::Workspace);
    wizard.next_step(); // Workspace -> ProviderAuth (detect_existing_key clears api_key_input)
    assert_eq!(wizard.step, OnboardingStep::ProviderAuth);
    wizard.ps.api_key_input = "test-key".to_string(); // set key AFTER reaching ProviderAuth
    wizard.next_step(); // ProviderAuth -> Channels
    assert_eq!(wizard.step, OnboardingStep::Channels);
    wizard.next_step(); // Channels -> VoiceSetup
    assert_eq!(wizard.step, OnboardingStep::VoiceSetup);
    wizard.next_step(); // VoiceSetup -> ImageSetup (Advanced)
    assert_eq!(wizard.step, OnboardingStep::ImageSetup);
    wizard.next_step(); // ImageSetup -> Daemon
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

    // Continue to VoiceSetup
    wizard.focused_field = wizard.channel_toggles.len();
    wizard.handle_key(key(KeyCode::Enter));
    assert_eq!(wizard.step, OnboardingStep::VoiceSetup);
}

#[test]
fn test_channels_whatsapp_skips_to_voice() {
    let mut wizard = OnboardingWizard::new();
    wizard.mode = WizardMode::Advanced;

    wizard.next_step(); // ModeSelect -> Workspace
    wizard.next_step(); // Workspace -> ProviderAuth
    wizard.ps.api_key_input = "test-key".to_string();
    wizard.next_step(); // ProviderAuth -> Channels

    // Enable WhatsApp only (no token sub-step)
    wizard.channel_toggles[2].1 = true;
    wizard.next_step(); // Channels -> VoiceSetup (WhatsApp has no sub-step)
    assert_eq!(wizard.step, OnboardingStep::VoiceSetup);
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

    // Continue to VoiceSetup
    wizard.focused_field = wizard.channel_toggles.len();
    wizard.handle_key(key(KeyCode::Enter));
    assert_eq!(wizard.step, OnboardingStep::VoiceSetup);
}

#[test]
fn test_voice_setup_defaults() {
    let wizard = OnboardingWizard::new();
    assert!(wizard.groq_api_key_input.is_empty());
    assert!(!wizard.tts_enabled);
    assert_eq!(wizard.voice_field, VoiceField::SttModeSelect);
}

#[test]
fn test_step_numbers() {
    assert_eq!(OnboardingStep::ModeSelect.number(), 1);
    assert_eq!(OnboardingStep::Channels.number(), 4);
    assert_eq!(OnboardingStep::TelegramSetup.number(), 4); // sub-step of Channels
    assert_eq!(OnboardingStep::VoiceSetup.number(), 5);
    assert_eq!(OnboardingStep::ImageSetup.number(), 6);
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
    assert_eq!(wizard.ps.selected_provider, 0);
    assert_eq!(wizard.auth_field, AuthField::Provider);
    assert!(wizard.ps.api_key_input.is_empty());
    assert_eq!(wizard.ps.selected_model, 0);
    // First provider is Anthropic Claude
    assert_eq!(
        PROVIDERS[wizard.ps.selected_provider].name,
        "Anthropic Claude"
    );
    assert!(!PROVIDERS[wizard.ps.selected_provider].help_lines.is_empty());
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
    assert!(toggle_names.contains(&"Trello"));
}

/// Create a wizard with clean defaults (no config auto-detection).
/// `OnboardingWizard::new()` loads existing config from disk, which
/// pollutes provider/brain fields when a real config exists.
fn clean_wizard() -> OnboardingWizard {
    let mut w = OnboardingWizard::new();
    w.ps.selected_provider = 0;
    w.ps.api_key_input = String::new();
    w.ps.base_url = String::new();
    w.ps.custom_model = String::new();
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
    assert_eq!(wizard.ps.selected_provider, 0);

    wizard.handle_key(key(KeyCode::Down));
    assert_eq!(wizard.ps.selected_provider, 7); // Claude CLI (next alphabetically after Anthropic)

    wizard.handle_key(key(KeyCode::Up));
    assert_eq!(wizard.ps.selected_provider, 0);

    // Can't go below 0
    wizard.handle_key(key(KeyCode::Up));
    assert_eq!(wizard.ps.selected_provider, 0);
}

#[test]
fn test_handle_key_api_key_typing() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::ProviderAuth;
    wizard.auth_field = AuthField::Provider;

    // Enter to select provider -> goes to ApiKey field
    wizard.handle_key(key(KeyCode::Enter));
    assert_eq!(wizard.auth_field, AuthField::ApiKey);

    // Type a key
    wizard.handle_key(key(KeyCode::Char('s')));
    wizard.handle_key(key(KeyCode::Char('k')));
    assert_eq!(wizard.ps.api_key_input, "sk");

    // Backspace
    wizard.handle_key(key(KeyCode::Backspace));
    assert_eq!(wizard.ps.api_key_input, "s");
}

#[test]
fn test_handle_key_provider_auth_field_flow() {
    let mut wizard = clean_wizard();
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

    wizard.next_step(); // ModeSelect -> Workspace
    assert_eq!(wizard.step, OnboardingStep::Workspace);
    wizard.next_step(); // Workspace -> ProviderAuth
    assert_eq!(wizard.step, OnboardingStep::ProviderAuth);
    wizard.ps.api_key_input = "test-key".to_string();
    wizard.next_step(); // ProviderAuth -> Daemon (QuickStart skips Channels & Voice)
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
            .is_some_and(|m| m.contains("required"))
    );
}

#[test]
fn test_model_selection() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::ProviderAuth;
    wizard.auth_field = AuthField::Model;
    // Set up config models for selection testing
    wizard.ps.config_models = vec!["model-a".into(), "model-b".into(), "model-c".into()];

    assert_eq!(wizard.ps.selected_model, 0);
    wizard.handle_key(key(KeyCode::Down));
    assert_eq!(wizard.ps.selected_model, 1);
    wizard.handle_key(key(KeyCode::Down));
    assert_eq!(wizard.ps.selected_model, 2);
    // Should clamp to max
    for _ in 0..20 {
        wizard.handle_key(key(KeyCode::Down));
    }
    // Provider selection stays within bounds (7 static + existing custom providers)
    let max_idx = PROVIDERS.len() + wizard.ps.custom_names.len();
    assert!(wizard.ps.selected_provider < max_idx);
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
    // OpenRouter is index 4, Custom is last
    assert_eq!(PROVIDERS[4].name, "OpenRouter");
    assert_eq!(PROVIDERS.last().unwrap().name, "Custom OpenAI-Compatible");
}

#[test]
fn test_model_count_uses_fetched_when_available() {
    let mut wizard = OnboardingWizard::new();
    // Clear any models loaded from existing config
    wizard.ps.config_models.clear();
    wizard.ps.models.clear();
    // Anthropic (0) has no static models — fetched from API
    wizard.ps.selected_provider = 0;
    assert_eq!(wizard.ps.model_count(), 0);

    // After fetching
    wizard.ps.models = vec![
        "model-a".into(),
        "model-b".into(),
        "model-c".into(),
        "model-d".into(),
    ];
    assert_eq!(wizard.ps.model_count(), 4);
}

#[test]
fn test_selected_model_name_uses_fetched() {
    let mut wizard = OnboardingWizard::new();
    // No static models - should use fetched or show placeholder
    assert!(wizard.ps.selected_model_name().is_empty() || wizard.ps.models.is_empty());

    wizard.ps.models = vec!["live-model-1".into(), "live-model-2".into()];
    wizard.ps.selected_model = 1;
    assert_eq!(wizard.ps.selected_model_name(), "live-model-2");
}

#[test]
fn test_supports_model_fetch() {
    let mut wizard = OnboardingWizard::new();
    wizard.ps.selected_provider = 0; // Anthropic
    assert!(wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 1; // OpenAI
    assert!(wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 2; // GitHub Copilot (has /models endpoint)
    assert!(wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 3; // Gemini
    assert!(wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 4; // OpenRouter
    assert!(wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 5; // Minimax
    assert!(!wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 6; // z.ai GLM (supports fetch)
    assert!(wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 7; // Claude CLI
    assert!(!wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 8; // OpenCode CLI (supports fetch)
    assert!(wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 9; // Qwen CLI — static models only, no live fetch
    assert!(!wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 10; // Qwen native — static `coder-model` only
    assert!(!wizard.ps.supports_model_fetch());
    wizard.ps.selected_provider = 11; // Custom
    assert!(!wizard.ps.supports_model_fetch());
}

#[test]
fn test_fetch_models_unsupported_provider_returns_empty() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(fetch_provider_models(99, None, None));
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
    let models = rt.block_on(fetch_provider_models(0, Some(&key), None));
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
    let models = rt.block_on(fetch_provider_models(0, Some(&key), None));
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
    let models = rt.block_on(fetch_provider_models(1, Some(&key), None));
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
    let models = rt.block_on(fetch_provider_models(4, Some(&key), None));
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
        None,
    ));
    assert!(
        models.is_empty(),
        "Bad key should return empty, got {} models",
        models.len()
    );
}

// ── handle_text_input / handle_text_paste tests ──

use super::helpers::{handle_text_input, handle_text_paste};

fn key_mod(code: KeyCode, modifiers: crossterm::event::KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn test_text_input_char_insert_at_cursor() {
    let mut buf = "hello".to_string();
    let mut cursor = 3; // between 'l' and 'l'
    let event = key(KeyCode::Char('X'));
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(buf, "helXlo");
    assert_eq!(cursor, 4);
}

#[test]
fn test_text_input_backspace_at_cursor() {
    let mut buf = "hello".to_string();
    let mut cursor = 3;
    let event = key(KeyCode::Backspace);
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(buf, "helo");
    assert_eq!(cursor, 2);
}

#[test]
fn test_text_input_backspace_at_start_noop() {
    let mut buf = "hello".to_string();
    let mut cursor = 0;
    let event = key(KeyCode::Backspace);
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(buf, "hello");
    assert_eq!(cursor, 0);
}

#[test]
fn test_text_input_delete_at_cursor() {
    let mut buf = "hello".to_string();
    let mut cursor = 2;
    let event = key(KeyCode::Delete);
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(buf, "helo");
    assert_eq!(cursor, 2);
}

#[test]
fn test_text_input_delete_at_end_noop() {
    let mut buf = "hello".to_string();
    let mut cursor = 5;
    let event = key(KeyCode::Delete);
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(buf, "hello");
    assert_eq!(cursor, 5);
}

#[test]
fn test_text_input_left_right_cursor() {
    let mut buf = "abc".to_string();
    let mut cursor = 2;

    let left = key(KeyCode::Left);
    assert!(handle_text_input(&left, &mut buf, &mut cursor, false, None));
    assert_eq!(cursor, 1);

    let right = key(KeyCode::Right);
    assert!(handle_text_input(
        &right,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(cursor, 2);
}

#[test]
fn test_text_input_home_end() {
    let mut buf = "hello world".to_string();
    let mut cursor = 5;

    let home = key(KeyCode::Home);
    assert!(handle_text_input(&home, &mut buf, &mut cursor, false, None));
    assert_eq!(cursor, 0);

    let end = key(KeyCode::End);
    assert!(handle_text_input(&end, &mut buf, &mut cursor, false, None));
    assert_eq!(cursor, buf.len());
}

#[test]
fn test_text_input_sentinel_clears_on_char() {
    let mut buf = "__EXISTING_KEY__".to_string();
    let mut cursor = 16;
    let event = key(KeyCode::Char('a'));
    assert!(handle_text_input(&event, &mut buf, &mut cursor, true, None));
    assert_eq!(buf, "a");
    assert_eq!(cursor, 1);
}

#[test]
fn test_text_input_sentinel_clears_on_backspace() {
    let mut buf = "__EXISTING_KEY__".to_string();
    let mut cursor = 16;
    let event = key(KeyCode::Backspace);
    assert!(handle_text_input(&event, &mut buf, &mut cursor, true, None));
    assert_eq!(buf, "");
    assert_eq!(cursor, 0);
}

#[test]
fn test_text_input_sentinel_clears_on_delete() {
    let mut buf = "__EXISTING_KEY__".to_string();
    let mut cursor = 0;
    let event = key(KeyCode::Delete);
    assert!(handle_text_input(&event, &mut buf, &mut cursor, true, None));
    assert_eq!(buf, "");
    assert_eq!(cursor, 0);
}

#[test]
fn test_text_input_ctrl_backspace_clears_all() {
    let mut buf = "hello world".to_string();
    let mut cursor = 5;
    let event = key_mod(KeyCode::Backspace, crossterm::event::KeyModifiers::CONTROL);
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(buf, "");
    assert_eq!(cursor, 0);
}

#[test]
fn test_text_input_char_filter() {
    let mut buf = String::new();
    let mut cursor = 0;
    let digits_only: Option<fn(char) -> bool> = Some(|c: char| c.is_ascii_digit());

    // Letter rejected
    let event = key(KeyCode::Char('a'));
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        digits_only
    ));
    assert_eq!(buf, "");

    // Digit accepted
    let event = key(KeyCode::Char('5'));
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        digits_only
    ));
    assert_eq!(buf, "5");
    assert_eq!(cursor, 1);
}

#[test]
fn test_text_input_enter_not_consumed() {
    let mut buf = "hello".to_string();
    let mut cursor = 5;
    let event = key(KeyCode::Enter);
    assert!(!handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(buf, "hello"); // unchanged
}

#[test]
fn test_text_input_word_jump_ctrl_left() {
    let mut buf = "hello world foo".to_string();
    let mut cursor = 15; // end
    let event = key_mod(KeyCode::Left, crossterm::event::KeyModifiers::CONTROL);
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(cursor, 12); // start of "foo"
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(cursor, 6); // start of "world"
}

#[test]
fn test_text_input_word_jump_ctrl_right() {
    let mut buf = "hello world foo".to_string();
    let mut cursor = 0;
    let event = key_mod(KeyCode::Right, crossterm::event::KeyModifiers::CONTROL);
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(cursor, 6); // after "hello "
    assert!(handle_text_input(
        &event,
        &mut buf,
        &mut cursor,
        false,
        None
    ));
    assert_eq!(cursor, 12); // after "world "
}

// ── handle_text_paste tests ──

#[test]
fn test_text_paste_at_cursor() {
    let mut buf = "helo".to_string();
    let mut cursor = 2;
    handle_text_paste("ll", &mut buf, &mut cursor, false, None);
    assert_eq!(buf, "helllo");
    assert_eq!(cursor, 4);
}

#[test]
fn test_text_paste_sentinel_clears_first() {
    let mut buf = "__EXISTING_KEY__".to_string();
    let mut cursor = 16;
    handle_text_paste("new-token-123", &mut buf, &mut cursor, true, None);
    assert_eq!(buf, "new-token-123");
    assert_eq!(cursor, 13);
}

#[test]
fn test_text_paste_with_filter() {
    let mut buf = String::new();
    let mut cursor = 0;
    handle_text_paste(
        "abc123def456",
        &mut buf,
        &mut cursor,
        false,
        Some(|c: char| c.is_ascii_digit()),
    );
    assert_eq!(buf, "123456");
    assert_eq!(cursor, 6);
}

#[test]
fn test_text_paste_sentinel_then_filtered() {
    let mut buf = "__EXISTING_KEY__".to_string();
    let mut cursor = 16;
    handle_text_paste(
        "+1-555-1234",
        &mut buf,
        &mut cursor,
        true,
        Some(|c: char| c.is_ascii_digit() || c == '+' || c == '-'),
    );
    assert_eq!(buf, "+1-555-1234");
    assert_eq!(cursor, 11);
}

// ── Channel input cursor integration tests ──

#[test]
fn test_telegram_sentinel_paste_replaces() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::TelegramSetup;
    wizard.telegram_field = TelegramField::BotToken;
    wizard.telegram_token_input = EXISTING_KEY_SENTINEL.to_string();
    wizard.channel_input_cursor = EXISTING_KEY_SENTINEL.len();

    // Paste new token — should replace sentinel, not append
    wizard.handle_paste("123456:ABC-DEF");
    assert_eq!(wizard.telegram_token_input, "123456:ABC-DEF");
    assert_eq!(wizard.channel_input_cursor, 14);
}

#[test]
fn test_telegram_sentinel_backspace_clears() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::TelegramSetup;
    wizard.telegram_field = TelegramField::BotToken;
    wizard.telegram_token_input = EXISTING_KEY_SENTINEL.to_string();
    wizard.channel_input_cursor = EXISTING_KEY_SENTINEL.len();

    wizard.handle_key(key(KeyCode::Backspace));
    assert_eq!(wizard.telegram_token_input, "");
    assert_eq!(wizard.channel_input_cursor, 0);
}

#[test]
fn test_telegram_sentinel_enter_preserves() {
    let mut wizard = clean_wizard();
    wizard.mode = WizardMode::Advanced;
    wizard.step = OnboardingStep::TelegramSetup;
    wizard.telegram_field = TelegramField::BotToken;
    wizard.telegram_token_input = EXISTING_KEY_SENTINEL.to_string();
    wizard.channel_input_cursor = EXISTING_KEY_SENTINEL.len();

    // Enter advances to next field, sentinel stays
    wizard.handle_key(key(KeyCode::Enter));
    assert_eq!(wizard.telegram_token_input, EXISTING_KEY_SENTINEL);
    assert_eq!(wizard.telegram_field, TelegramField::UserID);
}

#[test]
fn test_discord_cursor_movement_in_field() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::DiscordSetup;
    wizard.discord_field = DiscordField::ChannelID;
    wizard.discord_channel_id_input = "12345".to_string();
    wizard.channel_input_cursor = 5;

    // Move left twice
    wizard.handle_key(key(KeyCode::Left));
    wizard.handle_key(key(KeyCode::Left));
    assert_eq!(wizard.channel_input_cursor, 3);

    // Type a char at cursor position
    wizard.handle_key(key(KeyCode::Char('X')));
    assert_eq!(wizard.discord_channel_id_input, "123X45");
    assert_eq!(wizard.channel_input_cursor, 4);
}

// --- Provider display order & navigation with custom providers ---

#[test]
fn test_provider_display_order_no_customs() {
    let mut wizard = clean_wizard();
    wizard.ps.custom_names.clear();
    let order = wizard.ps.provider_display_order();
    // Named providers (0-9) sorted alphabetically, then 10 ("+ New Custom") last
    // Alphabetical: Anthropic(0), Claude CLI(7), GitHub Copilot(2), Google Gemini(3),
    //               Minimax(5), OpenAI(1), OpenCode CLI(8), OpenRouter(4), Qwen CLI(9), z.ai GLM(6)
    assert_eq!(order, vec![0, 7, 2, 3, 5, 1, 8, 4, 9, 6, 10]);
}

#[test]
fn test_provider_display_order_with_customs() {
    let mut wizard = clean_wizard();
    wizard.ps.custom_names = vec!["nvidia".into(), "opus".into(), "opusdistil".into()];
    let order = wizard.ps.provider_display_order();
    // Named providers sorted alphabetically, then 12,13,14 existing customs, 11 ("+ New Custom") last
    // Alphabetic: Anthropic(0), Claude CLI(7), GitHub(2), Gemini(3), Minimax(5),
    //             OpenAI(1), OpenCode CLI(8), OpenRouter(4), Qwen(10), Qwen CLI(9), z.ai GLM(6)
    assert_eq!(
        order,
        vec![0, 7, 2, 3, 5, 1, 8, 4, 10, 9, 6, 12, 13, 14, 11]
    );
}

#[test]
fn test_provider_nav_down_from_last_static_goes_to_first_custom() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::ProviderAuth;
    wizard.auth_field = AuthField::Provider;
    wizard.ps.custom_names = vec!["nvidia".into(), "opus".into()];
    wizard.ps.selected_provider = 6; // z.ai GLM (last named alphabetically)

    wizard.handle_key(key(KeyCode::Down));
    // Should go to nvidia (index 12 = CUSTOM_INSTANCES_START), not "+ New Custom" (index 11)
    assert_eq!(
        wizard.ps.selected_provider, 12,
        "Down from z.ai GLM should go to first custom provider, not +New Custom"
    );
}

#[test]
fn test_provider_nav_down_through_customs_to_new() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::ProviderAuth;
    wizard.auth_field = AuthField::Provider;
    wizard.ps.custom_names = vec!["nvidia".into()];
    wizard.ps.selected_provider = 12; // nvidia

    wizard.handle_key(key(KeyCode::Down));
    // Should go to "+ New Custom" (index 11) which is visually last
    assert_eq!(
        wizard.ps.selected_provider, 11,
        "Down from last custom should go to +New Custom"
    );
}

#[test]
fn test_provider_nav_up_from_new_custom_goes_to_last_custom() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::ProviderAuth;
    wizard.auth_field = AuthField::Provider;
    wizard.ps.custom_names = vec!["nvidia".into(), "opus".into()];
    wizard.ps.selected_provider = 11; // "+ New Custom"

    wizard.handle_key(key(KeyCode::Up));
    // Should go to opus (index 13), not z.ai GLM (index 6)
    assert_eq!(
        wizard.ps.selected_provider, 13,
        "Up from +New Custom should go to last custom provider"
    );
}

#[test]
fn test_provider_nav_up_from_first_custom_goes_to_last_static() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::ProviderAuth;
    wizard.auth_field = AuthField::Provider;
    wizard.ps.custom_names = vec!["nvidia".into(), "opus".into()];
    wizard.ps.selected_provider = 12; // nvidia (first custom)

    wizard.handle_key(key(KeyCode::Up));
    assert_eq!(
        wizard.ps.selected_provider, 6,
        "Up from first custom should go to z.ai GLM (last named alphabetically)"
    );
}

#[test]
fn test_provider_nav_clamps_at_top_and_bottom() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::ProviderAuth;
    wizard.auth_field = AuthField::Provider;
    wizard.ps.custom_names = vec!["nvidia".into()];

    // At top, Up stays at 0
    wizard.ps.selected_provider = 0;
    wizard.handle_key(key(KeyCode::Up));
    assert_eq!(wizard.ps.selected_provider, 0);

    // At bottom ("+ New Custom" = CUSTOM_PROVIDER_IDX = 11), Down stays
    wizard.ps.selected_provider = 11;
    wizard.handle_key(key(KeyCode::Down));
    assert_eq!(wizard.ps.selected_provider, 11);
}

#[test]
fn test_provider_nav_full_cycle_matches_display_order() {
    let mut wizard = clean_wizard();
    wizard.step = OnboardingStep::ProviderAuth;
    wizard.auth_field = AuthField::Provider;
    wizard.ps.custom_names = vec!["nvidia".into(), "opus".into()];
    wizard.ps.selected_provider = 0;

    let expected_order = wizard.ps.provider_display_order();
    let mut visited = vec![wizard.ps.selected_provider];

    for _ in 1..expected_order.len() {
        wizard.handle_key(key(KeyCode::Down));
        visited.push(wizard.ps.selected_provider);
    }

    assert_eq!(
        visited, expected_order,
        "Navigation order must match display order"
    );
}
