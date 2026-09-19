use serde::Serialize;

#[derive(Serialize)]
pub struct OnboardingProvider {
    pub id: String,
    pub name: String,
    pub key_label: String,
    pub help_lines: Vec<String>,
    pub needs_key: bool,
}

#[derive(Serialize)]
pub struct HealthCheckResult {
    pub db_ok: bool,
    pub provider_ok: bool,
    pub tools_count: usize,
    pub brain_ok: bool,
    pub errors: Vec<String>,
}

fn provider_specs() -> Vec<OnboardingProvider> {
    vec![
        OnboardingProvider {
            id: "anthropic".to_string(),
            name: "Anthropic".to_string(),
            key_label: "API Key".to_string(),
            help_lines: vec![
                "Get your key at console.anthropic.com".to_string(),
                "Supports Claude Sonnet, Opus, Haiku".to_string(),
            ],
            needs_key: true,
        },
        OnboardingProvider {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            key_label: "API Key".to_string(),
            help_lines: vec![
                "Get your key at platform.openai.com".to_string(),
                "Supports GPT-4, GPT-5, o-series".to_string(),
            ],
            needs_key: true,
        },
        OnboardingProvider {
            id: "openrouter".to_string(),
            name: "OpenRouter".to_string(),
            key_label: "API Key".to_string(),
            help_lines: vec![
                "Get your key at openrouter.ai".to_string(),
                "300+ models via single key".to_string(),
            ],
            needs_key: true,
        },
        OnboardingProvider {
            id: "gemini".to_string(),
            name: "Google Gemini".to_string(),
            key_label: "API Key".to_string(),
            help_lines: vec![
                "Get your key at aistudio.google.com".to_string(),
                "Gemini 2.x family".to_string(),
            ],
            needs_key: true,
        },
        OnboardingProvider {
            id: "ollama".to_string(),
            name: "Ollama (Local)".to_string(),
            key_label: "No key needed".to_string(),
            help_lines: vec![
                "Runs models locally".to_string(),
                "Install ollama first".to_string(),
            ],
            needs_key: false,
        },
        OnboardingProvider {
            id: "claude_cli".to_string(),
            name: "Claude CLI (Max)".to_string(),
            key_label: "No key needed".to_string(),
            help_lines: vec![
                "Requires Claude Max subscription".to_string(),
                "Uses claude CLI in path".to_string(),
            ],
            needs_key: false,
        },
        OnboardingProvider {
            id: "codex_cli".to_string(),
            name: "Codex CLI".to_string(),
            key_label: "No key needed".to_string(),
            help_lines: vec![
                "Requires ChatGPT/Codex subscription".to_string(),
                "Uses codex CLI in path".to_string(),
            ],
            needs_key: false,
        },
        OnboardingProvider {
            id: "codex".to_string(),
            name: "Codex OAuth".to_string(),
            key_label: "OAuth".to_string(),
            help_lines: vec![
                "Native device-code flow".to_string(),
                "No API key needed".to_string(),
            ],
            needs_key: false,
        },
        OnboardingProvider {
            id: "command_code_cli".to_string(),
            name: "Command Code CLI".to_string(),
            key_label: "No key needed".to_string(),
            help_lines: vec!["Uses command-code CLI in path".to_string()],
            needs_key: false,
        },
    ]
}

fn canonical_provider_name(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("anthropic"),
        "openai" => Some("openai"),
        "openrouter" => Some("openrouter"),
        "gemini" => Some("gemini"),
        "ollama" => Some("ollama"),
        "claude_cli" | "claude-cli" => Some("claude_cli"),
        "codex_cli" | "codex-cli" => Some("codex_cli"),
        "codex" => Some("codex"),
        "command_code_cli" | "command-code-cli" => Some("command_code_cli"),
        _ => None,
    }
}

fn provider_section(provider: &str) -> Result<String, String> {
    canonical_provider_name(provider)
        .map(|p| format!("providers.{p}"))
        .ok_or_else(|| format!("Unknown provider: {provider}"))
}

fn key_looks_valid(provider: &str, key: &str) -> bool {
    let key = key.trim();
    if key.is_empty() {
        return false;
    }
    match canonical_provider_name(provider) {
        Some("anthropic") => key.starts_with("sk-ant-"),
        Some("openai") => key.starts_with("sk-") && key.len() >= 20,
        Some("openrouter") => {
            key.starts_with("sk-or-") || (key.starts_with("sk-") && key.len() >= 20)
        }
        Some("gemini") => key.len() >= 20,
        Some(_) => true,
        None => false,
    }
}

