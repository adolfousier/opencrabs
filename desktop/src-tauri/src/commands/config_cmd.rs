use crate::AppState;
use serde::Serialize;
use tauri::State;

const SAFE_CONFIG_KEYS: &[(&str, &str)] = &[
    ("agent", "approval_policy"),
    ("agent", "default_provider"),
    ("agent", "default_model"),
    ("agent", "silent_compaction"),
    ("agent", "debug_logs"),
    ("channels.telegram", "enabled"),
    ("channels.discord", "enabled"),
    ("channels.whatsapp", "enabled"),
    ("channels.slack", "enabled"),
    ("channels.trello", "enabled"),
    ("providers.anthropic", "default_model"),
    ("providers.openai", "default_model"),
    ("providers.openrouter", "default_model"),
    ("providers.gemini", "default_model"),
    ("providers.ollama", "default_model"),
    ("providers.claude_cli", "default_model"),
    ("providers.codex_cli", "default_model"),
    ("providers.codex", "default_model"),
    ("providers.command_code_cli", "default_model"),
    ("providers.opencode_cli", "default_model"),
    ("providers.opencode", "default_model"),
    ("providers.qwen", "default_model"),
    ("providers.github", "default_model"),
    ("providers.anthropic", "enabled"),
    ("providers.openai", "enabled"),
    ("providers.openrouter", "enabled"),
    ("providers.gemini", "enabled"),
    ("providers.ollama", "enabled"),
    ("providers.claude_cli", "enabled"),
    ("providers.codex_cli", "enabled"),
    ("providers.codex", "enabled"),
    ("providers.command_code_cli", "enabled"),
    ("providers.opencode_cli", "enabled"),
    ("providers.opencode", "enabled"),
    ("providers.qwen", "enabled"),
    ("providers.github", "enabled"),
];

#[derive(Serialize)]
pub struct ConfigInfo {
    pub providers: Vec<ProviderEntry>,
    pub agent_auto_approve: bool,
    pub a2a_enabled: bool,
    pub a2a_port: u16,
}

#[derive(Serialize)]
pub struct ProviderEntry {
    pub name: String,
    pub enabled: bool,
    pub default_model: Option<String>,
    pub models: Vec<String>,
    pub has_api_key: bool,
}

fn provider_entry(name: &str, config: &Option<opencrabs::config::ProviderConfig>) -> ProviderEntry {
    match config {
        Some(c) => ProviderEntry {
            name: name.to_string(),
            enabled: c.enabled,
            default_model: c.default_model.clone(),
            models: c.models.clone(),
            has_api_key: c
                .api_key
                .as_ref()
                .map(|k| !k.trim().is_empty())
                .unwrap_or(false),
        },
        None => ProviderEntry {
            name: name.to_string(),
            enabled: false,
            default_model: None,
            models: vec![],
            has_api_key: false,
        },
    }
}

fn config_to_info(config: &opencrabs::config::Config) -> ConfigInfo {
    let mut providers = Vec::new();
    let p = &config.providers;

    providers.push(provider_entry("anthropic", &p.anthropic));
    providers.push(provider_entry("openai", &p.openai));
    providers.push(provider_entry("openrouter", &p.openrouter));
    providers.push(provider_entry("gemini", &p.gemini));
    providers.push(provider_entry("ollama", &p.ollama));
    providers.push(provider_entry("claude_cli", &p.claude_cli));
    providers.push(provider_entry("codex_cli", &p.codex_cli));
    providers.push(provider_entry("codex", &p.codex));
    providers.push(provider_entry("opencode_cli", &p.opencode_cli));
    providers.push(provider_entry("opencode", &p.opencode));
    providers.push(provider_entry("qwen", &p.qwen));
    providers.push(provider_entry("github", &p.github));
    providers.push(provider_entry("command_code_cli", &p.command_code_cli));

    if let Some(ref custom) = p.custom {
        for (name, cfg) in custom {
            providers.push(provider_entry(
                &format!("custom.{}", name),
                &Some(cfg.clone()),
            ));
        }
    }

    ConfigInfo {
        providers,
        agent_auto_approve: config.agent.approval_policy == "auto-always",
        a2a_enabled: config.a2a.enabled,
        a2a_port: config.a2a.port,
    }
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
        "opencode_cli" | "opencode-cli" => Some("opencode_cli"),
        "opencode" => Some("opencode"),
        "qwen" => Some("qwen"),
        "github" => Some("github"),
        _ => None,
    }
}

fn provider_section(provider: &str) -> Result<String, String> {
    canonical_provider_name(provider)
        .map(|p| format!("providers.{p}"))
        .ok_or_else(|| format!("Unknown provider: {provider}"))
}

fn validate_config_value(section: &str, key: &str, value: &str) -> Result<(), String> {
    match (section, key) {
        (_, "enabled") => match value {
            "true" | "false" => Ok(()),
            _ => Err(format!("Invalid boolean value for [{section}].{key}")),
        },
        ("agent", "approval_policy") => match value {
            "manual" | "on-request" | "auto-always" => Ok(()),
            _ => Err("Invalid approval policy".to_string()),
        },
        (_, "default_model") | (_, "default_provider") => {
            if value.trim().is_empty() {
                Err(format!("[{section}].{key} cannot be empty"))
            } else {
                Ok(())
            }
        }
        (_, "silent_compaction") | (_, "debug_logs") => match value {
            "true" | "false" => Ok(()),
            _ => Err(format!("Invalid boolean value for [{section}].{key}")),
        },
        _ => Ok(()),
    }
}

