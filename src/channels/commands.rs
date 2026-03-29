//! Shared slash-command handlers for channel platforms (Telegram, Discord, Slack).
//!
//! Each channel handler calls [`handle_command`] before forwarding to the agent.
//! If the message is a known command, the channel renders the response directly.

use uuid::Uuid;

use crate::brain::agent::AgentService;
use crate::config::Config;
use crate::db::repository::SessionListOptions;
use crate::services::SessionService;

/// Sync the channel agent's provider with the current config.
///
/// Each channel has its own `AgentService` instance, so when the TUI (or another
/// channel) switches models, this agent won't know until we check. Call this at
/// the top of every message handler so the agent always uses the live provider.
pub fn sync_provider_from_config(agent: &AgentService) {
    let config = match Config::load() {
        Ok(c) => c,
        Err(_) => return,
    };

    let (cfg_provider, cfg_model) = config.providers.active_provider_and_model();
    let agent_provider = agent.provider_name();
    let agent_model = agent.provider_model();

    let cfg_provider_norm = normalize_provider_name(&cfg_provider);
    let agent_provider_norm = normalize_provider_name(&agent_provider);
    let same_provider = provider_names_match(&cfg_provider_norm, &agent_provider_norm);

    // Only swap if provider/model changed after normalizing aliases
    if !same_provider || cfg_model != agent_model {
        match crate::brain::provider::create_provider(&config) {
            Ok(new_provider) => {
                tracing::info!(
                    "Channel agent synced: {} ({}) / {} → {} ({}) / {}",
                    agent_provider,
                    agent_provider_norm,
                    agent_model,
                    cfg_provider,
                    cfg_provider_norm,
                    cfg_model,
                );
                agent.swap_provider(new_provider);
            }
            Err(e) => {
                tracing::warn!("Failed to sync channel agent provider: {}", e);
            }
        }
    }
}

/// Normalize provider names/aliases to stable IDs used by config.
fn normalize_provider_name(name: &str) -> String {
    crate::utils::providers::normalize_provider_name(name)
}

/// Compare normalized provider names, handling custom runtime names (`deepseek`).
fn provider_names_match(config_provider: &str, runtime_provider: &str) -> bool {
    config_provider == runtime_provider
        || config_provider
            .strip_prefix("custom:")
            .is_some_and(|name| name == runtime_provider)
}

