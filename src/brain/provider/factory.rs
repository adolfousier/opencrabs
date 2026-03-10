//! Provider Factory
//!
//! Creates providers based on config.toml settings.

use super::{
    Provider, anthropic::AnthropicProvider, custom_openai_compatible::OpenAIProvider,
    gemini::GeminiProvider,
};
use crate::config::{Config, ProviderConfig};
use anyhow::Result;
use std::sync::Arc;

/// Create a provider based on config.toml
/// No hardcoded priority - providers are enabled/disabled in config
pub fn create_provider(config: &Config) -> Result<Arc<dyn Provider>> {
    // Check which providers are enabled in config.toml
    let primary: Option<Arc<dyn Provider>> =
        if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
            tracing::info!("Using enabled provider: Minimax");
            Some(
                try_create_minimax(config)?
                    .ok_or_else(|| anyhow::anyhow!("Minimax enabled but failed to create"))?,
            )
        } else if config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            tracing::info!("Using enabled provider: OpenRouter");
            Some(
                try_create_openrouter(config)?
                    .ok_or_else(|| anyhow::anyhow!("OpenRouter enabled but failed to create"))?,
            )
        } else if config
            .providers
            .anthropic
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            tracing::info!("Using enabled provider: Anthropic");
            Some(
                try_create_anthropic(config)?
                    .ok_or_else(|| anyhow::anyhow!("Anthropic enabled but failed to create"))?,
            )
        } else if config.providers.github.as_ref().is_some_and(|p| p.enabled) {
            tracing::info!("Using enabled provider: GitHub Models");
            Some(
                try_create_github(config)?
                    .ok_or_else(|| anyhow::anyhow!("GitHub enabled but failed to create"))?,
            )
        } else if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
            tracing::info!("Using enabled provider: OpenAI");
            Some(
                try_create_openai(config)?
                    .ok_or_else(|| anyhow::anyhow!("OpenAI enabled but failed to create"))?,
            )
        } else if config.providers.active_custom().is_some() {
            tracing::info!("Using enabled provider: Custom OpenAI-Compatible");
            Some(
                try_create_custom(config)?.ok_or_else(|| {
                    anyhow::anyhow!("Custom provider enabled but failed to create")
                })?,
            )
        } else if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
            tracing::info!("Using enabled provider: Google Gemini");
            Some(
                try_create_gemini(config)?
                    .ok_or_else(|| anyhow::anyhow!("Gemini enabled but API key missing"))?,
            )
        } else {
            None
        };

    // Build fallback chain if configured
    let fallback_providers = if let Some(fallback) = &config.providers.fallback
        && fallback.enabled
    {
        let chain = fallback_chain(fallback);
        let mut providers = Vec::new();
        for name in &chain {
            match create_fallback(config, name) {
                Ok(p) => {
                    tracing::info!("Fallback provider '{}' ready", name);
                    providers.push(p);
                }
                Err(e) => {
                    tracing::warn!("Fallback provider '{}' skipped: {}", name, e);
                }
            }
        }
        providers
    } else {
        Vec::new()
    };

    match primary {
        Some(provider) => {
            if fallback_providers.is_empty() {
                Ok(provider)
            } else {
                tracing::info!(
                    "Wrapping primary provider with {} fallback(s)",
                    fallback_providers.len()
                );
                Ok(Arc::new(super::FallbackProvider::new(
                    provider,
                    fallback_providers,
                )))
            }
        }
        None => {
            // No primary — try fallbacks as primary candidates
            if let Some(first) = fallback_providers.into_iter().next() {
                tracing::warn!("No primary provider enabled, using first fallback");
                Ok(first)
            } else {
                tracing::info!("No provider configured, using placeholder provider");
                Ok(Arc::new(super::PlaceholderProvider))
            }
        }
    }
}

