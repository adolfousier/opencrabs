//! Shared slash-command handlers for channel platforms (Telegram, Discord, Slack).
//!
//! Each channel handler calls [`handle_command`] before forwarding to the agent.
//! If the message is a known command, the channel renders the response directly.

use uuid::Uuid;

use crate::brain::agent::AgentService;
use crate::db::repository::SessionListOptions;
use crate::services::SessionService;

/// Result of matching a channel message against known commands.
pub enum ChannelCommand {
    /// `/help` — formatted help text
    Help(String),
    /// `/usage` — formatted session/cost stats
    Usage(String),
    /// `/models` — provider picker (step 1: choose provider, step 2: choose model)
    Models(ProvidersResponse),
    /// `/stop` — cancel the running agent task
    Stop,
    /// Not a recognised command — pass through to agent
    NotACommand,
}

/// Data for rendering a provider-picker on the channel platform.
pub struct ProvidersResponse {
    pub current_provider: String,
    pub current_model: String,
    /// Available providers (name, display label) that have API keys configured.
    pub providers: Vec<(String, String)>,
    /// Fallback text when platform buttons are unavailable.
    pub text: String,
}

/// Data for rendering a model-picker after a provider is selected.
pub struct ModelsResponse {
    pub provider_name: String,
    pub current_model: String,
    pub models: Vec<String>,
    /// Fallback text when platform buttons are unavailable.
    pub text: String,
}

/// Check if a message is a known channel command and return the response.
pub async fn handle_command(
    text: &str,
    session_id: Uuid,
    agent: &AgentService,
    session_svc: &SessionService,
) -> ChannelCommand {
    let trimmed = text.trim();
    match trimmed {
        "/help" => ChannelCommand::Help(format_help()),
        "/usage" => ChannelCommand::Usage(format_usage(session_id, agent, session_svc).await),
        "/models" => ChannelCommand::Models(format_providers(agent)),
        "/stop" => ChannelCommand::Stop,
        _ => ChannelCommand::NotACommand,
    }
}

// ── /help ───────────────────────────────────────────────────────────────────

fn format_help() -> String {
    [
        "📖 *Available Commands*",
        "",
        "`/evolve` — Download latest release & restart",
        "`/help`   — Show this message",
        "`/models` — Switch AI model",
        "`/stop`   — Abort current operation",
        "`/usage`  — Session token & cost stats",
        "",
        "🦀 Any other message is sent to OpenCrabs. 🦀",
    ]
    .join("\n")
}

// ── /usage ──────────────────────────────────────────────────────────────────

async fn format_usage(
    session_id: Uuid,
    agent: &AgentService,
    session_svc: &SessionService,
) -> String {
    let mut lines = vec!["📊 *Usage Stats*".to_string(), String::new()];

    // Current session
    let current_model = agent.provider_model();
    match session_svc.get_session(session_id).await {
        Ok(Some(session)) => {
            let name = session.title.as_deref().unwrap_or("Current Session");
            let model = session
                .model
                .as_deref()
                .filter(|m| !m.is_empty())
                .unwrap_or(&current_model);
            let tokens = session.token_count;
            let cost = if session.total_cost > 0.0 {
                session.total_cost
            } else if tokens > 0 {
                estimate_cost(model, tokens as i64).unwrap_or(0.0)
            } else {
                0.0
            };
            lines.push(format!("*Current Session:* {}", name));
            lines.push(format!("  Model: `{}`", model));
            lines.push(format!("  Tokens: {}", format_number(tokens as i64)));
            lines.push(format!("  Cost: ${:.4}", cost));
        }
        _ => {
            lines.push("*Current Session:* (not found)".to_string());
        }
    }

    // All-time stats from usage ledger (survives session deletes)
    lines.push(String::new());
    {
        use crate::db::repository::UsageLedgerRepository;
        let ledger = UsageLedgerRepository::new(session_svc.pool());
        let ledger_stats = ledger.stats_by_model().await.unwrap_or_default();

        let all_tokens: i64 = ledger_stats.iter().map(|s| s.total_tokens).sum();
        let all_cost: f64 = ledger_stats.iter().map(|s| s.total_cost).sum();

        let total_sessions = session_svc
            .list_sessions(SessionListOptions::default())
            .await
            .map(|s| s.len())
            .unwrap_or(0);

        lines.push(format!(
            "*All-Time:* {} sessions, {} tokens, ${:.4}",
            total_sessions,
            format_number(all_tokens),
            all_cost
        ));

        // Top models by cost (already sorted desc from ledger)
        for stats in ledger_stats.iter().take(5) {
            lines.push(format!(
                "  `{}` — {} tokens, ${:.4}",
                stats.model,
                format_number(stats.total_tokens),
                stats.total_cost
            ));
        }
    }

    lines.join("\n")
}

fn estimate_cost(model: &str, token_count: i64) -> Option<f64> {
    crate::pricing::PricingConfig::load().estimate_cost(model, token_count)
}