fn scrubbed_key(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= 8 {
        return "[redacted]".to_string();
    }
    format!("{}…{}", &trimmed[..4], &trimmed[trimmed.len() - 4..])
}

#[tauri::command]
pub async fn is_first_time_setup() -> Result<bool, String> {
    let home = opencrabs::config::opencrabs_home();
    Ok(!home.join("config.toml").exists())
}

#[tauri::command]
pub async fn get_available_providers() -> Result<Vec<OnboardingProvider>, String> {
    Ok(provider_specs())
}

#[tauri::command]
pub async fn validate_api_key(provider: String, key: String) -> Result<bool, String> {
    Ok(key_looks_valid(&provider, &key))
}

#[tauri::command]
pub async fn save_onboarding_config(
    provider: String,
    api_key: String,
    model: String,
    _workspace_dir: Option<String>,
) -> Result<(), String> {
    let section = provider_section(&provider)?;
    if !api_key.trim().is_empty() {
        if !key_looks_valid(&provider, &api_key) {
            return Err(format!(
                "API key for {} failed validation ({})",
                provider,
                scrubbed_key(&api_key)
            ));
        }
        opencrabs::config::Config::write_keys_key(&section, "api_key", &api_key)
            .map_err(|e| e.to_string())?;
    }
    if !model.trim().is_empty() {
        opencrabs::config::Config::write_key(&section, "default_model", &model)
            .map_err(|e| e.to_string())?;
    }
    opencrabs::config::Config::write_key(&section, "enabled", "true").map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn run_health_check() -> Result<HealthCheckResult, String> {
    let mut errors = Vec::new();

    let db_ok = std::fs::metadata(opencrabs::config::opencrabs_home().join("opencrabs.db")).is_ok();
    if !db_ok {
        errors.push("Database file not found".to_string());
    }

    let brain_ok = opencrabs::config::opencrabs_home().join("SOUL.md").exists();
    if !brain_ok {
        errors.push("No SOUL.md found".to_string());
    }

    let config = opencrabs::config::Config::load().map_err(|e| e.to_string())?;
    let providers = [
        config.providers.anthropic.as_ref(),
        config.providers.openai.as_ref(),
        config.providers.openrouter.as_ref(),
        config.providers.gemini.as_ref(),
        config.providers.ollama.as_ref(),
        config.providers.claude_cli.as_ref(),
        config.providers.codex_cli.as_ref(),
        config.providers.codex.as_ref(),
        config.providers.command_code_cli.as_ref(),
    ];
    let provider_ok = providers.iter().flatten().any(|p| p.enabled);
    if !provider_ok {
        errors.push("No enabled provider found".to_string());
    }

    let tools_count = opencrabs::brain::tools::dynamic::DynamicToolLoader::default_path()
        .map(|path| {
            opencrabs::brain::tools::dynamic::DynamicToolLoader::list_tools_detailed(&path)
                .map(|tools| tools.into_iter().filter(|tool| tool.enabled).count())
                .unwrap_or(0)
        })
        .unwrap_or(0);

    Ok(HealthCheckResult {
        db_ok,
        provider_ok,
        tools_count,
        brain_ok,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_provider_aliases() {
        assert_eq!(canonical_provider_name("claude-cli"), Some("claude_cli"));
        assert_eq!(
            canonical_provider_name("command-code-cli"),
            Some("command_code_cli")
        );
        assert_eq!(canonical_provider_name("mystery"), None);
    }

    #[test]
    fn validates_provider_key_shapes() {
        assert!(key_looks_valid("anthropic", "sk-ant-1234567890"));
        assert!(!key_looks_valid("anthropic", "sk-plain-openai-style"));
        assert!(key_looks_valid("openai", "sk-12345678901234567890"));
        assert!(!key_looks_valid("openai", "short"));
    }

    #[test]
    fn scrubs_keys_without_leaking_middle() {
        let scrubbed = scrubbed_key("sk-ant-abcdefghijklmnopqrstuvwxyz");
        assert!(scrubbed.starts_with("sk-a"));
        assert!(scrubbed.ends_with("wxyz"));
        assert!(scrubbed.contains('…'));
    }
}