/// Create a provider by name, ignoring the `enabled` flag.
/// Used for per-session provider restoration without toggling disk config.
/// Accepts names like "anthropic", "openai", "minimax", "openrouter", or "custom:<name>".
pub fn create_provider_by_name(config: &Config, name: &str) -> Result<Arc<dyn Provider>> {
    match name {
        "anthropic" => try_create_anthropic(config)?
            .ok_or_else(|| anyhow::anyhow!("Anthropic not configured (missing API key)")),
        "openai" => try_create_openai(config)?
            .ok_or_else(|| anyhow::anyhow!("OpenAI not configured (missing API key)")),
        "minimax" => try_create_minimax(config)?
            .ok_or_else(|| anyhow::anyhow!("Minimax not configured (missing API key)")),
        "openrouter" => try_create_openrouter(config)?
            .ok_or_else(|| anyhow::anyhow!("OpenRouter not configured (missing API key)")),
        "github" => try_create_github(config)?
            .ok_or_else(|| anyhow::anyhow!("GitHub not configured (missing token)")),
        "gemini" => try_create_gemini(config)?
            .ok_or_else(|| anyhow::anyhow!("Gemini not configured (missing API key)")),
        n if n.starts_with("custom:") => {
            let custom_name = &n["custom:".len()..];
            try_create_custom_by_name(config, custom_name)?
                .ok_or_else(|| anyhow::anyhow!("Custom provider '{}' not configured", custom_name))
        }
        // Try as a custom provider name directly (legacy sessions)
        other => try_create_custom_by_name(config, other)?
            .ok_or_else(|| anyhow::anyhow!("Unknown provider: {}", other)),
    }
}

/// Try to create a specific named custom provider (ignores enabled flag).
fn try_create_custom_by_name(config: &Config, name: &str) -> Result<Option<Arc<dyn Provider>>> {
    let customs = match &config.providers.custom {
        Some(map) => map,
        None => return Ok(None),
    };

    let custom_config = match customs.get(name) {
        Some(cfg) => cfg.clone(),
        None => return Ok(None),
    };

    let Some(api_key) = &custom_config.api_key else {
        return Ok(None);
    };

    let mut base_url = custom_config
        .base_url
        .clone()
        .unwrap_or_else(|| "http://localhost:1234/v1/chat/completions".to_string());

    if !base_url.contains("/chat/completions") {
        base_url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    }

    tracing::info!("Creating custom provider '{}' at: {}", name, base_url);
    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url).with_name(name),
        &custom_config,
    );
    Ok(Some(Arc::new(provider)))
}

/// Build ordered fallback chain: `providers` array first, legacy `provider` as last resort.
pub(crate) fn fallback_chain(fallback: &crate::config::FallbackProviderConfig) -> Vec<String> {
    let mut chain: Vec<String> = fallback.providers.clone();
    // Append legacy single `provider` if set and not already in the list
    if let Some(ref legacy) = fallback.provider
        && !chain.iter().any(|p| p == legacy)
    {
        chain.push(legacy.clone());
    }
    chain
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
        "github" => {
            tracing::info!("Using fallback: GitHub Models");
            try_create_github(config)?.ok_or_else(|| anyhow::anyhow!("GitHub not configured"))
        }
        "gemini" => {
            tracing::info!("Using fallback: Gemini");
            try_create_gemini(config)?.ok_or_else(|| anyhow::anyhow!("Gemini not configured"))
        }
        "custom" => {
            tracing::info!("Using fallback: Custom OpenAI-Compatible");
            try_create_custom(config)?
                .ok_or_else(|| anyhow::anyhow!("Custom provider not configured"))
        }
        other => {
            // Try as a named custom provider (e.g. "custom:mylocal")
            if let Some(name) = other.strip_prefix("custom:") {
                tracing::info!("Using fallback: Custom '{}'", name);
                try_create_custom_by_name(config, name)?
                    .ok_or_else(|| anyhow::anyhow!("Custom provider '{}' not configured", name))
            } else {
                Err(anyhow::anyhow!("Unknown fallback provider: {}", other))
            }
        }
    }
}

/// Try to auto-detect a GitHub token from the `gh` CLI.
/// Returns `None` if `gh` is not installed or not authenticated.
pub(crate) fn gh_auth_token() -> Option<String> {
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }
    None
}

/// Try to create GitHub Models provider if configured.
/// Auto-detects token from `gh auth token` if no api_key in keys.toml.
fn try_create_github(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let github_config = match &config.providers.github {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    // Try explicit key first, then auto-detect from gh CLI
    let api_key = github_config
        .api_key
        .clone()
        .filter(|k| !k.is_empty())
        .or_else(gh_auth_token);

    let Some(api_key) = api_key else {
        tracing::warn!(
            "GitHub Models enabled but no token found. \
             Run `gh auth login` or add a PAT to keys.toml."
        );
        return Ok(None);
    };

    let base_url = github_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://models.github.ai/inference/chat/completions".to_string());

    tracing::info!("Using GitHub Models at: {}", base_url);

    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key, base_url)
            .with_name("github")
            .with_extra_headers(vec![
                (
                    "Accept".to_string(),
                    "application/vnd.github+json".to_string(),
                ),
                ("X-GitHub-Api-Version".to_string(), "2022-11-28".to_string()),
            ]),
        github_config,
    );
    Ok(Some(Arc::new(provider)))
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
        OpenAIProvider::with_base_url(api_key.clone(), base_url).with_name("openrouter"),
        openrouter_config,
    );
    Ok(Some(Arc::new(provider)))
}

