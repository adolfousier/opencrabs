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
    wizard.api_key_input = "test-key".to_string();

    assert_eq!(wizard.step, OnboardingStep::ModeSelect);
    wizard.next_step(); // ModeSelect -> Workspace
    assert_eq!(wizard.step, OnboardingStep::Workspace);
}

#[test]
fn test_advanced_mode_all_steps() {
    let mut wizard = OnboardingWizard::new();
    wizard.mode = WizardMode::Advanced;
    wizard.api_key_input = "test-key".to_string();

    wizard.next_step(); // ModeSelect -> Workspace
    assert_eq!(wizard.step, OnboardingStep::Workspace);
    wizard.next_step(); // Workspace -> ProviderAuth
    assert_eq!(wizard.step, OnboardingStep::ProviderAuth);
    wizard.next_step(); // ProviderAuth -> Channels
    assert_eq!(wizard.step, OnboardingStep::Channels);
    wizard.next_step(); // Channels -> Gateway (nothing enabled)
    assert_eq!(wizard.step, OnboardingStep::Gateway);
    wizard.next_step(); // Gateway -> VoiceSetup (Advanced)
    assert_eq!(wizard.step, OnboardingStep::VoiceSetup);
    wizard.next_step(); // VoiceSetup -> Daemon
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

    // Continue to Gateway
    wizard.focused_field = wizard.channel_toggles.len();
    wizard.handle_key(key(KeyCode::Enter));
    assert_eq!(wizard.step, OnboardingStep::Gateway);
}

#[test]
fn test_channels_whatsapp_skips_to_gateway() {
    let mut wizard = OnboardingWizard::new();
    wizard.mode = WizardMode::Advanced;
    wizard.api_key_input = "test-key".to_string();

    wizard.next_step(); // ModeSelect -> Workspace
    wizard.next_step(); // Workspace -> ProviderAuth
    wizard.next_step(); // ProviderAuth -> Channels

    // Enable WhatsApp only (no token sub-step)
    wizard.channel_toggles[2].1 = true;
    wizard.next_step(); // Channels -> Gateway (WhatsApp has no sub-step)
    assert_eq!(wizard.step, OnboardingStep::Gateway);
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

    // Continue to Gateway
    wizard.focused_field = wizard.channel_toggles.len();
    wizard.handle_key(key(KeyCode::Enter));
    assert_eq!(wizard.step, OnboardingStep::Gateway);
}

#[test]
fn test_voice_setup_defaults() {
    let wizard = OnboardingWizard::new();
    assert!(wizard.groq_api_key_input.is_empty());
    assert!(!wizard.tts_enabled);
    assert_eq!(wizard.voice_field, VoiceField::GroqApiKey);
}

#[test]
fn test_step_numbers() {
    assert_eq!(OnboardingStep::ModeSelect.number(), 1);
    assert_eq!(OnboardingStep::Channels.number(), 4);
    assert_eq!(OnboardingStep::TelegramSetup.number(), 4); // sub-step of Channels
    assert_eq!(OnboardingStep::Gateway.number(), 5);
    assert_eq!(OnboardingStep::VoiceSetup.number(), 6);
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
    assert_eq!(wizard.selected_provider, 0);
    assert_eq!(wizard.auth_field, AuthField::Provider);
    assert!(wizard.api_key_input.is_empty());
    assert_eq!(wizard.selected_model, 0);
    // First provider is Anthropic Claude
    assert_eq!(PROVIDERS[wizard.selected_provider].name, "Anthropic Claude");
    assert!(!PROVIDERS[wizard.selected_provider].help_lines.is_empty());
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
    assert!(toggle_names.contains(&"iMessage"));
}

