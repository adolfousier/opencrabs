//! Provider Factory
//!
//! Creates providers based on config.toml settings.

use super::qwen::{
    QWEN_DEFAULT_CHAT_URL, QWEN_OAUTH_MODEL, QwenCredentials, QwenTokenManager,
    qwen_body_transform, qwen_extra_headers,
};
use super::{
    Provider, anthropic::AnthropicProvider, claude_cli::ClaudeCliProvider,
    custom_openai_compatible::OpenAIProvider, gemini::GeminiProvider,
    opencode_cli::OpenCodeCliProvider, qwen_code::QwenCodeCliProvider,
};
use crate::config::{Config, ProviderConfig};
use anyhow::Result;
use std::sync::Arc;

type ProviderAttempt<'a> = (
    &'a str,
    Box<dyn FnOnce() -> Result<Option<Arc<dyn Provider>>> + 'a>,
);

/// Create a provider based on config.toml
/// No hardcoded priority - providers are enabled/disabled in config
pub fn create_provider(config: &Config) -> Result<Arc<dyn Provider>> {
    let (provider, warning) = create_provider_with_warning(config)?;
    if let Some(msg) = &warning {
        tracing::warn!("{}", msg);
    }
    Ok(provider)
}

/// Like `create_provider` but returns a warning message when a fallback was used.
/// The caller (TUI) should surface this to the user instead of printing to stderr.
pub fn create_provider_with_warning(
    config: &Config,
) -> Result<(Arc<dyn Provider>, Option<String>)> {
    // Try the enabled provider. If it fails, warn and try others before giving up.
    // Priority order: Claude CLI > OpenCode CLI > Qwen CLI > Anthropic > OpenAI > GitHub > Gemini > OpenRouter > Minimax > zhipu > Custom
    let enabled_attempts: Vec<ProviderAttempt<'_>> = vec![
        ("Claude CLI", Box::new(|| try_create_claude_cli(config))),
        ("OpenCode CLI", Box::new(|| try_create_opencode_cli(config))),
        ("Qwen CLI", Box::new(|| try_create_qwen_code(config))),
        ("Qwen", Box::new(|| try_create_qwen(config))),
        ("Anthropic", Box::new(|| try_create_anthropic(config))),
        ("OpenAI", Box::new(|| try_create_openai(config))),
        ("GitHub Copilot", Box::new(|| try_create_github(config))),
        ("Google Gemini", Box::new(|| try_create_gemini(config))),
        ("OpenRouter", Box::new(|| try_create_openrouter(config))),
        ("Minimax", Box::new(|| try_create_minimax(config))),
        ("z.ai GLM", Box::new(|| try_create_zhipu(config))),
        ("Custom", Box::new(|| try_create_custom(config))),
    ];

    // Which providers are enabled in config?
    let is_enabled: Vec<bool> = vec![
        config
            .providers
            .claude_cli
            .as_ref()
            .is_some_and(|p| p.enabled),
        config
            .providers
            .opencode_cli
            .as_ref()
            .is_some_and(|p| p.enabled),
        config
            .providers
            .qwen_code_cli
            .as_ref()
            .is_some_and(|p| p.enabled),
        config.providers.qwen.as_ref().is_some_and(|p| p.enabled),
        config
            .providers
            .anthropic
            .as_ref()
            .is_some_and(|p| p.enabled),
        config.providers.openai.as_ref().is_some_and(|p| p.enabled),
        config.providers.github.as_ref().is_some_and(|p| p.enabled),
        config.providers.gemini.as_ref().is_some_and(|p| p.enabled),
        config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled),
        config.providers.minimax.as_ref().is_some_and(|p| p.enabled),
        config.providers.zhipu.as_ref().is_some_and(|p| p.enabled),
        config.providers.active_custom().is_some(),
    ];

    let mut primary: Option<Arc<dyn Provider>> = None;
    let mut failed_name: Option<&str> = None;
    let mut warning: Option<String> = None;

    for (i, (name, create_fn)) in enabled_attempts.into_iter().enumerate() {
        if !is_enabled[i] {
            continue;
        }

        match create_fn() {
            Ok(Some(provider)) => {
                if let Some(failed) = failed_name {
                    let msg = format!(
                        "{} failed to initialize — fell back to {}. Run /onboard:provider to reconfigure.",
                        failed, name
                    );
                    tracing::warn!("{}", msg);
                    warning = Some(msg);
                }
                tracing::info!("Using enabled provider: {}", name);
                primary = Some(provider);
                break;
            }
            Ok(None) => {
                tracing::warn!(
                    "{} enabled but could not be created (missing API key?)",
                    name
                );
                if failed_name.is_none() {
                    failed_name = Some(name);
                }
                // Continue to try next enabled provider
            }
            Err(e) => {
                tracing::error!("{} provider error: {}", name, e);
                if failed_name.is_none() {
                    failed_name = Some(name);
                }
                // Continue to try next enabled provider
            }
        }
    }

    // If the enabled provider failed, try ALL providers as fallback (any with keys)
    if primary.is_none() && failed_name.is_some() {
        let fallback_attempts: Vec<ProviderAttempt<'_>> = vec![
            ("Claude CLI", Box::new(|| try_create_claude_cli(config))),
            ("OpenCode CLI", Box::new(|| try_create_opencode_cli(config))),
            ("Qwen CLI", Box::new(|| try_create_qwen_code(config))),
            ("Qwen", Box::new(|| try_create_qwen(config))),
            ("Anthropic", Box::new(|| try_create_anthropic(config))),
            ("OpenAI", Box::new(|| try_create_openai(config))),
            ("GitHub Copilot", Box::new(|| try_create_github(config))),
            ("Google Gemini", Box::new(|| try_create_gemini(config))),
            ("OpenRouter", Box::new(|| try_create_openrouter(config))),
            ("Minimax", Box::new(|| try_create_minimax(config))),
            ("z.ai GLM", Box::new(|| try_create_zhipu(config))),
            ("Custom", Box::new(|| try_create_custom(config))),
        ];

        for (name, create_fn) in fallback_attempts {
            if let Ok(Some(provider)) = create_fn() {
                let msg = format!(
                    "Fell back to {}. Run /onboard:provider to reconfigure.",
                    name
                );
                tracing::warn!("{}", msg);
                warning = Some(msg);
                primary = Some(provider);
                break;
            }
        }
    }

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
                Ok((provider, warning))
            } else {
                tracing::info!(
                    "Wrapping primary provider with {} fallback(s)",
                    fallback_providers.len()
                );
                Ok((
                    Arc::new(super::FallbackProvider::new(provider, fallback_providers)),
                    warning,
                ))
            }
        }
        None => {
            // No primary — try fallbacks as primary candidates
            if let Some(first) = fallback_providers.into_iter().next() {
                tracing::warn!("No primary provider enabled, using first fallback");
                Ok((first, warning))
            } else {
                tracing::info!("No provider configured, using placeholder provider");
                Ok((Arc::new(super::PlaceholderProvider), warning))
            }
        }
    }
}