/// Result of matching a channel message against known commands.
pub enum ChannelCommand {
    /// `/help` — formatted help text
    Help(String),
    /// `/usage` — formatted session/cost stats
    Usage(String),
    /// `/models` — provider picker (step 1: choose provider, step 2: choose model)
    Models(ProvidersResponse),
    /// `/new` — create a new session and switch to it
    NewSession,
    /// `/sessions` — list recent sessions to switch between
    Sessions(SessionsResponse),
    /// `/stop` — cancel the running agent task
    Stop,
    /// `/compact` — trigger context compaction via the agent
    Compact,
    /// `/doctor` — health check (no LLM needed)
    Doctor,
    /// `/evolve` — check for updates and install directly (no LLM needed)
    Evolve,
    /// User-defined command with action "prompt" — forward prompt text to the agent
    UserPrompt(String),
    /// User-defined command with action "system" — display text directly
    UserSystem(String),
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

/// Data for rendering a session-picker on the channel platform.
pub struct SessionsResponse {
    pub current_session_id: Uuid,
    /// (session_id, display_label)
    pub sessions: Vec<(Uuid, String)>,
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
    /// When true, the provider has too many models for inline buttons (OpenRouter, custom).
    /// Channels should switch to default immediately and let the agent handle follow-up.
    pub agent_handled: bool,
}

/// Check if a message is a known channel command and return the response.
/// Commands that produce output are persisted to session history so they
/// appear in TUI and give the agent context about what happened.
pub async fn handle_command(
    text: &str,
    session_id: Uuid,
    agent: &AgentService,
    session_svc: &SessionService,
) -> ChannelCommand {
    let trimmed = text.trim();
    let result = match trimmed {
        "/compact" => ChannelCommand::Compact,
        "/doctor" => ChannelCommand::Doctor,
        "/evolve" => ChannelCommand::Evolve,
        "/help" => ChannelCommand::Help(format_help()),
        "/models" => ChannelCommand::Models(format_providers(agent)),
        "/new" => ChannelCommand::NewSession,
        "/sessions" => ChannelCommand::Sessions(format_sessions(session_id, session_svc).await),
        "/stop" => ChannelCommand::Stop,
        "/usage" => ChannelCommand::Usage(format_usage(session_id, agent, session_svc).await),
        _ if trimmed.starts_with('/') => match_user_command(trimmed),
        _ => ChannelCommand::NotACommand,
    };

    // Persist command + response to session history
    let response_text = match &result {
        ChannelCommand::Help(body) | ChannelCommand::Usage(body) => Some(body.clone()),
        ChannelCommand::Models(resp) => Some(resp.text.clone()),
        ChannelCommand::Sessions(resp) => Some(resp.text.clone()),
        ChannelCommand::NewSession => Some("New session started.".to_string()),
        ChannelCommand::Stop => Some("Operation stopped.".to_string()),
        ChannelCommand::UserSystem(body) => Some(body.clone()),
        ChannelCommand::Doctor => Some("Running health check...".to_string()),
        ChannelCommand::Evolve => Some("Checking for updates...".to_string()),
        ChannelCommand::Compact | ChannelCommand::UserPrompt(_) | ChannelCommand::NotACommand => {
            None
        }
    };

    if let Some(response) = response_text {
        persist_command_to_history(agent, session_id, trimmed, &response).await;
    }

    result
}

/// Save the user command and bot response to session message history,
/// then notify TUI so it refreshes live.
async fn persist_command_to_history(
    agent: &AgentService,
    session_id: Uuid,
    command: &str,
    response: &str,
) {
    let msg_svc = crate::services::MessageService::new(agent.context().clone());
    if let Err(e) = msg_svc
        .create_message(session_id, "user".to_string(), command.to_string())
        .await
    {
        tracing::warn!("Failed to persist channel command to history: {}", e);
    }
    if let Err(e) = msg_svc
        .create_message(session_id, "assistant".to_string(), response.to_string())
        .await
    {
        tracing::warn!(
            "Failed to persist channel command response to history: {}",
            e
        );
    }
    // Notify TUI to reload session messages (same mechanism as agent responses)
    if let Some(tx) = agent.session_updated_tx() {
        let _ = tx.send(session_id);
    }
}

// ── User-defined commands ───────────────────────────────────────────────────

fn match_user_command(text: &str) -> ChannelCommand {
    let brain_path = crate::brain::BrainLoader::resolve_path();
    let loader = crate::brain::CommandLoader::from_brain_path(&brain_path);
    let commands = loader.load();
    match_user_command_inner(text, &commands)
}

fn match_user_command_inner(
    text: &str,
    commands: &[crate::brain::commands::UserCommand],
) -> ChannelCommand {
    // Split "/command args" into command name and optional args
    let (cmd_name, args) = text
        .split_once(' ')
        .map(|(c, a)| (c, a.trim()))
        .unwrap_or((text, ""));

    if let Some(cmd) = commands.iter().find(|c| c.name == cmd_name) {
        let prompt = if args.is_empty() {
            cmd.prompt.clone()
        } else {
            format!("{} {}", cmd.prompt, args)
        };
        match cmd.action.as_str() {
            "system" => ChannelCommand::UserSystem(prompt),
            _ => ChannelCommand::UserPrompt(prompt),
        }
    } else {
        ChannelCommand::NotACommand
    }
}

// ── /help ───────────────────────────────────────────────────────────────────

fn format_help() -> String {
    let mut lines = vec![
        "📖 *Available Commands*".to_string(),
        String::new(),
        "`/compact`  — Compact context (summarize & trim)".to_string(),
        "`/evolve`   — Download latest release & restart".to_string(),
        "`/help`     — Show this message".to_string(),
        "`/models`   — Switch AI model".to_string(),
        "`/new`      — Start a new session".to_string(),
        "`/sessions` — Switch between sessions".to_string(),
        "`/stop`     — Abort current operation".to_string(),
        "`/usage`    — Session token & cost stats".to_string(),
    ];

    // Append user-defined commands from commands.toml
    let brain_path = crate::brain::BrainLoader::resolve_path();
    let loader = crate::brain::CommandLoader::from_brain_path(&brain_path);
    let mut user_cmds = loader.load();
    if !user_cmds.is_empty() {
        user_cmds.sort_by(|a, b| a.name.cmp(&b.name));
        lines.push(String::new());
        lines.push("📌 *Custom Commands*".to_string());
        for cmd in &user_cmds {
            lines.push(format!("`{}`  — {}", cmd.name, cmd.description));
        }
    }

    lines.push(String::new());
    lines.push("🦀 Any other message is sent to OpenCrabs. 🦀".to_string());
    lines.join("\n")
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

// ── /sessions ──────────────────────────────────────────────────────────────

async fn format_sessions(
    current_session_id: Uuid,
    session_svc: &SessionService,
) -> SessionsResponse {
    let sessions = session_svc
        .list_sessions(SessionListOptions {
            include_archived: false,
            limit: Some(10),
            offset: 0,
        })
        .await
        .unwrap_or_default();

    let mut text_lines = vec!["📂 *Sessions*".to_string(), String::new()];
    let mut items = Vec::new();

    for s in &sessions {
        let title = s.title.as_deref().unwrap_or("Untitled");
        let marker = if s.id == current_session_id {
            " ✓"
        } else {
            ""
        };
        let date = s.updated_at.format("%b %d %H:%M");
        let label = format!("{} ({})", title, date);
        text_lines.push(format!("• `{}`{}", label, marker));
        items.push((s.id, label));
    }

    if sessions.is_empty() {
        text_lines.push("No sessions found.".to_string());
    }

    SessionsResponse {
        current_session_id,
        sessions: items,
        text: text_lines.join("\n"),
    }
}

// ── /models ─────────────────────────────────────────────────────────────────

fn format_providers(agent: &AgentService) -> ProvidersResponse {
    // Read current provider/model from config (not the channel agent, which may be stale)
    let (current_provider, current_model) = match crate::config::Config::load() {
        Ok(cfg) => {
            let (prov, model) = cfg.providers.active_provider_and_model();
            (prov, model)
        }
        Err(_) => (agent.provider_name(), agent.provider_model()),
    };

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

/// List configured providers (those with API keys set or enabled CLI providers).
fn configured_providers() -> Vec<(String, String)> {
    let config = match crate::config::Config::load() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    crate::utils::providers::configured_providers(&config.providers)
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
                agent_handled: false,
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
                    agent_handled: false,
                };
            }
        };

