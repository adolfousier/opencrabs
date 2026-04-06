//! Shared provider registry — single source of truth for all LLM provider metadata.
//!
//! Used by TUI `/models`, `/onboard`, channel `/models` commands, and config layer.
//! Add new providers HERE — everything else derives from this list.

use crate::config::{ProviderConfig, ProviderConfigs};

/// Static metadata for a known (non-custom) LLM provider.
pub struct ProviderMeta {
    /// Canonical identifier used in config keys and protocol (e.g. "anthropic", "claude-cli")
    pub id: &'static str,
    /// Human-readable display name (e.g. "Anthropic", "Claude CLI")
    pub display_name: &'static str,
    /// Config section path (e.g. "providers.anthropic")
    pub config_section: &'static str,
    /// Whether this provider needs an API key (false for CLI providers)
    pub needs_api_key: bool,
}

/// All known providers in alphabetical order. Custom providers are dynamic and not listed here.
pub const KNOWN_PROVIDERS: &[ProviderMeta] = &[
    ProviderMeta {
        id: "anthropic",
        display_name: "Anthropic",
        config_section: "providers.anthropic",
        needs_api_key: true,
    },
    ProviderMeta {
        id: "claude-cli",
        display_name: "Claude CLI",
        config_section: "providers.claude_cli",
        needs_api_key: false,
    },
    ProviderMeta {
        id: "gemini",
        display_name: "Gemini",
        config_section: "providers.gemini",
        needs_api_key: true,
    },
    ProviderMeta {
        id: "github",
        display_name: "GitHub Copilot",
        config_section: "providers.github",
        needs_api_key: true,
    },
    ProviderMeta {
        id: "minimax",
        display_name: "MiniMax",
        config_section: "providers.minimax",
        needs_api_key: true,
    },
    ProviderMeta {
        id: "openai",
        display_name: "OpenAI",
        config_section: "providers.openai",
        needs_api_key: true,
    },
    ProviderMeta {
        id: "opencode-cli",
        display_name: "OpenCode CLI",
        config_section: "providers.opencode_cli",
        needs_api_key: false,
    },
    ProviderMeta {
        id: "openrouter",
        display_name: "OpenRouter",
        config_section: "providers.openrouter",
        needs_api_key: true,
    },
    ProviderMeta {
        id: "qwen-code-cli",
        display_name: "Qwen Code",
        config_section: "providers.qwen_code_cli",
        needs_api_key: false,
    },
    ProviderMeta {
        id: "zhipu",
        display_name: "z.ai GLM",
        config_section: "providers.zhipu",
        needs_api_key: true,
    },
];

/// Look up a known provider by any of its aliases (e.g. "claude_cli", "claude-cli").
pub fn find_provider_meta(name: &str) -> Option<&'static ProviderMeta> {
    let n = name.trim().to_lowercase();
    KNOWN_PROVIDERS.iter().find(|p| {
        p.id == n
            || p.display_name.to_lowercase() == n
            || match p.id {
                "github" => n == "github copilot",
                "gemini" => n == "google" || n == "google gemini",
                "zhipu" => n == "z.ai glm",
                "claude-cli" => n == "claude_cli",
                "opencode-cli" => n == "opencode_cli" || n == "opencode",
                "qwen-code-cli" => {
                    n == "qwen_code_cli" || n == "qwen_code" || n == "qwen-code" || n == "qwen"
                }
                _ => false,
            }
    })
}

/// Canonical provider id for any alias. Returns the alias itself if unknown.
pub fn normalize_provider_name(name: &str) -> String {
    if let Some(meta) = find_provider_meta(name) {
        return meta.id.to_string();
    }
    let lowered = name.trim().to_lowercase();
    if lowered.starts_with("custom:") {
        return lowered;
    }
    if let Some(inner) = lowered
        .strip_prefix("custom(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return format!("custom:{}", inner);
    }
    lowered
}

/// Display name for any provider id (known or custom).
pub fn display_name(name: &str) -> &str {
    if let Some(meta) = find_provider_meta(name) {
        return meta.display_name;
    }
    if let Some(custom_name) = name.strip_prefix("custom:") {
        return custom_name;
    }
    name
}

/// Config section path for any provider id (known or custom).
pub fn config_section(name: &str) -> Option<String> {
    if let Some(meta) = find_provider_meta(name) {
        return Some(meta.config_section.to_string());
    }
    let custom_name = name.strip_prefix("custom:").or_else(|| {
        name.strip_prefix("custom(")
            .and_then(|s| s.strip_suffix(')'))
    })?;
    Some(format!("providers.custom.{}", custom_name))
}

/// Get the ProviderConfig for a given provider id from ProviderConfigs.
pub fn config_for<'a>(providers: &'a ProviderConfigs, name: &str) -> Option<&'a ProviderConfig> {
    let meta = find_provider_meta(name);
    match meta.map(|m| m.id) {
        Some("anthropic") => providers.anthropic.as_ref(),
        Some("openai") => providers.openai.as_ref(),
        Some("github") => providers.github.as_ref(),
        Some("gemini") => providers.gemini.as_ref(),
        Some("openrouter") => providers.openrouter.as_ref(),
        Some("minimax") => providers.minimax.as_ref(),
        Some("zhipu") => providers.zhipu.as_ref(),
        Some("claude-cli") => providers.claude_cli.as_ref(),
        Some("opencode-cli") => providers.opencode_cli.as_ref(),
        Some("qwen-code-cli") => providers.qwen_code_cli.as_ref(),
        _ => {
            let custom_name = name.strip_prefix("custom:")?;
            providers.custom.as_ref()?.get(custom_name)
        }
    }
}

/// List all configured providers (have API key or are enabled CLI providers).
/// Returns `(provider_id, display_name)` pairs.
pub fn configured_providers(providers: &ProviderConfigs) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for meta in KNOWN_PROVIDERS {
        let cfg = config_for(providers, meta.id);
        let is_configured = if meta.needs_api_key {
            cfg.is_some_and(|c| c.api_key.is_some())
        } else {
            // CLI providers (no API key needed) — always show them.
            // If the binary isn't installed, the error surfaces when the user selects it.
            // This matches TUI behaviour where CLI providers are always listed.
            true
        };
        if is_configured {
            result.push((meta.id.to_string(), meta.display_name.to_string()));
        }
    }
    if let Some(ref customs) = providers.custom {
        for (name, cfg) in customs {
            if cfg.api_key.is_some() {
                result.push((format!("custom:{}", name), format!("Custom ({})", name)));
            }
        }
    }
    result
}

/// Find the TUI PROVIDERS index for a provider name/alias.
/// Returns None for custom providers (those map to 9+ dynamically).
pub fn tui_index_for_id(name: &str) -> Option<usize> {
    use crate::tui::onboarding::PROVIDERS;
    let normalized = normalize_provider_name(name);
    PROVIDERS
        .iter()
        .position(|p| !p.id.is_empty() && p.id == normalized)
}

/// All config sections (for toggling enabled flags during model switch).
pub fn all_config_sections(providers: &ProviderConfigs) -> Vec<String> {
    let mut sections: Vec<String> = KNOWN_PROVIDERS
        .iter()
        .map(|p| p.config_section.to_string())
        .collect();
    if let Some(ref customs) = providers.custom {
        for name in customs.keys() {
            sections.push(format!("providers.custom.{}", name));
        }
    }
    sections
}
