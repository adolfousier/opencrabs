//! Regression guard for #2021: every OpenAI-compatible remote provider gets the
//! same compiled stream idle default (120s), not a host-specific one. A queued
//! request that sends no bytes for a while must not be killed by our own timer
//! at 20s (generic remote) or 45s (z.ai only). Local base URLs keep their own
//! runtime default, and the config tiers still override the compiled value.

use crate::brain::provider::factory::{
    COMPAT_STREAM_IDLE_SECS, compat_idle_default, create_provider,
};
use crate::config::{AgentConfig, Config, ProviderConfig, ProviderConfigs};
use std::time::Duration;

fn zai(base_url: Option<&str>, idle: Option<u64>) -> ProviderConfig {
    ProviderConfig {
        enabled: true,
        api_key: Some("test-key".to_string()),
        base_url: base_url.map(str::to_string),
        stream_idle_timeout_secs: idle,
        ..Default::default()
    }
}

fn openrouter(idle: Option<u64>) -> ProviderConfig {
    ProviderConfig {
        enabled: true,
        api_key: Some("test-key".to_string()),
        stream_idle_timeout_secs: idle,
        ..Default::default()
    }
}

fn config_with(
    zai_cfg: Option<ProviderConfig>,
    openrouter_cfg: Option<ProviderConfig>,
    agent_idle: Option<u64>,
) -> Config {
    Config {
        providers: ProviderConfigs {
            zai: zai_cfg,
            openrouter: openrouter_cfg,
            ..Default::default()
        },
        agent: AgentConfig {
            stream_idle_timeout_secs: agent_idle,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn the_compiled_compat_default_is_120_seconds() {
    assert_eq!(COMPAT_STREAM_IDLE_SECS, 120);
    assert_eq!(compat_idle_default(false), Some(120));
}

#[test]
fn a_local_base_url_keeps_the_runtime_default() {
    assert_eq!(compat_idle_default(true), None);
}

#[tokio::test]
async fn an_unset_remote_compat_provider_gets_the_shared_default() {
    let config = config_with(None, Some(openrouter(None)), None);
    let provider = create_provider(&config).await.expect("openrouter builds");
    assert_eq!(
        provider.stream_idle_timeout(),
        Some(Duration::from_secs(120))
    );
}

#[tokio::test]
async fn z_ai_gets_the_same_shared_default_as_every_other_compat_provider() {
    let config = config_with(Some(zai(None, None)), None, None);
    let provider = create_provider(&config).await.expect("zai builds");
    assert_eq!(
        provider.stream_idle_timeout(),
        Some(Duration::from_secs(120))
    );
}

#[tokio::test]
async fn a_local_zai_host_keeps_the_runtime_default_not_the_compat_one() {
    let config = config_with(
        Some(zai(Some("http://localhost:8080/v1"), None)),
        None,
        None,
    );
    let provider = create_provider(&config).await.expect("local zai builds");
    assert_eq!(provider.stream_idle_timeout(), None);
}

#[tokio::test]
async fn a_per_provider_value_overrides_the_shared_default() {
    let config = config_with(None, Some(openrouter(Some(600))), None);
    let provider = create_provider(&config).await.expect("openrouter builds");
    assert_eq!(
        provider.stream_idle_timeout(),
        Some(Duration::from_secs(600))
    );
}

#[tokio::test]
async fn an_agent_tier_value_overrides_the_shared_default() {
    let config = config_with(None, Some(openrouter(None)), Some(300));
    let provider = create_provider(&config).await.expect("openrouter builds");
    assert_eq!(
        provider.stream_idle_timeout(),
        Some(Duration::from_secs(300))
    );
}