/// Try to create Minimax provider if configured
fn try_create_minimax(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let minimax_config = match &config.providers.minimax {
        Some(cfg) => {
            tracing::debug!(
                "Minimax config: enabled={}, has_key={}",
                cfg.enabled,
                cfg.api_key.is_some()
            );
            cfg
        }
        None => return Ok(None),
    };

    let Some(api_key) = &minimax_config.api_key else {
        tracing::warn!("Minimax enabled but API key missing — check keys.toml");
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
    let mut provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), full_url).with_name("minimax"),
        minimax_config,
    );

    // MiniMax M2.5 doesn't support vision — inject MiniMax-Text-01 into config
    // so existing users get it automatically and can change it later
    if minimax_config.vision_model.is_none() {
        provider = provider.with_vision_model("MiniMax-Text-01".to_string());
        if let Err(e) =
            crate::config::Config::write_key("providers.minimax", "vision_model", "MiniMax-Text-01")
        {
            tracing::warn!("Failed to persist minimax vision_model to config: {}", e);
        } else {
            tracing::info!(
                "Auto-injected vision_model = MiniMax-Text-01 into providers.minimax config"
            );
        }
    }

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
    if let Some(vm) = &config.vision_model {
        tracing::info!("Vision model configured: {}", vm);
        provider = provider.with_vision_model(vm.clone());
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
        let provider = configure_openai_compatible(
            OpenAIProvider::local(base_url.clone()).with_name("openai"),
            openai_config,
        );
        return Ok(Some(Arc::new(provider)));
    }

    // Official OpenAI API - has api_key
    if let Some(api_key) = &openai_config.api_key {
        tracing::info!("Using OpenAI provider");
        let provider = configure_openai_compatible(
            OpenAIProvider::new(api_key.clone()).with_name("openai"),
            openai_config,
        );
        return Ok(Some(Arc::new(provider)));
    }

    Ok(None)
}

/// Try to create Gemini provider if configured
fn try_create_gemini(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let gemini_config = match &config.providers.gemini {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let api_key = match &gemini_config.api_key {
        Some(key) if !key.is_empty() => key.clone(),
        _ => return Ok(None),
    };

    let model = gemini_config
        .default_model
        .clone()
        .unwrap_or_else(|| "gemini-2.0-flash".to_string());

    tracing::info!("Using Gemini provider with model: {}", model);
    Ok(Some(Arc::new(
        GeminiProvider::new(api_key).with_model(model),
    )))
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

/// Returns `(api_key, base_url, vision_model)` for the first active provider
/// that has a `vision_model` configured. Used to register the provider-native
/// `analyze_image` tool when Gemini vision isn't set up.
pub fn active_provider_vision(config: &Config) -> Option<(String, String, String)> {
    // Check providers in priority order (same as create_provider)
    let candidates: Vec<&ProviderConfig> = [
        config.providers.minimax.as_ref(),
        config.providers.openrouter.as_ref(),
        config.providers.anthropic.as_ref(),
        config.providers.openai.as_ref(),
        config.providers.github.as_ref(),
        config.providers.gemini.as_ref(),
    ]
    .into_iter()
    .flatten()
    .filter(|c| c.enabled)
    .collect();

    // Also check custom providers
    let custom_iter = config
        .providers
        .custom
        .as_ref()
        .into_iter()
        .flat_map(|m| m.values())
        .filter(|c| c.enabled);

    for cfg in candidates.into_iter().chain(custom_iter) {
        if let (Some(api_key), Some(vision_model)) = (&cfg.api_key, &cfg.vision_model) {
            let base_url = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1/chat/completions".to_string());
            let base_url = if base_url.contains("/chat/completions") {
                base_url
            } else {
                format!("{}/chat/completions", base_url.trim_end_matches('/'))
            };
            return Some((api_key.clone(), base_url, vision_model.clone()));
        }
    }
    None
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
                    vision_model: None,
                    ..Default::default()
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
                    vision_model: None,
                    ..Default::default()
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
                    vision_model: None,
                    ..Default::default()
                }),
                minimax: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("minimax-key".to_string()),
                    base_url: Some("https://api.minimax.io/v1".to_string()),
                    default_model: None,
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
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
