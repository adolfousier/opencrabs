//! Context window configuration tests.
//!
//! Tests that the context_window field on ProviderConfig is properly wired
//! through to OpenAIProvider, overriding model-name heuristics for custom/local
//! providers whose models aren't recognized by name.

use crate::brain::Provider;
use crate::brain::provider::custom_openai_compatible::OpenAIProvider;
use crate::brain::provider::factory::{create_provider, create_provider_by_name};
use crate::config::{Config, ProviderConfig, ProviderConfigs};
use std::collections::BTreeMap;

// ── Helper ──────────────────────────────────────────────────────

fn config_with_context_window(context_window: Option<u32>) -> Config {
    let mut custom_map = BTreeMap::new();
    custom_map.insert(
        "lmstudio".to_string(),
        ProviderConfig {
            enabled: true,
            api_key: None,
            base_url: Some("http://localhost:1234/v1".to_string()),
            default_model: Some("my-local-model".to_string()),
            context_window,
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

// ── OpenAIProvider: with_context_window builder ─────────────────

#[test]
fn with_context_window_overrides_unknown_model() {
    let provider = OpenAIProvider::with_base_url(
        String::new(),
        "http://localhost:1234/v1/chat/completions".to_string(),
    )
    .with_name("local")
    .with_context_window(32_000);

    // Unknown model would normally return None, but configured value takes priority
    assert_eq!(provider.context_window("my-custom-model"), Some(32_000));
}

#[test]
fn with_context_window_overrides_known_model() {
    let provider = OpenAIProvider::with_base_url(
        String::new(),
        "http://localhost:1234/v1/chat/completions".to_string(),
    )
    .with_context_window(64_000);

    // Even known models (gpt-4o = 128k) are overridden by explicit config
    assert_eq!(provider.context_window("gpt-4o"), Some(64_000));
}

#[test]
fn without_context_window_returns_none_for_unknown() {
    let provider = OpenAIProvider::with_base_url(
        String::new(),
        "http://localhost:1234/v1/chat/completions".to_string(),
    );

    // No configured value, unknown model → None
    assert_eq!(provider.context_window("my-custom-model"), None);
}

#[test]
fn without_context_window_returns_heuristic_for_known() {
    let provider = OpenAIProvider::with_base_url(
        String::new(),
        "http://localhost:1234/v1/chat/completions".to_string(),
    );

    // No configured value, known model → heuristic
    assert_eq!(provider.context_window("gpt-4o"), Some(128_000));
}

// ── Factory: context_window wired through config ────────────────

#[test]
fn factory_passes_context_window_to_provider() {
    let config = config_with_context_window(Some(200_000));
    let result = create_provider(&config);
    assert!(result.is_ok());
    let provider = result.unwrap();
    assert_eq!(
        provider.context_window("my-local-model"),
        Some(200_000),
        "Factory should pass context_window from config to provider"
    );
}

#[test]
fn factory_no_context_window_returns_none_for_unknown() {
    let config = config_with_context_window(None);
    let result = create_provider(&config);
    assert!(result.is_ok());
    let provider = result.unwrap();
    assert_eq!(
        provider.context_window("my-local-model"),
        None,
        "Without context_window in config, unknown model should return None"
    );
}

#[test]
fn factory_by_name_passes_context_window() {
    let config = config_with_context_window(Some(16_384));
    let result = create_provider_by_name(&config, "custom:lmstudio");
    assert!(result.is_ok());
    let provider = result.unwrap();
    assert_eq!(provider.context_window("whatever-model"), Some(16_384));
}

// ── ProviderConfig serialization ────────────────────────────────

#[test]
fn context_window_serializes_when_set() {
    let cfg = ProviderConfig {
        enabled: true,
        context_window: Some(128_000),
        ..Default::default()
    };
    let toml_str = toml::to_string(&cfg).expect("serialize");
    assert!(
        toml_str.contains("context_window = 128000"),
        "context_window should appear in serialized TOML: {}",
        toml_str
    );
}

#[test]
fn context_window_omitted_when_none() {
    let cfg = ProviderConfig {
        enabled: true,
        context_window: None,
        ..Default::default()
    };
    let toml_str = toml::to_string(&cfg).expect("serialize");
    assert!(
        !toml_str.contains("context_window"),
        "context_window should be omitted when None: {}",
        toml_str
    );
}

#[test]
fn context_window_deserializes_from_toml() {
    let toml_str = r#"
enabled = true
context_window = 32000
"#;
    let cfg: ProviderConfig = toml::from_str(toml_str).expect("deserialize");
    assert_eq!(cfg.context_window, Some(32_000));
}

#[test]
fn context_window_defaults_to_none_when_missing() {
    let toml_str = r#"
enabled = true
"#;
    let cfg: ProviderConfig = toml::from_str(toml_str).expect("deserialize");
    assert_eq!(cfg.context_window, None);
}

// ── Multiple custom providers with different context windows ────

#[test]
fn multiple_customs_each_get_own_context_window() {
    let mut custom_map = BTreeMap::new();
    custom_map.insert(
        "nvidia".to_string(),
        ProviderConfig {
            enabled: true,
            base_url: Some("https://integrate.api.nvidia.com/v1".to_string()),
            default_model: Some("llama-70b".to_string()),
            context_window: Some(128_000),
            ..Default::default()
        },
    );
    custom_map.insert(
        "ollama".to_string(),
        ProviderConfig {
            enabled: false,
            base_url: Some("http://localhost:11434/v1".to_string()),
            default_model: Some("phi3".to_string()),
            context_window: Some(4_096),
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

    // nvidia is the active (enabled) one
    let provider = create_provider(&config).unwrap();
    assert_eq!(provider.context_window("llama-70b"), Some(128_000));

    // ollama via by_name
    let ollama = create_provider_by_name(&config, "custom:ollama").unwrap();
    assert_eq!(ollama.context_window("phi3"), Some(4_096));
}

#[test]
fn context_window_zero_is_valid() {
    // Edge case: user sets context_window = 0 (unusual but shouldn't crash)
    let provider = OpenAIProvider::with_base_url(
        String::new(),
        "http://localhost:1234/v1/chat/completions".to_string(),
    )
    .with_context_window(0);
    assert_eq!(provider.context_window("any-model"), Some(0));
}

#[test]
fn context_window_large_value() {
    // Very large context windows (e.g. 1M+ tokens)
    let provider = OpenAIProvider::with_base_url(
        String::new(),
        "http://localhost:1234/v1/chat/completions".to_string(),
    )
    .with_context_window(2_000_000);
    assert_eq!(provider.context_window("any-model"), Some(2_000_000));
}