    let current_model = provider.default_model().to_string();

    // OpenRouter (300+ models) and custom providers skip live fetch on channels.
    // Switch to default immediately; the agent handles follow-up via config_manager tool.
    if provider_name == "openrouter" || provider_name.starts_with("custom:") {
        return ModelsResponse {
            provider_name: provider_name.to_string(),
            current_model,
            models: vec![],
            text: String::new(),
            agent_handled: true,
        };
    }

    // Standard providers: config models first (instant), then live fetch with timeout.
    let config_models = provider_config_models(&config, provider_name);
    let mut models = if !config_models.is_empty() {
        config_models
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(10), provider.fetch_models())
            .await
        {
            Ok(fetched) if !fetched.is_empty() => fetched,
            Ok(_) => vec![current_model.clone()],
            Err(_) => {
                tracing::warn!("fetch_models timed out for '{}'", provider_name);
                vec![current_model.clone()]
            }
        }
    };

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
        agent_handled: false,
    }
}

/// Get models from the provider's config section (for providers without /models endpoint).
fn provider_config_models(config: &crate::config::Config, name: &str) -> Vec<String> {
    crate::utils::providers::config_for(&config.providers, name)
        .map(|c| c.models.clone())
        .unwrap_or_default()
}

pub fn provider_display_name(name: &str) -> &str {
    crate::utils::providers::display_name(name)
}

// ── Model switching ─────────────────────────────────────────────────────────

/// Switch the active model within the current provider and persist to config.
/// Saves a `[Model changed to ...]` message to the session history so the agent
/// is aware of the switch — matching TUI behaviour.
/// Returns an error message on failure so channels can report it to the user.
pub async fn switch_model(
    agent: &AgentService,
    model_name: &str,
    session_id: Option<uuid::Uuid>,
) -> Result<String, String> {
    let provider_name = agent.provider_name();
    let section = provider_section(&provider_name)
        .ok_or_else(|| format!("Unknown provider '{}' — cannot switch model", provider_name))?;

    // Toggle enabled flags — disable all others, enable selected (same as TUI)
    let config =
        crate::config::Config::load().map_err(|e| format!("Failed to load config: {}", e))?;
    for s in crate::utils::providers::all_config_sections(&config.providers) {
        if s != section {
            let _ = crate::config::Config::write_key(&s, "enabled", "false");
        }
    }
    let _ = crate::config::Config::write_key(&section, "enabled", "true");

    crate::config::Config::write_key(&section, "default_model", model_name).map_err(|e| {
        tracing::warn!("Failed to persist model to config: {}", e);
        format!("Failed to save model: {}", e)
    })?;

    tracing::info!(
        "Channel: switched model to {} (provider: {})",
        model_name,
        provider_name
    );

    // Reload provider from config so the change takes effect immediately
    let config = crate::config::Config::load().map_err(|e| {
        tracing::warn!("Failed to reload config after model switch: {}", e);
        format!("Model saved but failed to reload config: {}", e)
    })?;
    let new_provider = crate::brain::provider::create_provider(&config).map_err(|e| {
        tracing::warn!("Failed to create provider after model switch: {}", e);
        format!("Model saved but failed to reload provider: {}", e)
    })?;
    let display_name = provider_display_name(&provider_name);
    agent.swap_provider(new_provider);

    let change_msg = format!(
        "[Model changed to {} (provider: {})]",
        model_name, display_name
    );

    // Persist to session history so the agent knows about the switch
    if let Some(sid) = session_id {
        let msg_svc = crate::services::MessageService::new(agent.context().clone());
        if let Err(e) = msg_svc
            .create_message(sid, "user".to_string(), change_msg.clone())
            .await
        {
            tracing::warn!("Failed to persist model-change message: {}", e);
        }
    }

    Ok(change_msg)
}

