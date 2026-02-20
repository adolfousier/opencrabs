//! Provider Factory
//!
//! Centralized provider creation logic to reduce code duplication.

use super::{
    anthropic::AnthropicProvider,
    openai::OpenAIProvider,
    qwen::{QwenProvider, ToolCallParser},
    Provider,
};
use crate::config::{Config, ProviderConfig, QwenProviderConfig};
use anyhow::Result;
use std::sync::Arc;

/// Create a provider based on configuration with fallback priority
///
/// Priority order:
/// 1. Qwen (if explicitly configured with base_url or DashScope key)
/// 2. Anthropic (default — recommended provider)
/// 3. OpenAI (only if Anthropic is unavailable)
///
/// Note: OPENAI_API_KEY may be present just for TTS voice synthesis.
/// OpenAI is only used as the text provider when Anthropic has no credentials.
pub fn create_provider(config: &Config) -> Result<Arc<dyn Provider>> {
    // Check which provider is explicitly ENABLED (user selected in /models or onboard)
    // Priority: Qwen > Anthropic > OpenAI > Gemini

    if config.providers.qwen.as_ref().is_some_and(|p| p.enabled) {
        tracing::info!("Using enabled provider: Qwen/DashScope");
        return try_create_qwen(config)?
            .ok_or_else(|| anyhow::anyhow!("Qwen enabled but failed to create"));
    }

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

    if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
        let is_openrouter = config
            .providers
            .openai
            .as_ref()
            .and_then(|p| p.base_url.as_ref())
            .map(|u| u.contains("openrouter"))
            .unwrap_or(false);
        tracing::info!(
            "Using enabled provider: {}",
            if is_openrouter {
                "OpenRouter"
            } else {
                "OpenAI"
            }
        );
        return try_create_openai(config)?
            .ok_or_else(|| anyhow::anyhow!("OpenAI/OpenRouter enabled but failed to create"));
    }

    if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
        tracing::info!("Using enabled provider: Google Gemini");
        // Gemini creation would go here if implemented
        return Err(anyhow::anyhow!(
            "Gemini provider enabled but not yet implemented"
        ));
    }

    // No provider enabled - user must select one
    Err(anyhow::anyhow!(
        "No provider enabled.\n\nPlease select a provider using /models command."
    ))
}

