//! Provider Factory
//!
//! Creates providers based on config.toml settings.

use super::{Provider, anthropic::AnthropicProvider, custom_openai_compatible::OpenAIProvider};
use crate::config::{Config, ProviderConfig};
use anyhow::Result;
use std::sync::Arc;

/// Create a provider based on config.toml
/// No hardcoded priority - providers are enabled/disabled in config
pub fn create_provider(config: &Config) -> Result<Arc<dyn Provider>> {
    // Check which providers are enabled in config.toml

    // Try Minimax first
    if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
        tracing::info!("Using enabled provider: Minimax");
        return try_create_minimax(config)?
            .ok_or_else(|| anyhow::anyhow!("Minimax enabled but failed to create"));
    }

    // Try OpenRouter
    if config
        .providers
        .openrouter
        .as_ref()
        .is_some_and(|p| p.enabled)
    {
        tracing::info!("Using enabled provider: OpenRouter");
        return try_create_openrouter(config)?
            .ok_or_else(|| anyhow::anyhow!("OpenRouter enabled but failed to create"));
    }

    // Try Anthropic
    if config
        .providers
        .anthropic
        .as_ref()
        .is_some_and(|p| p.enabled)
    {
        tracing::info!("Using enabled provider: Anthropic");
        return try_create_anthropic(config)?
            .ok_or_else(|| anyhow::anyhow!("Anthropic enabled but failed to create"));
    }

    // Try OpenAI (official)
    if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
        tracing::info!("Using enabled provider: OpenAI");
        return try_create_openai(config)?
            .ok_or_else(|| anyhow::anyhow!("OpenAI enabled but failed to create"));
    }

    // Try Custom OpenAI-compatible (first enabled named provider)
    if config.providers.active_custom().is_some() {
        tracing::info!("Using enabled provider: Custom OpenAI-Compatible");
        return try_create_custom(config)?
            .ok_or_else(|| anyhow::anyhow!("Custom provider enabled but failed to create"));
    }

    // Try Gemini
    if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
        tracing::info!("Using enabled provider: Google Gemini");
        return Err(anyhow::anyhow!("Gemini provider not yet implemented"));
    }

    // Try fallback if primary fails
    if let Some(fallback) = &config.providers.fallback
        && fallback.enabled
        && let Some(fallback_type) = &fallback.provider
    {
        tracing::warn!(
            "No primary provider enabled, trying fallback: {}",
            fallback_type
        );
        return create_fallback(config, fallback_type);
    }

    // No provider enabled - return placeholder provider so app can start and show onboarding
    tracing::info!("No provider configured, using placeholder provider");
    Ok(Arc::new(super::PlaceholderProvider))
}

/// Create fallback provider
fn create_fallback(config: &Config, fallback_type: &str) -> Result<Arc<dyn Provider>> {
    match fallback_type {
        "openrouter" => {
            tracing::info!("Using fallback: OpenRouter");
            try_create_openrouter(config)?
                .ok_or_else(|| anyhow::anyhow!("OpenRouter not configured"))
        }
        "minimax" => {
            tracing::info!("Using fallback: Minimax");
            try_create_minimax(config)?.ok_or_else(|| anyhow::anyhow!("Minimax not configured"))
        }
        "anthropic" => {
            tracing::info!("Using fallback: Anthropic");
            try_create_anthropic(config)?.ok_or_else(|| anyhow::anyhow!("Anthropic not configured"))
        }
        "openai" => {
            tracing::info!("Using fallback: OpenAI");
            try_create_openai(config)?.ok_or_else(|| anyhow::anyhow!("OpenAI not configured"))
        }
        "custom" => {
            tracing::info!("Using fallback: Custom OpenAI-Compatible");
            try_create_custom(config)?
                .ok_or_else(|| anyhow::anyhow!("Custom provider not configured"))
        }
        _ => Err(anyhow::anyhow!(
            "Unknown fallback provider: {}",
            fallback_type
        )),
    }
}

/// Try to create OpenRouter provider if configured
fn try_create_openrouter(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let openrouter_config = match &config.providers.openrouter {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let Some(api_key) = &openrouter_config.api_key else {
        return Ok(None);
    };

    let base_url = openrouter_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://openrouter.ai/api/v1/chat/completions".to_string());

    tracing::info!("Using OpenRouter at: {}", base_url);
    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url),
        openrouter_config,
    );
    Ok(Some(Arc::new(provider)))
}