/// Canonical agent approval policies. Kept in one place so the desktop UI's
/// per-tool approval gesture and the raw config write stay in lock-step and
/// can never disagree about which values are legal.
pub(crate) const APPROVAL_POLICIES: &[&str] = &["manual", "on-request", "auto-always"];

/// Translate a per-tool approve/deny gesture onto the canonical global policy
/// that actually governs the agent. The desktop preview does not yet persist
/// per-tool approval, so a gesture is mapped to the global policy honestly and
/// the originating tool/session are logged for traceability.
pub(crate) fn policy_for_approval(approved: bool, always_approve: bool) -> &'static str {
    match (approved, always_approve) {
        (true, true) => "auto-always",
        (true, false) => "on-request",
        (false, _) => "manual",
    }
}

/// Persist the agent approval policy through the same validated, allowlisted
/// path used by `update_config`, so no desktop command can bypass the guard or
/// write a value the rest of the application would reject. Membership in the
/// canonical set is asserted before the write so the two sources of truth can
/// never disagree.
pub(crate) fn apply_approval_policy(policy: &str) -> Result<(), String> {
    if !APPROVAL_POLICIES.contains(&policy) {
        return Err(format!("Unknown approval policy: {policy}"));
    }
    safe_config_write("agent", "approval_policy", policy)
}

fn safe_config_write(section: &str, key: &str, value: &str) -> Result<(), String> {
    if !SAFE_CONFIG_KEYS
        .iter()
        .any(|(s, k)| *s == section && *k == key)
    {
        return Err(format!("Desktop config edit denied for [{section}].{key}"));
    }
    validate_config_value(section, key, value)?;
    opencrabs::config::Config::write_key(section, key, value).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<ConfigInfo, String> {
    let config = state.config.read().await;
    Ok(config_to_info(&config))
}

#[tauri::command]
pub async fn get_providers(state: State<'_, AppState>) -> Result<Vec<ProviderEntry>, String> {
    let config = state.config.read().await;
    Ok(config_to_info(&config).providers)
}

#[tauri::command]
pub async fn select_model(
    state: State<'_, AppState>,
    provider_name: String,
    model: String,
) -> Result<(), String> {
    let section = provider_section(&provider_name)?;
    safe_config_write(&section, "default_model", &model)?;
    let refreshed = opencrabs::config::Config::load().map_err(|e| e.to_string())?;
    *state.config.write().await = refreshed;
    Ok(())
}

#[tauri::command]
pub async fn update_config(
    state: State<'_, AppState>,
    section: String,
    key: String,
    value: String,
) -> Result<(), String> {
    safe_config_write(&section, &key, &value)?;
    let refreshed = opencrabs::config::Config::load().map_err(|e| e.to_string())?;
    *state.config.write().await = refreshed;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_allowlisted_config_key() {
        let err = safe_config_write("agent", "totally_fake", "true")
            .expect_err("unknown key should fail");
        assert!(err.contains("Desktop config edit denied"));
    }

    #[test]
    fn validates_boolean_fields() {
        let err = validate_config_value("channels.telegram", "enabled", "maybe")
            .expect_err("invalid bool");
        assert!(err.contains("Invalid boolean value"));
        validate_config_value("channels.telegram", "enabled", "true").expect("true is valid");
    }

    #[test]
    fn validates_approval_policy_values() {
        validate_config_value("agent", "approval_policy", "manual").expect("manual valid");
        validate_config_value("agent", "approval_policy", "auto-always").expect("auto valid");
        let err =
            validate_config_value("agent", "approval_policy", "yolo").expect_err("bad policy");
        assert_eq!(err, "Invalid approval policy");
    }

    #[test]
    fn rejects_empty_model_or_provider_defaults() {
        let err = validate_config_value("agent", "default_provider", "   ")
            .expect_err("empty provider invalid");
        assert!(err.contains("cannot be empty"));
        let err = validate_config_value("providers.openai", "default_model", "")
            .expect_err("empty model invalid");
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn maps_provider_sections_consistently() {
        assert_eq!(
            provider_section("claude-cli").unwrap(),
            "providers.claude_cli"
        );
        assert_eq!(
            provider_section("command_code_cli").unwrap(),
            "providers.command_code_cli"
        );
        assert!(provider_section("mystery").is_err());
    }

    #[test]
    fn approval_gestures_map_to_canonical_policies() {
        assert_eq!(policy_for_approval(true, true), "auto-always");
        assert_eq!(policy_for_approval(true, false), "on-request");
        assert_eq!(policy_for_approval(false, false), "manual");
        assert_eq!(policy_for_approval(false, true), "manual");
        // Every mapped value must be one the config validator accepts.
        for policy in [
            policy_for_approval(true, true),
            policy_for_approval(true, false),
            policy_for_approval(false, false),
        ] {
            assert!(APPROVAL_POLICIES.contains(&policy));
            validate_config_value("agent", "approval_policy", policy)
                .unwrap_or_else(|e| panic!("mapped policy {policy} rejected by validator: {e}"));
        }
    }
}