/// Run evolve directly (no LLM needed). Returns a user-facing status message.
pub async fn run_evolve() -> String {
    use crate::brain::tools::{evolve::EvolveTool, Tool, ToolExecutionContext};

    let ctx = ToolExecutionContext::new(uuid::Uuid::nil());
    let tool = EvolveTool::new(None);
    match tool
        .execute(serde_json::json!({"check_only": false}), &ctx)
        .await
    {
        Ok(result) => result.output,
        Err(e) => format!("Evolve failed: {}", e),
    }
}

/// Run doctor health check directly (no LLM needed). Returns a user-facing status message.
pub fn run_doctor() -> String {
    use crate::brain::tools::slash_command::SlashCommandTool;

    // Reuse the slash command tool's doctor logic
    SlashCommandTool::doctor_text()
}

/// Try to execute a command that returns a simple text response (no platform-specific UI).
/// Returns `Some(text)` for commands handled here, `None` for commands that need
/// platform-specific rendering (Models, Sessions, NewSession) or agent passthrough.
/// Channels call this first — if it returns Some, send the text and return.
pub async fn try_execute_text_command(cmd: &ChannelCommand) -> Option<String> {
    match cmd {
        ChannelCommand::Help(body) | ChannelCommand::Usage(body) | ChannelCommand::UserSystem(body) => {
            Some(body.clone())
        }
        ChannelCommand::Doctor => Some(run_doctor()),
        ChannelCommand::Evolve => Some(run_evolve().await),
        _ => None,
    }
}