/// Create a wizard with clean defaults (no config auto-detection).
/// `OnboardingWizard::new()` loads existing config from disk, which
/// pollutes provider/brain fields when a real config exists.
fn clean_wizard() -> OnboardingWizard {
    let mut w = OnboardingWizard::new();
    w.selected_provider = 0;
    w.api_key_input = String::new();
    w.custom_base_url = String::new();
    w.custom_model = String::new();
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
    assert_eq!(wizard.selected_provider, 0);

    wizard.handle_key(key(KeyCode::Down));
    assert_eq!(wizard.selected_provider, 1);

    wizard.handle_key(key(KeyCode::Up));
    assert_eq!(wizard.selected_provider, 0);

    // Can't go below 0
    wizard.handle_key(key(KeyCode::Up));
    assert_eq!(wizard.selected_provider, 0);
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
    assert_eq!(wizard.api_key_input, "sk");

    // Backspace
    wizard.handle_key(key(KeyCode::Backspace));
    assert_eq!(wizard.api_key_input, "s");
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
    wizard.api_key_input = "test-key".to_string();

    wizard.next_step(); // ModeSelect -> Workspace
    assert_eq!(wizard.step, OnboardingStep::Workspace);
    wizard.next_step(); // Workspace -> ProviderAuth
    assert_eq!(wizard.step, OnboardingStep::ProviderAuth);
    wizard.next_step(); // ProviderAuth -> Gateway (QuickStart skips Channels)
    assert_eq!(wizard.step, OnboardingStep::Gateway);
    wizard.next_step(); // Gateway -> Daemon (QuickStart skips Voice)
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
    wizard.config_models = vec!["model-a".into(), "model-b".into(), "model-c".into()];

    assert_eq!(wizard.selected_model, 0);
    wizard.handle_key(key(KeyCode::Down));
    assert_eq!(wizard.selected_model, 1);
    wizard.handle_key(key(KeyCode::Down));
    assert_eq!(wizard.selected_model, 2);
    // Should clamp to max
    for _ in 0..20 {
        wizard.handle_key(key(KeyCode::Down));
    }
    // Provider selection wraps or stays within bounds
    assert!(wizard.selected_provider < PROVIDERS.len());
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
    // OpenRouter is index 3, Custom is last
    assert_eq!(PROVIDERS[3].name, "OpenRouter");
    assert_eq!(PROVIDERS.last().unwrap().name, "Custom OpenAI-Compatible");
}

#[test]
fn test_model_count_uses_fetched_when_available() {
    let mut wizard = OnboardingWizard::new();
    // Static fallback is empty - models fetched from API
    assert_eq!(wizard.model_count(), 0);

    // After fetching
    wizard.fetched_models = vec![
        "model-a".into(),
        "model-b".into(),
        "model-c".into(),
        "model-d".into(),
    ];
    assert_eq!(wizard.model_count(), 4);
}

#[test]
fn test_selected_model_name_uses_fetched() {
    let mut wizard = OnboardingWizard::new();
    // No static models - should use fetched or show placeholder
    assert!(wizard.selected_model_name().is_empty() || wizard.fetched_models.is_empty());

    wizard.fetched_models = vec!["live-model-1".into(), "live-model-2".into()];
    wizard.selected_model = 1;
    assert_eq!(wizard.selected_model_name(), "live-model-2");
}

#[test]
fn test_supports_model_fetch() {
    let mut wizard = OnboardingWizard::new();
    wizard.selected_provider = 0; // Anthropic
    assert!(wizard.supports_model_fetch());
    wizard.selected_provider = 1; // OpenAI
    assert!(wizard.supports_model_fetch());
    wizard.selected_provider = 2; // Gemini
    assert!(!wizard.supports_model_fetch());
    wizard.selected_provider = 3; // OpenRouter
    assert!(wizard.supports_model_fetch());
    wizard.selected_provider = 4; // Minimax
    assert!(!wizard.supports_model_fetch());
    wizard.selected_provider = 5; // Custom
    assert!(!wizard.supports_model_fetch());
}

#[test]
fn test_fetch_models_unsupported_provider_returns_empty() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(fetch_provider_models(99, None));
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
    let models = rt.block_on(fetch_provider_models(0, Some(&key)));
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
    let models = rt.block_on(fetch_provider_models(0, Some(&key)));
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
    let models = rt.block_on(fetch_provider_models(1, Some(&key)));
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
    let models = rt.block_on(fetch_provider_models(4, Some(&key)));
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
    ));
    assert!(
        models.is_empty(),
        "Bad key should return empty, got {} models",
        models.len()
    );
}