/// Create a provider by name, ignoring the `enabled` flag.
/// Used for per-session provider restoration without toggling disk config.
/// Accepts names like "anthropic", "openai", "minimax", "openrouter", or "custom:<name>".
pub fn create_provider_by_name(config: &Config, name: &str) -> Result<Arc<dyn Provider>> {
    match name {
        "claude-cli" | "claude_cli" => {
            // Bypass enabled check — session explicitly requested this provider
            let model = config
                .providers
                .claude_cli
                .as_ref()
                .and_then(|c| c.default_model.clone());
            match ClaudeCliProvider::new() {
                Ok(mut provider) => {
                    if let Some(m) = model {
                        provider = provider.with_default_model(m);
                    }
                    Ok(Arc::new(provider))
                }
                Err(e) => Err(anyhow::anyhow!("Claude CLI binary not found: {}", e)),
            }
        }
        "opencode" | "opencode-cli" | "opencode_cli" => {
            // Bypass enabled check — session explicitly requested this provider
            let model = config
                .providers
                .opencode_cli
                .as_ref()
                .and_then(|c| c.default_model.clone());
            match OpenCodeCliProvider::new() {
                Ok(mut provider) => {
                    if let Some(m) = model {
                        provider = provider.with_default_model(m);
                    }
                    Ok(Arc::new(provider))
                }
                Err(e) => Err(anyhow::anyhow!("OpenCode CLI binary not found: {}", e)),
            }
        }
        "qwen-cli" | "qwen-code" | "qwen_code" | "qwen-code-cli" | "qwen_code_cli" => {
            // Bypass enabled check — session explicitly requested this provider
            let model = config
                .providers
                .qwen_code_cli
                .as_ref()
                .and_then(|c| c.default_model.clone());
            match QwenCodeCliProvider::new() {
                Ok(mut provider) => {
                    if let Some(m) = model {
                        provider = provider.with_default_model(m);
                    }
                    Ok(Arc::new(provider))
                }
                Err(e) => Err(anyhow::anyhow!("Qwen Code binary not found: {}", e)),
            }
        }
        "anthropic" => try_create_anthropic(config)?
            .ok_or_else(|| anyhow::anyhow!("Anthropic not configured (missing API key)")),
        "openai" => try_create_openai(config)?
            .ok_or_else(|| anyhow::anyhow!("OpenAI not configured (missing API key)")),
        "minimax" => try_create_minimax(config)?
            .ok_or_else(|| anyhow::anyhow!("Minimax not configured (missing API key)")),
        "zhipu" => try_create_zhipu(config)?
            .ok_or_else(|| anyhow::anyhow!("z.ai GLM not configured (missing API key)")),
        "openrouter" => try_create_openrouter(config)?
            .ok_or_else(|| anyhow::anyhow!("OpenRouter not configured (missing API key)")),
        "github" => try_create_github(config)?
            .ok_or_else(|| anyhow::anyhow!("GitHub not configured (missing token)")),
        "gemini" => try_create_gemini(config)?
            .ok_or_else(|| anyhow::anyhow!("Gemini not configured (missing API key)")),
        "qwen" => try_create_qwen(config)?
            .ok_or_else(|| anyhow::anyhow!("Qwen not configured (run /onboard:provider)")),
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
        None => {
            tracing::warn!("Custom provider '{}' not found in config", name);
            return Ok(None);
        }
    };

    // API key is optional for local providers (LM Studio, Ollama, etc.)
    let api_key = custom_config.api_key.clone().unwrap_or_default();

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
        "claude-cli" | "claude_cli" => {
            tracing::info!("Using fallback: Claude CLI");
            try_create_claude_cli(config)?
                .ok_or_else(|| anyhow::anyhow!("Claude CLI not available"))
        }
        "opencode" | "opencode-cli" | "opencode_cli" => {
            tracing::info!("Using fallback: OpenCode CLI");
            try_create_opencode_cli(config)?
                .ok_or_else(|| anyhow::anyhow!("OpenCode CLI not available"))
        }
        "openrouter" => {
            tracing::info!("Using fallback: OpenRouter");
            try_create_openrouter(config)?
                .ok_or_else(|| anyhow::anyhow!("OpenRouter not configured"))
        }
        "minimax" => {
            tracing::info!("Using fallback: Minimax");
            try_create_minimax(config)?.ok_or_else(|| anyhow::anyhow!("Minimax not configured"))
        }
        "zhipu" => {
            tracing::info!("Using fallback: z.ai GLM");
            try_create_zhipu(config)?.ok_or_else(|| anyhow::anyhow!("z.ai GLM not configured"))
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
            tracing::info!("Using fallback: GitHub Copilot");
            try_create_github(config)?.ok_or_else(|| anyhow::anyhow!("GitHub not configured"))
        }
        "gemini" => {
            tracing::info!("Using fallback: Gemini");
            try_create_gemini(config)?.ok_or_else(|| anyhow::anyhow!("Gemini not configured"))
        }
        "qwen-cli" | "qwen-code" | "qwen_code" | "qwen-code-cli" | "qwen_code_cli" => {
            tracing::info!("Using fallback: Qwen Code");
            try_create_qwen_code(config)?.ok_or_else(|| anyhow::anyhow!("Qwen Code not available"))
        }
        "qwen" => {
            tracing::info!("Using fallback: Qwen native");
            try_create_qwen(config)?.ok_or_else(|| anyhow::anyhow!("Qwen native not configured"))
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

/// Try to create GitHub Copilot provider if configured.
/// Uses OAuth token to exchange for short-lived Copilot API tokens.
fn try_create_github(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    use super::copilot::{COPILOT_CHAT_URL, CopilotTokenManager, copilot_extra_headers};

    let github_config = match &config.providers.github {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let oauth_token = github_config.api_key.clone().filter(|k| !k.is_empty());

    let Some(oauth_token) = oauth_token else {
        tracing::warn!(
            "GitHub Copilot enabled but no OAuth token found. \
             Run /onboard:provider to authenticate."
        );
        return Ok(None);
    };

    // Create the token manager — background task does the initial + recurring refresh
    let manager = Arc::new(CopilotTokenManager::new(oauth_token));
    manager.clone().start_background_refresh();

    // Build a token_fn closure that reads the cached Copilot token
    let mgr_clone = manager.clone();
    let token_fn: super::custom_openai_compatible::TokenFn =
        Arc::new(move || mgr_clone.get_cached_token());

    let base_url = github_config
        .base_url
        .clone()
        .unwrap_or_else(|| COPILOT_CHAT_URL.to_string());

    tracing::info!("Using GitHub Copilot at: {}", base_url);

    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url("copilot-managed".to_string(), base_url)
            .with_name("GitHub Copilot")
            .with_token_fn(token_fn)
            .with_extra_headers(copilot_extra_headers()),
        github_config,
    );
    Ok(Some(Arc::new(provider)))
}

/// Try to create the native **Qwen** provider (OAuth + DashScope OpenAI-compatible).
///
/// Reads credentials from `keys.toml [providers.qwen]`. If they're missing
/// but the user has a `~/.qwen/oauth_creds.json` left behind by `qwen-code-cli`,
/// imports that into keys.toml on first use so the migration is invisible.
///
/// On 401/403, the token is force-refreshed once via the background manager.
fn try_create_qwen(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let qwen_config = match &config.providers.qwen {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    // 1) Try keys.toml directly.
    let mut creds = QwenCredentials::from_provider_config(qwen_config);

    // 2) Also peek at qwen-code-cli's shared file. If it has a NEWER expiry
    //    than keys.toml (i.e. the user reauthed via `qwen` cli since our last
    //    import), prefer it and overwrite keys.toml. This makes reauth "just
    //    work" without requiring the user to re-run onboarding in opencrabs.
    if let Some(imported) = QwenCredentials::import_from_qwen_cli() {
        let should_use_file = match &creds {
            None => true,
            Some(existing) => imported.expiry_date > existing.expiry_date,
        };
        if should_use_file {
            if let Err(e) = imported.persist_to_keys() {
                tracing::warn!(
                    "Failed to sync ~/.qwen/oauth_creds.json into keys.toml: {}",
                    e
                );
            } else {
                tracing::info!(
                    "Synced Qwen credentials from ~/.qwen/oauth_creds.json into keys.toml \
                     (file expiry={}, keys.toml expiry={:?})",
                    imported.expiry_date,
                    creds.as_ref().map(|c| c.expiry_date)
                );
            }
            creds = Some(imported);
        }
    }

    let Some(creds) = creds else {
        tracing::warn!(
            "Qwen enabled but no OAuth credentials found in keys.toml \
             or ~/.qwen/oauth_creds.json. Run /onboard:provider to authenticate."
        );
        return Ok(None);
    };

    let manager = Arc::new(QwenTokenManager::new(creds));
    manager.clone().start_background_refresh();

    let mgr_for_token = manager.clone();
    let token_fn: super::custom_openai_compatible::TokenFn =
        Arc::new(move || mgr_for_token.current_access_token());

    // Dynamic base-URL callback — qwen's `resource_url` can change after a
    // token refresh, so we resolve the chat endpoint on every request
    // instead of freezing it at startup.
    let mgr_for_url = manager.clone();
    let qwen_override_url = qwen_config.base_url.clone();
    let base_url_fn: super::custom_openai_compatible::BaseUrlFn = Arc::new(move || {
        if let Some(ref u) = qwen_override_url
            && !u.is_empty()
        {
            return u.clone();
        }
        let live = mgr_for_url.current_chat_url();
        if live.is_empty() {
            QWEN_DEFAULT_CHAT_URL.to_string()
        } else {
            live
        }
    });

    // Auth-refresh callback — on 401/403, force-refresh the token before
    // retrying. The returned token_fn closure picks up the new access_token
    // automatically.
    let mgr_for_auth = manager.clone();
    let auth_refresh_fn: super::custom_openai_compatible::AuthRefreshFn = Arc::new(move || {
        let mgr = mgr_for_auth.clone();
        Box::pin(async move {
            mgr.force_refresh()
                .await
                .map_err(|e| format!("qwen force_refresh failed: {}", e))
        })
    });

    let base_url = qwen_config
        .base_url
        .clone()
        .unwrap_or_else(|| manager.current_chat_url());
    let base_url = if base_url.is_empty() {
        QWEN_DEFAULT_CHAT_URL.to_string()
    } else {
        base_url
    };

    tracing::info!("Using Qwen native provider at: {}", base_url);

    // Default to the free-tier OAuth model unless the user pinned another one.
    let mut effective_config = qwen_config.clone();
    if effective_config.default_model.is_none() {
        effective_config.default_model = Some(QWEN_OAUTH_MODEL.to_string());
    }

    // Proactive pacing: the Qwen free tier is 60 req/min (1 req/s sustained).
    // Our tool loop bursts well past that on multi-tool turns, which triggers
    // instant 429s from portal.qwen.ai even with a valid token. 1500ms =
    // 40 req/min, leaves 33% headroom under the quota, and still fires fast
    // enough to keep interactive latency reasonable. qwen-cli gets this
    // pacing for free from the openai npm SDK; we have to wire it explicitly.
    let qwen_limiter = Arc::new(super::rate_limiter::RateLimiter::new(
        std::time::Duration::from_millis(1500),
    ));

    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url("qwen-managed".to_string(), base_url)
            .with_name("qwen")
            .with_token_fn(token_fn)
            .with_base_url_fn(base_url_fn)
            .with_auth_refresh_fn(auth_refresh_fn)
            .with_extra_headers(qwen_extra_headers())
            .with_body_transform(Arc::new(qwen_body_transform))
            .with_rate_limiter(qwen_limiter),
        &effective_config,
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
        tracing::warn!("OpenRouter enabled but API key missing — check keys.toml");
        return Ok(None);
    };

    let base_url = openrouter_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://openrouter.ai/api/v1/chat/completions".to_string());

    let model = openrouter_config
        .default_model
        .clone()
        .unwrap_or_else(|| "openai/gpt-4o".to_string());

    let is_free = model.ends_with(":free");

    tracing::info!(
        "Using OpenRouter at: {} (model={}, free={})",
        base_url,
        model,
        is_free
    );
    let mut provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url)
            .with_name("openrouter")
            .with_default_model(model.clone())
            .with_extra_headers(vec![
                ("X-Title".to_string(), "Open Crabs".to_string()),
                (
                    "HTTP-Referer".to_string(),
                    "https://opencrabs.com".to_string(),
                ),
            ]),
        openrouter_config,
    );

    // Proactive pacing for :free models — per-model rate limiter shared across
    // all provider instances. Enforces 4s between requests per model (~15 req/min,
    // safely under OpenRouter's 20 req/min window with 25% headroom).
    if is_free {
        let limiter = super::rate_limiter::OPENROUTER_FREE_LIMITERS.get(&model);
        provider = provider.with_rate_limiter(limiter);
        tracing::info!("Rate limiter attached for :free model: {}", model);
    }

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

    // MiniMax M2.7/M2.5 doesn't support vision — default to MiniMax-Text-01 in-memory.
    // Do NOT write to config here — this runs inside ConfigWatcher callbacks and
    // writing triggers another reload → infinite loop → crash.
    if minimax_config.vision_model.is_none() {
        provider = provider.with_vision_model("MiniMax-Text-01".to_string());
    }

    Ok(Some(Arc::new(provider)))
}