/// Map a provider name to its config section key.
pub(crate) fn provider_section(provider_name: &str) -> Option<String> {
    crate::utils::providers::config_section(provider_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::commands::UserCommand;

    // ── format_number ──────────────────────────────────────────────────────

    #[test]
    fn format_number_small() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(1), "1");
        assert_eq!(format_number(999), "999");
    }

    #[test]
    fn format_number_thousands() {
        assert_eq!(format_number(1_000), "1.0K");
        assert_eq!(format_number(1_500), "1.5K");
        assert_eq!(format_number(999_999), "1000.0K");
    }

    #[test]
    fn format_number_millions() {
        assert_eq!(format_number(1_000_000), "1.0M");
        assert_eq!(format_number(2_500_000), "2.5M");
        assert_eq!(format_number(123_456_789), "123.5M");
    }

    // ── format_help ────────────────────────────────────────────────────────

    #[test]
    fn format_help_contains_all_commands() {
        let help = format_help();
        for cmd in [
            "/evolve",
            "/help",
            "/models",
            "/new",
            "/sessions",
            "/stop",
            "/usage",
        ] {
            assert!(help.contains(cmd), "help text missing {}", cmd);
        }
    }

    #[test]
    fn format_help_is_alphabetical() {
        let help = format_help();
        // Only check built-in commands (before "Custom Commands" section)
        let builtin_section = help.split("Custom Commands").next().unwrap_or(&help);
        let commands: Vec<&str> = builtin_section
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim().strip_prefix('`')?;
                let cmd = trimmed.split('`').next()?;
                if cmd.starts_with('/') {
                    Some(cmd.split_whitespace().next().unwrap_or(cmd))
                } else {
                    None
                }
            })
            .collect();
        let mut sorted = commands.clone();
        sorted.sort();
        assert_eq!(
            commands, sorted,
            "built-in help commands are not alphabetical"
        );
    }

    // ── provider_display_name ──────────────────────────────────────────────

    #[test]
    fn provider_display_name_known() {
        assert_eq!(provider_display_name("anthropic"), "Anthropic");
        assert_eq!(provider_display_name("openai"), "OpenAI");
        assert_eq!(provider_display_name("github"), "GitHub Copilot");
        assert_eq!(provider_display_name("openrouter"), "OpenRouter");
        assert_eq!(provider_display_name("minimax"), "MiniMax");
        assert_eq!(provider_display_name("gemini"), "Gemini");
    }

    #[test]
    fn provider_aliases_normalize_and_match() {
        assert_eq!(normalize_provider_name("GitHub Copilot"), "github");
        assert_eq!(normalize_provider_name("Google Gemini"), "gemini");
        assert_eq!(
            normalize_provider_name("custom(DeepSeek)"),
            "custom:deepseek"
        );
        assert!(provider_names_match("custom:deepseek", "deepseek"));
    }

    #[test]
    fn provider_display_name_custom() {
        assert_eq!(provider_display_name("custom:deepseek"), "deepseek");
        assert_eq!(provider_display_name("custom:local-llm"), "local-llm");
    }

    #[test]
    fn provider_display_name_unknown() {
        assert_eq!(provider_display_name("mystery"), "mystery");
    }

    // ── match_user_command_inner ────────────────────────────────────────────

    fn make_cmd(name: &str, action: &str, prompt: &str) -> UserCommand {
        UserCommand {
            name: name.to_string(),
            description: String::new(),
            action: action.to_string(),
            prompt: prompt.to_string(),
        }
    }

    #[test]
    fn user_command_prompt_no_args() {
        let cmds = vec![make_cmd(
            "/credits",
            "prompt",
            "Check my OpenRouter credits",
        )];
        match match_user_command_inner("/credits", &cmds) {
            ChannelCommand::UserPrompt(p) => {
                assert_eq!(p, "Check my OpenRouter credits");
            }
            other => panic!("expected UserPrompt, got {:?}", variant_name(&other)),
        }
    }

    #[test]
    fn user_command_prompt_with_args() {
        let cmds = vec![make_cmd("/deploy", "prompt", "Deploy the service")];
        match match_user_command_inner("/deploy staging --dry-run", &cmds) {
            ChannelCommand::UserPrompt(p) => {
                assert_eq!(p, "Deploy the service staging --dry-run");
            }
            other => panic!("expected UserPrompt, got {:?}", variant_name(&other)),
        }
    }

    #[test]
    fn user_command_system_action() {
        let cmds = vec![make_cmd("/info", "system", "OpenCrabs v0.2")];
        match match_user_command_inner("/info", &cmds) {
            ChannelCommand::UserSystem(t) => assert_eq!(t, "OpenCrabs v0.2"),
            other => panic!("expected UserSystem, got {:?}", variant_name(&other)),
        }
    }

    #[test]
    fn user_command_unknown_falls_through() {
        let cmds = vec![make_cmd("/credits", "prompt", "Check credits")];
        assert!(matches!(
            match_user_command_inner("/unknown", &cmds),
            ChannelCommand::NotACommand
        ));
    }

    #[test]
    fn user_command_empty_list() {
        assert!(matches!(
            match_user_command_inner("/anything", &[]),
            ChannelCommand::NotACommand
        ));
    }

    #[test]
    fn user_command_default_action_is_prompt() {
        let cmds = vec![make_cmd("/test", "whatever", "test prompt")];
        assert!(matches!(
            match_user_command_inner("/test", &cmds),
            ChannelCommand::UserPrompt(_)
        ));
    }

    /// Helper to name variants for panic messages (ChannelCommand has no Debug).
    fn variant_name(cmd: &ChannelCommand) -> &'static str {
        match cmd {
            ChannelCommand::Compact => "Compact",
            ChannelCommand::Help(_) => "Help",
            ChannelCommand::Usage(_) => "Usage",
            ChannelCommand::Models(_) => "Models",
            ChannelCommand::NewSession => "NewSession",
            ChannelCommand::Sessions(_) => "Sessions",
            ChannelCommand::Stop => "Stop",
            ChannelCommand::UserPrompt(_) => "UserPrompt",
            ChannelCommand::UserSystem(_) => "UserSystem",
            ChannelCommand::Doctor => "Doctor",
            ChannelCommand::Evolve => "Evolve",
            ChannelCommand::NotACommand => "NotACommand",
        }
    }
}