/// Try to create Minimax provider if configured
fn try_create_minimax(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let minimax_config = match &config.providers.minimax {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let Some(api_key) = &minimax_config.api_key else {
        return Ok(None);
    };

    // MiniMax requires specific endpoint path, not just /v1
    let base_url = minimax_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.minimax.io/v1".to_string());

    // Append correct path if not already present
    let full_url = if base_url.contains("minimax.io") && !base_url.contains("/text/") {
        format!("{}/text/chatcompletion_v2", base_url.trim_end_matches('/'))
    } else {
        base_url
    };

    tracing::info!("Using Minimax at: {}", full_url);
    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), full_url).with_name("minimax"),
        minimax_config,
    );
    Ok(Some(Arc::new(provider)))
}

/// Try to create Custom OpenAI-compatible provider if configured.
/// Picks the first enabled named custom provider from the map.
fn try_create_custom(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let (name, custom_config) = match config.providers.active_custom() {
        Some((n, c)) => (n.to_string(), c.clone()),
        None => return Ok(None),
    };

    let Some(api_key) = &custom_config.api_key else {
        return Ok(None);
    };

    let mut base_url = custom_config
        .base_url
        .clone()
        .unwrap_or_else(|| "http://localhost:1234/v1/chat/completions".to_string());

    // Auto-append /chat/completions if missing — all OpenAI-compatible APIs need it
    if !base_url.contains("/chat/completions") {
        base_url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    }

    tracing::info!("Using Custom OpenAI-compatible '{}' at: {}", name, base_url);
    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url).with_name(&name),
        &custom_config,
    );
    Ok(Some(Arc::new(provider)))
}

/// Configure OpenAI-compatible provider with custom model
fn configure_openai_compatible(
    mut provider: OpenAIProvider,
    config: &ProviderConfig,
) -> OpenAIProvider {
    tracing::debug!(
        "configure_openai_compatible: default_model = {:?}",
        config.default_model
    );
    if let Some(model) = &config.default_model {
        tracing::info!("Using custom default model: {}", model);
        provider = provider.with_default_model(model.clone());
    }
    provider
}

/// Try to create OpenAI provider if configured
fn try_create_openai(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let openai_config = match &config.providers.openai {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    // Local LLM (LM Studio, Ollama, etc.) - has base_url but NO api_key
    if let Some(base_url) = &openai_config.base_url
        && openai_config.api_key.is_none()
    {
        tracing::info!("Using local LLM at: {}", base_url);
        let provider =
            configure_openai_compatible(OpenAIProvider::local(base_url.clone()), openai_config);
        return Ok(Some(Arc::new(provider)));
    }

    // Official OpenAI API - has api_key
    if let Some(api_key) = &openai_config.api_key {
        tracing::info!("Using OpenAI provider");
        let provider =
            configure_openai_compatible(OpenAIProvider::new(api_key.clone()), openai_config);
        return Ok(Some(Arc::new(provider)));
    }

    Ok(None)
}

/// Try to create Anthropic provider if configured
fn try_create_anthropic(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let anthropic_config = match &config.providers.anthropic {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let api_key = match &anthropic_config.api_key {
        Some(key) => key.clone(),
        None => return Ok(None),
    };

    let mut provider = AnthropicProvider::new(api_key);

    if let Some(model) = &anthropic_config.default_model {
        tracing::info!("Using custom default model: {}", model);
        provider = provider.with_default_model(model.clone());
    }

    tracing::info!("Using Anthropic provider");

    Ok(Some(Arc::new(provider)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ProviderConfig, ProviderConfigs};

    #[test]
    fn test_create_provider_with_anthropic() {
        let config = Config {
            providers: ProviderConfigs {
                anthropic: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("test-key".to_string()),
                    base_url: None,
                    default_model: None,
                    models: vec![],
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = create_provider(&config);
        assert!(result.is_ok());
        let provider = result.unwrap();
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_create_provider_with_minimax() {
        let config = Config {
            providers: ProviderConfigs {
                minimax: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("test-key".to_string()),
                    base_url: Some("https://api.minimax.io/v1".to_string()),
                    default_model: Some("MiniMax-M2.5".to_string()),
                    models: vec![],
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = create_provider(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_minimax_takes_priority() {
        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("openai-key".to_string()),
                    base_url: None,
                    default_model: None,
                    models: vec![],
                }),
                minimax: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("minimax-key".to_string()),
                    base_url: Some("https://api.minimax.io/v1".to_string()),
                    default_model: None,
                    models: vec![],
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = create_provider(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_provider_no_credentials() {
        let config = Config {
            providers: ProviderConfigs::default(),
            ..Default::default()
        };

        // No credentials → PlaceholderProvider (app starts, shows onboarding)
        let result = create_provider(&config);
        assert!(result.is_ok());
    }
}
