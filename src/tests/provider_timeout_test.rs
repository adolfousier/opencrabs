use std::sync::Arc;
use std::time::Duration;

use crate::brain::provider::custom_openai_compatible::OpenAIProvider;
use crate::brain::provider::fallback::FallbackProvider;
use crate::brain::provider::r#trait::Provider;
use crate::config::types::ProviderConfig;

#[test]
fn test_provider_config_timeout_deserialization() {
    let toml_str = r#"
        enabled = true
        timeout_secs = 45
        stream_idle_timeout_secs = 15
    "#;
    let config: ProviderConfig =
        toml::from_str(toml_str).expect("Failed to deserialize ProviderConfig");
    assert_eq!(config.timeout_secs, Some(45));
    assert_eq!(config.stream_idle_timeout_secs, Some(15));

    let empty_toml = "enabled = true";
    let empty_config: ProviderConfig =
        toml::from_str(empty_toml).expect("Failed to deserialize empty config");
    assert_eq!(empty_config.timeout_secs, None);
    assert_eq!(empty_config.stream_idle_timeout_secs, None);
}

#[test]
fn test_openai_provider_timeout_builder_and_getters() {
    let provider = OpenAIProvider::new("test-key".to_string())
        .with_timeout(Duration::from_secs(30))
        .with_stream_idle_timeout(Duration::from_secs(12));

    assert_eq!(provider.request_timeout(), Some(Duration::from_secs(30)));
    assert_eq!(
        provider.stream_idle_timeout(),
        Some(Duration::from_secs(12))
    );

    let default_provider = OpenAIProvider::new("test-key".to_string());
    assert_eq!(default_provider.request_timeout(), None);
    assert_eq!(default_provider.stream_idle_timeout(), None);
}

#[test]
fn test_fallback_provider_timeout_delegation() {
    let primary = Arc::new(
        OpenAIProvider::new("primary-key".to_string())
            .with_timeout(Duration::from_secs(50))
            .with_stream_idle_timeout(Duration::from_secs(18)),
    );
    let fallback = Arc::new(
        OpenAIProvider::new("fallback-key".to_string())
            .with_timeout(Duration::from_secs(25))
            .with_stream_idle_timeout(Duration::from_secs(10)),
    );

    let chain = FallbackProvider::new(primary, vec![fallback]);
    assert_eq!(chain.request_timeout(), Some(Duration::from_secs(50)));
    assert_eq!(chain.stream_idle_timeout(), Some(Duration::from_secs(18)));
}