/// Try to create Qwen provider if configured
fn try_create_qwen(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let qwen_config = match &config.providers.qwen {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    // Local Qwen (vLLM, LM Studio, etc.)
    if let Some(base_url) = &qwen_config.base_url {
        tracing::info!("Using local Qwen at: {}", base_url);

        let provider = configure_qwen(QwenProvider::local(base_url.clone()), qwen_config);
        return Ok(Some(Arc::new(provider)));
    }

    // DashScope cloud API
    if let Some(api_key) = &qwen_config.api_key {
        let region = qwen_config.region.as_deref().unwrap_or("intl");

        let provider_base = match region {
            "cn" => {
                tracing::info!("Using DashScope China (Beijing)");
                QwenProvider::dashscope_cn(api_key.clone())
            }
            _ => {
                tracing::info!("Using DashScope International (Singapore)");
                QwenProvider::dashscope_intl(api_key.clone())
            }
        };

        let provider = configure_qwen(provider_base, qwen_config);
        return Ok(Some(Arc::new(provider)));
    }

    Ok(None)
}

/// Configure Qwen provider with tool parser, thinking mode, and model
fn configure_qwen(mut provider: QwenProvider, config: &QwenProviderConfig) -> QwenProvider {
    // Set tool parser
    if let Some(parser) = &config.tool_parser {
        let tool_parser = match parser.as_str() {
            "openai" => ToolCallParser::OpenAI,
            "native" | "qwen" => ToolCallParser::NativeQwen,
            _ => ToolCallParser::Hermes,
        };
        provider = provider.with_tool_parser(tool_parser);
        tracing::info!("Using tool parser: {:?}", tool_parser);

        if tool_parser == ToolCallParser::NativeQwen {
            tracing::info!("Using native Qwen function calling");
        }
    }

    // Set thinking mode
    if config.enable_thinking {
        provider = provider.with_thinking(true);
        tracing::info!("Qwen3 thinking mode enabled");

        if let Some(budget) = config.thinking_budget {
            provider = provider.with_thinking_budget(budget);
            tracing::info!("Thinking budget: {} tokens", budget);
        }
    }

    // Set custom model
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

    let has_api_key = openai_config.api_key.is_some();
    let has_base_url = openai_config.base_url.is_some();
    let is_openrouter = openai_config
        .base_url
        .as_ref()
        .map(|u| u.contains("openrouter"))
        .unwrap_or(false);

    // OpenRouter: requires both api_key AND base_url
    if is_openrouter && has_api_key {
        let base_url = openai_config.base_url.as_ref().unwrap();
        let api_key = openai_config.api_key.as_ref().unwrap();
        tracing::info!("Using OpenRouter at: {}", base_url);
        let provider = configure_openai(
            OpenAIProvider::with_base_url(api_key.clone(), base_url.clone()),
            openai_config,
        );
        return Ok(Some(Arc::new(provider)));
    }

    // Local LLM (LM Studio, Ollama, etc.) - NO api_key, YES base_url
    if has_base_url && !has_api_key {
        let base_url = openai_config.base_url.as_ref().unwrap();
        tracing::info!("Using local LLM at: {}", base_url);
        let provider = configure_openai(OpenAIProvider::local(base_url.clone()), openai_config);
        return Ok(Some(Arc::new(provider)));
    }

    // Official OpenAI API - has api_key, no base_url (or default)
    if has_api_key {
        tracing::info!("Using OpenAI provider");
        let api_key = openai_config.api_key.as_ref().unwrap();
        let provider = configure_openai(OpenAIProvider::new(api_key.clone()), openai_config);
        return Ok(Some(Arc::new(provider)));
    }

    Ok(None)
}

/// Configure OpenAI provider with custom model
fn configure_openai(mut provider: OpenAIProvider, config: &ProviderConfig) -> OpenAIProvider {
    if let Some(model) = &config.default_model {
        tracing::info!("Using custom default model: {}", model);
        provider = provider.with_default_model(model.clone());
    }
    provider
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
    use crate::config::{Config, ProviderConfig, ProviderConfigs, QwenProviderConfig};

    #[test]
    fn test_create_provider_with_anthropic() {
        let config = Config {
            providers: ProviderConfigs {
                anthropic: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("test-key".to_string()),
                    base_url: None,
                    default_model: None,
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
    fn test_anthropic_takes_priority_over_openai() {
        // When both are configured, Anthropic wins (OpenAI key may be for TTS only)
        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("openai-key".to_string()),
                    base_url: None,
                    default_model: None,
                }),
                anthropic: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("anthropic-key".to_string()),
                    base_url: None,
                    default_model: None,
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
    fn test_openai_used_when_anthropic_unavailable() {
        // OpenAI is only used as text provider when Anthropic has no credentials
        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("openai-key".to_string()),
                    base_url: None,
                    default_model: None,
                }),
                anthropic: None,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = create_provider(&config);
        assert!(result.is_ok());
        let provider = result.unwrap();
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn test_create_provider_with_qwen() {
        let config = Config {
            providers: ProviderConfigs {
                qwen: Some(QwenProviderConfig {
                    enabled: true,
                    api_key: Some("test-key".to_string()),
                    base_url: None,
                    default_model: None,
                    tool_parser: None,
                    enable_thinking: false,
                    thinking_budget: None,
                    region: None,
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = create_provider(&config);
        assert!(result.is_ok());
        let provider = result.unwrap();
        assert_eq!(provider.name(), "qwen");
    }

    #[test]
    fn test_create_provider_no_credentials() {
        let config = Config {
            providers: ProviderConfigs {
                anthropic: None,
                openai: None,
                qwen: None,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = create_provider(&config);
        assert!(result.is_err());
        let err = format!("{}", result.as_ref().err().expect("should be error"));
        assert!(err.contains("No provider configured"), "error: {}", err);
    }
}
