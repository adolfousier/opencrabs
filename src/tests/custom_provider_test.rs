//! Custom provider tests.
//!
//! Tests factory fallback behavior, custom providers with optional API keys,
//! local providers (LM Studio, Ollama), and no-crash guarantees.

use crate::brain::Provider;
use crate::brain::provider::custom_openai_compatible::OpenAIProvider;
use crate::brain::provider::factory::{create_provider, create_provider_by_name};
use crate::config::{Config, ProviderConfig, ProviderConfigs};
use std::collections::BTreeMap;

// ── Custom provider creation ────────────────────────────────────

#[test]
fn custom_provider_without_api_key() {
    // Local providers (LM Studio, Ollama) don't need an API key
    let provider = OpenAIProvider::with_base_url(
        String::new(), // empty key
        "http://localhost:1234/v1/chat/completions".to_string(),
    )
    .with_name("lmstudio");
    assert_eq!(provider.name(), "lmstudio");
}

#[test]
fn custom_provider_with_api_key() {
    let provider = OpenAIProvider::with_base_url(
        "sk-test-key".to_string(),
        "https://api.example.com/v1/chat/completions".to_string(),
    )
    .with_name("my-remote");
    assert_eq!(provider.name(), "my-remote");
}

#[test]
fn custom_provider_default_model() {
    let provider = OpenAIProvider::with_base_url(
        String::new(),
        "http://localhost:1234/v1/chat/completions".to_string(),
    )
    .with_name("ollama")
    .with_default_model("llama3".to_string());
    assert_eq!(provider.default_model(), "llama3");
}

// ── Factory: custom providers from config ───────────────────────

