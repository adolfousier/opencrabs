//! Provider wiring sync tests.
//!
//! Ensures that every named provider field in `ProviderConfigs` is wired into
//! `active_provider_and_model` (config resolution) so that enabling a provider
//! in config.toml actually resolves it. This catches the class of bug where a
//! new provider is added to the struct but forgotten in the candidates list.

use crate::config::{Config, ProviderConfig, resolve_provider_from_config};

/// Helper: create a config with only the given provider enabled.
fn config_with_only(provider: &str) -> Config {
    let mut config = Config::default();
    let pc = Some(ProviderConfig {
        enabled: true,
        default_model: Some("test-model".to_string()),
        ..Default::default()
    });
    match provider {
        "anthropic" => config.providers.anthropic = pc,
        "openai" => config.providers.openai = pc,
        "github" => config.providers.github = pc,
        "gemini" => config.providers.gemini = pc,
        "openrouter" => config.providers.openrouter = pc,
        "minimax" => config.providers.minimax = pc,
        _ => panic!("Unknown provider: {}", provider),
    }
    config
}

// ── Each named provider must resolve when it's the only one enabled ──

#[test]
fn anthropic_resolves_when_only_enabled() {
    let config = config_with_only("anthropic");
    let (name, _) = resolve_provider_from_config(&config);
    assert_eq!(name, "Anthropic", "Anthropic must be wired in candidates");
}

#[test]
fn openai_resolves_when_only_enabled() {
    let config = config_with_only("openai");
    let (name, _) = resolve_provider_from_config(&config);
    assert_eq!(name, "OpenAI", "OpenAI must be wired in candidates");
}

#[test]
fn github_resolves_when_only_enabled() {
    let config = config_with_only("github");
    let (name, _) = resolve_provider_from_config(&config);
    assert_eq!(name, "GitHub Copilot", "GitHub must be wired in candidates");
}

#[test]
fn gemini_resolves_when_only_enabled() {
    let config = config_with_only("gemini");
    let (name, _) = resolve_provider_from_config(&config);
    assert_eq!(name, "Google Gemini", "Gemini must be wired in candidates");
}

#[test]
fn openrouter_resolves_when_only_enabled() {
    let config = config_with_only("openrouter");
    let (name, _) = resolve_provider_from_config(&config);
    assert_eq!(name, "OpenRouter", "OpenRouter must be wired in candidates");
}

#[test]
fn minimax_resolves_when_only_enabled() {
    let config = config_with_only("minimax");
    let (name, _) = resolve_provider_from_config(&config);
    assert_eq!(name, "Minimax", "Minimax must be wired in candidates");
}

// ── PROVIDERS onboarding array must cover all named providers ──

#[test]
fn onboarding_providers_covers_all_named_providers() {
    use crate::tui::onboarding::PROVIDERS;
    let provider_names: Vec<&str> = PROVIDERS.iter().map(|p| p.name).collect();

    // Every named config provider must have a corresponding onboarding entry.
    // Map from config field name → expected onboarding display name.
    let required = [
        ("anthropic", "Anthropic Claude"),
        ("openai", "OpenAI"),
        ("github", "GitHub Copilot"),
        ("gemini", "Google Gemini"),
        ("openrouter", "OpenRouter"),
        ("minimax", "Minimax"),
    ];

    for (config_name, display_name) in required {
        assert!(
            provider_names.contains(&display_name),
            "Provider '{}' (config: {}) missing from PROVIDERS onboarding array. \
             Got: {:?}",
            display_name,
            config_name,
            provider_names
        );
    }
}

// ── No provider with enabled=true should silently fall through ──

#[test]
fn all_named_providers_resolve_to_known_name() {
    let known_names = [
        "Anthropic",
        "OpenAI",
        "GitHub Copilot",
        "Google Gemini",
        "OpenRouter",
        "Minimax",
    ];

    for provider in [
        "anthropic",
        "openai",
        "github",
        "gemini",
        "openrouter",
        "minimax",
    ] {
        let config = config_with_only(provider);
        let (name, _) = resolve_provider_from_config(&config);
        assert!(
            known_names.contains(&name.as_ref()),
            "Provider '{}' resolved to unknown name '{}' — is it wired in resolve_provider_from_config?",
            provider,
            name
        );
    }
}