/// Try to create z.ai GLM provider if configured
/// Supports two endpoint types: "api" (general) or "coding" (coding-specific)
fn try_create_zhipu(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let zhipu_config = match &config.providers.zhipu {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let Some(api_key) = &zhipu_config.api_key else {
        tracing::warn!("z.ai GLM enabled but API key missing — check keys.toml");
        return Ok(None);
    };

    // Determine base URL based on endpoint_type
    // API endpoint: https://api.z.ai/api/paas/v4
    // Coding endpoint: https://api.z.ai/api/coding/paas/v4
    let base_url = match zhipu_config.endpoint_type.as_deref() {
        Some("coding") => "https://api.z.ai/api/coding/paas/v4/chat/completions",
        _ => "https://api.z.ai/api/paas/v4/chat/completions",
    };

    tracing::info!(
        "Using z.ai GLM at: {} (endpoint_type: {:?})",
        base_url,
        zhipu_config.endpoint_type
    );
    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url.to_string()).with_name("zhipu"),
        zhipu_config,
    );
    Ok(Some(Arc::new(provider)))
}

/// Try to create Custom OpenAI-compatible provider if configured.
/// Picks the first enabled named custom provider from the map.
fn try_create_custom(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let (name, custom_config) = match config.providers.active_custom() {
        Some((n, c)) => (n.to_string(), c.clone()),
        None => {
            tracing::warn!("Custom provider requested but no active custom provider found");
            return Ok(None);
        }
    };

    // API key is optional for local providers (LM Studio, Ollama, etc.)
    let api_key = custom_config.api_key.clone().unwrap_or_default();

    let mut base_url = custom_config
        .base_url
        .clone()
        .unwrap_or_else(|| "http://localhost:1234/v1/chat/completions".to_string());

    // Auto-append /chat/completions if missing — all OpenAI-compatible APIs need it
    if !base_url.contains("/chat/completions") {
        base_url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    }

    tracing::info!(
        "Using Custom OpenAI-compatible '{}' at: {} (has_key={})",
        name,
        base_url,
        !api_key.is_empty()
    );
    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key, base_url).with_name(&name),
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
    if let Some(cw) = config.context_window {
        tracing::info!("Context window configured: {} tokens", cw);
        provider = provider.with_context_window(cw);
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

    tracing::warn!("OpenAI enabled but no API key and no base_url — check keys.toml");
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
        _ => {
            tracing::warn!("Gemini enabled but API key missing — check keys.toml");
            return Ok(None);
        }
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

/// Try to create Claude CLI provider if configured and binary is available.
fn try_create_claude_cli(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let cli_config = match &config.providers.claude_cli {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    match ClaudeCliProvider::new() {
        Ok(mut provider) => {
            if let Some(model) = &cli_config.default_model {
                provider = provider.with_default_model(model.clone());
            }
            tracing::info!("Using Claude CLI provider (Max subscription, no API key needed)");
            Ok(Some(Arc::new(provider)))
        }
        Err(e) => {
            tracing::warn!("Claude CLI enabled but binary not found: {}", e);
            Ok(None)
        }
    }
}

/// Try to create Qwen Code CLI provider if configured and binary is available.
fn try_create_qwen_code(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let cli_config = match &config.providers.qwen_code_cli {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    match QwenCodeCliProvider::new() {
        Ok(mut provider) => {
            if let Some(model) = &cli_config.default_model {
                provider = provider.with_default_model(model.clone());
            }
            tracing::info!("Using Qwen Code CLI provider (1k free req/day via Qwen OAuth)");
            Ok(Some(Arc::new(provider)))
        }
        Err(e) => {
            tracing::warn!("Qwen Code enabled but binary not found: {}", e);
            Ok(None)
        }
    }
}

/// Try to create OpenCode CLI provider if configured and binary is available.
fn try_create_opencode_cli(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let cli_config = match &config.providers.opencode_cli {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    match OpenCodeCliProvider::new() {
        Ok(mut provider) => {
            if let Some(model) = &cli_config.default_model {
                provider = provider.with_default_model(model.clone());
            }
            tracing::info!("Using OpenCode CLI provider (free models, no API key needed)");
            Ok(Some(Arc::new(provider)))
        }
        Err(e) => {
            tracing::warn!("OpenCode CLI enabled but binary not found: {}", e);
            Ok(None)
        }
    }
}

/// Try to create Anthropic provider if configured
fn try_create_anthropic(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let anthropic_config = match &config.providers.anthropic {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let api_key = match &anthropic_config.api_key {
        Some(key) => key.clone(),
        None => {
            tracing::warn!("Anthropic enabled but API key missing — check keys.toml");
            return Ok(None);
        }
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
    // Get the ACTUAL active provider — don't iterate all providers,
    // otherwise MiniMax wins just because it's listed before OpenRouter.
    let (name, _) = config.providers.active_provider_and_model();

    let active_cfg: Option<&ProviderConfig> = match name.as_str() {
        "minimax" => config.providers.minimax.as_ref(),
        "zhipu" => config.providers.zhipu.as_ref(),
        "openrouter" => config.providers.openrouter.as_ref(),
        "anthropic" => config.providers.anthropic.as_ref(),
        "openai" => config.providers.openai.as_ref(),
        "github" => config.providers.github.as_ref(),
        "gemini" => config.providers.gemini.as_ref(),
        "claude-cli" | "claude_cli" => config.providers.claude_cli.as_ref(),
        "opencode" | "opencode-cli" | "opencode_cli" => config.providers.opencode_cli.as_ref(),
        "qwen-cli" | "qwen-code" | "qwen_code" | "qwen-code-cli" | "qwen_code_cli" => {
            config.providers.qwen_code_cli.as_ref()
        }
        "qwen" => config.providers.qwen.as_ref(),
        cn if cn.starts_with("custom:") => config
            .providers
            .custom
            .as_ref()
            .and_then(|m| m.get(&cn["custom:".len()..])),
        _ => None,
    };

    if let Some(cfg) = active_cfg
        && let (Some(api_key), Some(vision_model)) = (&cfg.api_key, &cfg.vision_model)
    {
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
    // No provider-native vision — let cli/ui.rs fall back to Gemini if configured
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
                    default_model: Some("MiniMax-M2.7".to_string()),
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
