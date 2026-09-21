//! Regression test for RSI fallback provider wiring.
//!
//! Context (2026-06-04): The RSI autonomous loop called
//! `create_provider_by_name` directly, bypassing the `[providers.fallback]`
//! chain that `create_provider_with_warning` applied to normal sessions.
//! When the RSI provider rate-limited, the cycle died instead of cascading
//! to the configured fallback. The fix extracts the wrap logic into
//! `wrap_with_fallback_chain` and calls it from both paths.

use crate::brain::provider::Provider;
use crate::brain::provider::factory::wrap_with_fallback_chain;
use crate::config::{Config, FallbackProviderConfig};
use std::sync::Arc;

#[tokio::test]
async fn rsi_wrap_returns_raw_when_no_fallback_configured() {
    let mut config = Config::default();
    config.providers.fallback = None;

    let primary: Arc<dyn Provider> = Arc::new(crate::tests::agent_service_mocks::MockProvider);
    let result = wrap_with_fallback_chain(&config, primary.clone())
        .await
        .unwrap();

    assert!(
        !result.is_fallback_chain(),
        "with no fallback configured, wrap_with_fallback_chain must return the raw primary"
    );
}

#[tokio::test]
async fn rsi_wrap_returns_raw_when_fallback_disabled() {
    let mut config = Config::default();
    config.providers.fallback = Some(FallbackProviderConfig {
        generation: vec![],
        enabled: false,
        providers: vec!["minimax".to_string()],
        provider: None,
        vision: vec![],
    });

    let primary: Arc<dyn Provider> = Arc::new(crate::tests::agent_service_mocks::MockProvider);
    let result = wrap_with_fallback_chain(&config, primary.clone())
        .await
        .unwrap();

    assert!(
        !result.is_fallback_chain(),
        "with fallback disabled, wrap_with_fallback_chain must return the raw primary"
    );
}

/// Positive case: a valid custom fallback IS configured, so the wrap
/// helper must actually wrap. Without this test, the existing two only
/// pin the no-op paths and a regression that turned `wrap_with_fallback_chain`
/// into a no-op would still pass.
#[tokio::test]
async fn rsi_wrap_actually_wraps_when_valid_fallback_configured() {
    use crate::config::ProviderConfig;
    use std::collections::BTreeMap;

    let mut config = Config::default();
    // Configure a custom provider that constructs successfully (no
    // network call happens at construction time — `try_create_custom_by_name`
    // just builds the Arc).
    let mut customs = BTreeMap::new();
    customs.insert(
        "stub-fallback".to_string(),
        ProviderConfig {
            enabled: true,
            api_key: Some("test-key".to_string()),
            base_url: Some("http://127.0.0.1:1/v1".to_string()),
            default_model: Some("stub-model".to_string()),
            ..Default::default()
        },
    );
    config.providers.custom = Some(customs);
    config.providers.fallback = Some(FallbackProviderConfig {
        generation: vec![],
        enabled: true,
        providers: vec!["stub-fallback".to_string()],
        provider: None,
        vision: vec![],
    });

    let primary: Arc<dyn Provider> = Arc::new(crate::tests::agent_service_mocks::MockProvider);
    let result = wrap_with_fallback_chain(&config, primary.clone())
        .await
        .unwrap();

    assert!(
        result.is_fallback_chain(),
        "with a valid fallback configured, wrap_with_fallback_chain must return a FallbackProvider — \
         a no-op return would silently break the RSI loop's rescue path that this PR exists to fix"
    );
}

/// Self-name collision skip: if the fallback chain contains the
/// primary's own name, that entry must be filtered out. Without the
/// filter, a primary 429 would cascade straight to itself, defeating
/// the purpose of fallback. Pin the filter with a sole-entry chain so
/// the post-filter list is empty and the wrap returns the raw primary.
#[tokio::test]
async fn rsi_wrap_skips_fallback_with_same_name_as_primary() {
    let mut config = Config::default();
    config.providers.fallback = Some(FallbackProviderConfig {
        generation: vec![],
        enabled: true,
        // MockProvider.name() returns "mock" — same as this fallback id.
        providers: vec!["mock".to_string()],
        provider: None,
        vision: vec![],
    });

    let primary: Arc<dyn Provider> = Arc::new(crate::tests::agent_service_mocks::MockProvider);
    let result = wrap_with_fallback_chain(&config, primary.clone())
        .await
        .unwrap();

    assert!(
        !result.is_fallback_chain(),
        "fallback with the same name as primary must be filtered out — \
         leaving an empty chain that produces no wrap. Without this, a primary 429 \
         would cascade right back to itself, hitting the same dead endpoint."
    );
}