fn format_number(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

// ── /models ─────────────────────────────────────────────────────────────────

fn format_providers(agent: &AgentService) -> ProvidersResponse {
    let current_provider = agent.provider_name();
    let current_model = agent.provider_model();

    let providers = configured_providers();

    let mut text_lines = vec![
        "🤖 *Switch Provider*".to_string(),
        format!("Current: `{}` / `{}`", current_provider, current_model),
        String::new(),
    ];
    for (name, label) in &providers {
        let marker = if *name == current_provider {
            " ✓"
        } else {
            ""
        };
        text_lines.push(format!("• `{}`{}", label, marker));
    }

    ProvidersResponse {
        current_provider,
        current_model,
        providers,
        text: text_lines.join("\n"),
    }
}

/// List configured providers (those with API keys set).
fn configured_providers() -> Vec<(String, String)> {
    let config = match crate::config::Config::load() {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut result = Vec::new();

    if config
        .providers
        .anthropic
        .as_ref()
        .is_some_and(|p| p.api_key.is_some())
    {
        result.push(("anthropic".to_string(), "Anthropic".to_string()));
    }
    if config
        .providers
        .openai
        .as_ref()
        .is_some_and(|p| p.api_key.is_some())
    {
        result.push(("openai".to_string(), "OpenAI".to_string()));
    }
    if config
        .providers
        .openrouter
        .as_ref()
        .is_some_and(|p| p.api_key.is_some())
    {
        result.push(("openrouter".to_string(), "OpenRouter".to_string()));
    }
    if config
        .providers
        .minimax
        .as_ref()
        .is_some_and(|p| p.api_key.is_some())
    {
        result.push(("minimax".to_string(), "MiniMax".to_string()));
    }
    if config
        .providers
        .gemini
        .as_ref()
        .is_some_and(|p| p.api_key.is_some())
    {
        result.push(("gemini".to_string(), "Gemini".to_string()));
    }
    if let Some(ref customs) = config.providers.custom {
        for (name, cfg) in customs {
            if cfg.api_key.is_some() {
                result.push((format!("custom:{}", name), format!("Custom ({})", name)));
            }
        }
    }

    result
}

/// Fetch models for a specific provider (called from callback handler).
pub async fn models_for_provider(provider_name: &str) -> ModelsResponse {
    let config = match crate::config::Config::load() {
        Ok(c) => c,
        Err(_) => {
            return ModelsResponse {
                provider_name: provider_name.to_string(),
                current_model: String::new(),
                models: vec![],
                text: "Failed to load config.".to_string(),
            };
        }
    };

    // Create a temporary provider to fetch its models
    let provider =
        match crate::brain::provider::factory::create_provider_by_name(&config, provider_name) {
            Ok(p) => p,
            Err(e) => {
                return ModelsResponse {
                    provider_name: provider_name.to_string(),
                    current_model: String::new(),
                    models: vec![],
                    text: format!("Failed to create provider: {}", e),
                };
            }
        };

    let current_model = provider.default_model().to_string();

    // Try live fetch first, fall back to config models, then hardcoded
    let mut models = provider.fetch_models().await;

    // If fetch returned the hardcoded GPT fallback, check config models instead
    if models.first().is_some_and(|m| m.starts_with("gpt-")) && provider.name() != "openai" {
        // Fetch returned the hardcoded fallback — try config models
        let config_models = provider_config_models(&config, provider_name);
        if !config_models.is_empty() {
            models = config_models;
        }
    }

    if models.is_empty() {
        models = provider.supported_models();
    }

    // Ensure current model is in the list
    if !models.contains(&current_model) {
        models.insert(0, current_model.clone());
    }

    let display_name = provider_display_name(provider_name);
    let mut text_lines = vec![
        format!("🤖 *{} Models*", display_name),
        format!("Current: `{}`", current_model),
        String::new(),
    ];
    for (i, m) in models.iter().enumerate() {
        let marker = if *m == current_model { " ✓" } else { "" };
        text_lines.push(format!("{}. `{}`{}", i + 1, m, marker));
    }

    ModelsResponse {
        provider_name: provider_name.to_string(),
        current_model,
        models,
        text: text_lines.join("\n"),
    }
}

/// Get models from the provider's config section (for providers without /models endpoint).
fn provider_config_models(config: &crate::config::Config, name: &str) -> Vec<String> {
    let cfg = match name {
        "anthropic" => config.providers.anthropic.as_ref(),
        "openai" => config.providers.openai.as_ref(),
        "openrouter" => config.providers.openrouter.as_ref(),
        "minimax" => config.providers.minimax.as_ref(),
        "gemini" => config.providers.gemini.as_ref(),
        n if n.starts_with("custom:") => {
            let custom_name = &n["custom:".len()..];
            config
                .providers
                .custom
                .as_ref()
                .and_then(|m| m.get(custom_name))
        }
        _ => None,
    };
    cfg.map(|c| c.models.clone()).unwrap_or_default()
}

fn provider_display_name(name: &str) -> &str {
    match name {
        "anthropic" => "Anthropic",
        "openai" => "OpenAI",
        "openrouter" => "OpenRouter",
        "minimax" => "MiniMax",
        "gemini" => "Gemini",
        n if n.starts_with("custom:") => &n["custom:".len()..],
        other => other,
    }
}

// ── Model switching ─────────────────────────────────────────────────────────

/// Switch the active model within the current provider and persist to config.
pub fn switch_model(agent: &AgentService, model_name: &str) {
    // Detect provider section from provider name
    let provider_name = agent.provider_name();
    let section = match provider_name.to_lowercase().as_str() {
        "anthropic" => "providers.anthropic",
        "openai" => "providers.openai",
        "gemini" | "google" => "providers.gemini",
        "openrouter" => "providers.openrouter",
        "minimax" => "providers.minimax",
        _ => {
            // Custom provider — try to write under providers.custom.<name>
            // Fall back to just logging
            tracing::info!(
                "Channel: model switch to {} (custom provider {})",
                model_name,
                provider_name
            );
            return;
        }
    };

    if let Err(e) = crate::config::Config::write_key(section, "default_model", model_name) {
        tracing::warn!("Failed to persist model to config: {}", e);
    } else {
        tracing::info!(
            "Channel: switched model to {} (provider: {})",
            model_name,
            provider_name
        );
    }

    // Reload provider from config so the change takes effect immediately
    if let Ok(config) = crate::config::Config::load()
        && let Ok(new_provider) = crate::brain::provider::create_provider(&config)
    {
        agent.swap_provider(new_provider);
    }
}