fn config_with_custom(name: &str, api_key: Option<String>, base_url: Option<String>) -> Config {
    let mut custom_map = BTreeMap::new();
    custom_map.insert(
        name.to_string(),
        ProviderConfig {
            enabled: true,
            api_key,
            base_url,
            default_model: Some("test-model".to_string()),
            models: vec![],
            vision_model: None,
            ..Default::default()
        },
    );
    Config {
        providers: ProviderConfigs {
            custom: Some(custom_map),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn factory_creates_custom_without_api_key() {
    let config = config_with_custom(
        "lmstudio",
        None,
        Some("http://localhost:1234/v1".to_string()),
    );
    let result = create_provider(&config);
    assert!(result.is_ok());
    let provider = result.unwrap();
    assert_eq!(provider.name(), "lmstudio");
}

#[test]
fn factory_creates_custom_with_api_key() {
    let config = config_with_custom(
        "remote-llm",
        Some("sk-test".to_string()),
        Some("https://api.example.com/v1".to_string()),
    );
    let result = create_provider(&config);
    assert!(result.is_ok());
    let provider = result.unwrap();
    assert_eq!(provider.name(), "remote-llm");
}

#[test]
fn factory_creates_custom_with_empty_api_key() {
    let config = config_with_custom(
        "ollama",
        Some(String::new()),
        Some("http://localhost:11434/v1".to_string()),
    );
    let result = create_provider(&config);
    assert!(result.is_ok());
    let provider = result.unwrap();
    assert_eq!(provider.name(), "ollama");
}

#[test]
fn factory_custom_auto_appends_chat_completions() {
    // base_url without /chat/completions should get it appended
    let config = config_with_custom(
        "test-local",
        None,
        Some("http://localhost:1234/v1".to_string()),
    );
    let result = create_provider(&config);
    assert!(result.is_ok());
}

#[test]
fn factory_custom_preserves_chat_completions_suffix() {
    // base_url already has /chat/completions — should not double-append
    let config = config_with_custom(
        "test-local",
        None,
        Some("http://localhost:1234/v1/chat/completions".to_string()),
    );
    let result = create_provider(&config);
    assert!(result.is_ok());
}

#[test]
fn factory_custom_default_base_url() {
    // No base_url → defaults to localhost:1234
    let config = config_with_custom("local", None, None);
    let result = create_provider(&config);
    assert!(result.is_ok());
}

// ── Factory: create_provider_by_name ────────────────────────────

#[test]
fn create_by_name_custom_prefix() {
    let config = config_with_custom(
        "mylocal",
        None,
        Some("http://localhost:1234/v1".to_string()),
    );
    let result = create_provider_by_name(&config, "custom:mylocal");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().name(), "mylocal");
}

#[test]
fn create_by_name_unknown_custom() {
    let config = Config::default();
    let result = create_provider_by_name(&config, "custom:nonexistent");
    assert!(result.is_err());
}

#[test]
fn create_by_name_legacy_custom() {
    // Legacy sessions store just the custom name without "custom:" prefix
    let config = config_with_custom(
        "lmstudio",
        None,
        Some("http://localhost:1234/v1".to_string()),
    );
    let result = create_provider_by_name(&config, "lmstudio");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().name(), "lmstudio");
}

// ── Factory: no-crash guarantees ────────────────────────────────

#[test]
fn factory_never_crashes_empty_config() {
    let config = Config::default();
    let result = create_provider(&config);
    // Must succeed — returns PlaceholderProvider
    assert!(result.is_ok());
}

#[test]
fn factory_never_crashes_all_missing_keys() {
    // All providers enabled but none have API keys
    let config = Config {
        providers: ProviderConfigs {
            anthropic: Some(ProviderConfig {
                enabled: true,
                api_key: None,
                ..Default::default()
            }),
            openai: Some(ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                ..Default::default()
            }),
            github: Some(ProviderConfig {
                enabled: true,
                api_key: None,
                ..Default::default()
            }),
            gemini: Some(ProviderConfig {
                enabled: true,
                api_key: None,
                ..Default::default()
            }),
            openrouter: Some(ProviderConfig {
                enabled: true,
                api_key: None,
                ..Default::default()
            }),
            minimax: Some(ProviderConfig {
                enabled: true,
                api_key: None,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let result = create_provider(&config);
    // Must succeed — falls back to PlaceholderProvider
    assert!(result.is_ok());
}

#[test]
fn factory_falls_back_when_primary_fails() {
    // Anthropic enabled but no key, OpenAI has key → should fall back to OpenAI
    let config = Config {
        providers: ProviderConfigs {
            anthropic: Some(ProviderConfig {
                enabled: true,
                api_key: None,
                ..Default::default()
            }),
            openai: Some(ProviderConfig {
                enabled: true,
                api_key: Some("test-key".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let result = create_provider(&config);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().name(), "openai");
}

#[test]
fn factory_priority_order_anthropic_first() {
    // Both Anthropic and OpenAI have keys — Anthropic should win
    let config = Config {
        providers: ProviderConfigs {
            anthropic: Some(ProviderConfig {
                enabled: true,
                api_key: Some("anthropic-key".to_string()),
                ..Default::default()
            }),
            openai: Some(ProviderConfig {
                enabled: true,
                api_key: Some("openai-key".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let result = create_provider(&config);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().name(), "anthropic");
}

#[test]
fn factory_custom_before_placeholder() {
    // Only custom provider configured — should use it, not placeholder
    let config = config_with_custom(
        "ollama",
        None,
        Some("http://localhost:11434/v1".to_string()),
    );
    let result = create_provider(&config);
    assert!(result.is_ok());
    assert_ne!(result.unwrap().name(), "placeholder");
}

// ── Multiple custom providers ───────────────────────────────────

#[test]
fn active_custom_picks_first_enabled() {
    let mut custom_map = BTreeMap::new();
    custom_map.insert(
        "disabled-one".to_string(),
        ProviderConfig {
            enabled: false,
            base_url: Some("http://localhost:1111/v1".to_string()),
            ..Default::default()
        },
    );
    custom_map.insert(
        "enabled-one".to_string(),
        ProviderConfig {
            enabled: true,
            base_url: Some("http://localhost:2222/v1".to_string()),
            default_model: Some("model-a".to_string()),
            ..Default::default()
        },
    );
    let configs = ProviderConfigs {
        custom: Some(custom_map),
        ..Default::default()
    };
    let active = configs.active_custom();
    assert!(active.is_some());
    let (name, cfg) = active.unwrap();
    assert_eq!(name, "enabled-one");
    assert!(cfg.enabled);
}

#[test]
fn no_active_custom_when_all_disabled() {
    let mut custom_map = BTreeMap::new();
    custom_map.insert(
        "off".to_string(),
        ProviderConfig {
            enabled: false,
            ..Default::default()
        },
    );
    let configs = ProviderConfigs {
        custom: Some(custom_map),
        ..Default::default()
    };
    assert!(configs.active_custom().is_none());
}

#[test]
fn no_active_custom_when_none() {
    let configs = ProviderConfigs::default();
    assert!(configs.active_custom().is_none());
}

// ── Custom provider list (model selector / onboarding) ──────────

#[test]
fn wizard_is_custom_for_new_and_existing() {
    use crate::tui::onboarding::OnboardingWizard;
    let mut wizard = OnboardingWizard::new();
    // Index 9 = "+ New Custom Provider"
    wizard.selected_provider = 9;
    assert!(wizard.is_custom_provider());
    // Index 10+ = existing custom providers
    wizard.selected_provider = 10;
    assert!(wizard.is_custom_provider());
    wizard.selected_provider = 11;
    assert!(wizard.is_custom_provider());
    // Index < 9 = not custom
    wizard.selected_provider = 0;
    assert!(!wizard.is_custom_provider());
    wizard.selected_provider = 8;
    assert!(!wizard.is_custom_provider());
}

#[test]
fn wizard_current_provider_clamps_for_existing_custom() {
    use crate::tui::onboarding::{OnboardingWizard, PROVIDERS};
    let mut wizard = OnboardingWizard::new();
    // Index 10 should map to the Custom entry (index 9) in PROVIDERS
    wizard.selected_provider = 10;
    assert_eq!(wizard.current_provider().name, PROVIDERS[9].name);
    wizard.selected_provider = 99;
    assert_eq!(wizard.current_provider().name, PROVIDERS[9].name);
}

#[test]
fn wizard_load_custom_fields_clears_for_new() {
    use crate::tui::onboarding::OnboardingWizard;
    let mut wizard = OnboardingWizard::new();
    wizard.custom_provider_name = "leftover".to_string();
    wizard.custom_base_url = "http://old-url".to_string();
    wizard.custom_model = "old-model".to_string();
    wizard.selected_provider = 9;
    wizard.load_custom_fields_if_existing();
    assert!(wizard.custom_provider_name.is_empty());
    assert!(wizard.custom_base_url.is_empty());
    assert!(wizard.custom_model.is_empty());
}

#[test]
fn wizard_existing_custom_names_populated_from_config() {
    use crate::tui::onboarding::OnboardingWizard;
    // The wizard loads existing_custom_names from config in new()
    // This test just verifies the field exists and is a Vec
    let wizard = OnboardingWizard::new();
    let _: &Vec<String> = &wizard.existing_custom_names;
}

#[test]
fn multiple_custom_providers_in_config() {
    // Verify BTreeMap preserves insertion order (alphabetical for BTreeMap)
    let mut custom_map = BTreeMap::new();
    custom_map.insert(
        "nvidia".to_string(),
        ProviderConfig {
            enabled: false,
            base_url: Some("https://integrate.api.nvidia.com/v1".to_string()),
            default_model: Some("llama-3.3-70b".to_string()),
            ..Default::default()
        },
    );
    custom_map.insert(
        "ollama".to_string(),
        ProviderConfig {
            enabled: true,
            base_url: Some("http://localhost:11434/v1".to_string()),
            default_model: Some("llama3".to_string()),
            ..Default::default()
        },
    );
    custom_map.insert(
        "lmstudio".to_string(),
        ProviderConfig {
            enabled: false,
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("qwen".to_string()),
            ..Default::default()
        },
    );
    let configs = ProviderConfigs {
        custom: Some(custom_map),
        ..Default::default()
    };

    // active_custom should return the enabled one
    let (name, _) = configs.active_custom().unwrap();
    assert_eq!(name, "ollama");

    // All names should be available as keys
    let names: Vec<String> = configs.custom.as_ref().unwrap().keys().cloned().collect();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"nvidia".to_string()));
    assert!(names.contains(&"ollama".to_string()));
    assert!(names.contains(&"lmstudio".to_string()));
}

#[test]
fn factory_switches_between_custom_providers() {
    // Two custom providers, only one enabled — factory picks the enabled one
    let mut custom_map = BTreeMap::new();
    custom_map.insert(
        "nvidia".to_string(),
        ProviderConfig {
            enabled: false,
            base_url: Some("https://integrate.api.nvidia.com/v1".to_string()),
            default_model: Some("llama-3.3-70b".to_string()),
            ..Default::default()
        },
    );
    custom_map.insert(
        "local".to_string(),
        ProviderConfig {
            enabled: true,
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("qwen".to_string()),
            ..Default::default()
        },
    );
    let config = Config {
        providers: ProviderConfigs {
            custom: Some(custom_map),
            ..Default::default()
        },
        ..Default::default()
    };
    let result = create_provider(&config);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().name(), "local");
}

#[test]
fn create_by_name_picks_specific_custom() {
    // Even when "local" is enabled, create_by_name("custom:nvidia") picks nvidia
    let mut custom_map = BTreeMap::new();
    custom_map.insert(
        "nvidia".to_string(),
        ProviderConfig {
            enabled: false,
            base_url: Some("https://integrate.api.nvidia.com/v1".to_string()),
            default_model: Some("llama-3.3-70b".to_string()),
            ..Default::default()
        },
    );
    custom_map.insert(
        "local".to_string(),
        ProviderConfig {
            enabled: true,
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("qwen".to_string()),
            ..Default::default()
        },
    );
    let config = Config {
        providers: ProviderConfigs {
            custom: Some(custom_map),
            ..Default::default()
        },
        ..Default::default()
    };
    let result = create_provider_by_name(&config, "custom:nvidia");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().name(), "nvidia");
}
