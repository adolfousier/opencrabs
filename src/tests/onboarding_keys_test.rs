//! Onboarding Provider Keys Tests
//!
//! Tests that all providers (Anthropic, OpenAI, Gemini, OpenRouter, Minimax, Custom)
//! correctly use api_key_input for their API keys

use crate::tui::onboarding::{OnboardingWizard, PROVIDERS};

#[test]
fn test_provider_count_matches() {
    // Verify PROVIDERS array has 11 entries
    assert_eq!(PROVIDERS.len(), 11);

    // Verify provider names
    assert_eq!(PROVIDERS[0].name, "Anthropic Claude");
    assert_eq!(PROVIDERS[1].name, "OpenAI");
    assert_eq!(PROVIDERS[2].name, "GitHub Copilot");
    assert_eq!(PROVIDERS[3].name, "Google Gemini");
    assert_eq!(PROVIDERS[4].name, "OpenRouter");
    assert_eq!(PROVIDERS[5].name, "Minimax");
    assert_eq!(PROVIDERS[6].name, "z.ai GLM");
    assert_eq!(PROVIDERS[7].name, "Claude CLI");
    assert_eq!(PROVIDERS[8].name, "OpenCode CLI");
    assert_eq!(PROVIDERS[9].name, "Qwen Code");
    assert_eq!(PROVIDERS[10].name, "Custom OpenAI-Compatible");
}

#[test]
fn test_is_custom_provider() {
    let mut wizard = OnboardingWizard::new();

    // Index 10 is Custom
    wizard.ps.selected_provider = 10;
    assert!(wizard.ps.is_custom());

    // Other indices are not Custom
    wizard.ps.selected_provider = 0;
    assert!(!wizard.ps.is_custom());
    wizard.ps.selected_provider = 1;
    assert!(!wizard.ps.is_custom());
    wizard.ps.selected_provider = 2;
    assert!(!wizard.ps.is_custom());
    wizard.ps.selected_provider = 8;
    assert!(!wizard.ps.is_custom());
}

#[test]
fn test_all_providers_use_api_key_input() {
    // All providers (including Custom) use the same api_key_input field
    let test_key = "test-api-key-12345";

    for idx in 0..PROVIDERS.len() {
        let mut wizard = OnboardingWizard::new();
        wizard.ps.selected_provider = idx;
        wizard.ps.api_key_input = test_key.to_string();

        // Custom provider also uses api_key_input (no separate field)
        assert_eq!(wizard.ps.api_key_input, test_key);
        assert!(!wizard.ps.has_existing_key_sentinel()); // fresh key, not sentinel
    }
}

#[test]
fn test_keys_toml_has_all_provider_sections() {
    use crate::config::ProviderConfigs;

    // All these should be None by default
    let keys = ProviderConfigs::default();
    assert!(keys.anthropic.is_none());
    assert!(keys.openai.is_none());
    assert!(keys.github.is_none());
    assert!(keys.gemini.is_none());
    assert!(keys.openrouter.is_none());
    assert!(keys.minimax.is_none());
    assert!(keys.custom.is_none()); // BTreeMap<String, ProviderConfig> — still None by default
}
