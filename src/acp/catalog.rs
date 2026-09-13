//! Live model catalog for ACP `session/new` responses.
//!
//! MonoCode (and any other ACP client) renders the picker from
//! `models.availableModels` instead of a static list. The catalog walks the
//! same `provider_registry` the factory and TUI display use, so a provider
//! that is enabled but unusable (keyless keyed section) never appears, and a
//! newly added provider field shows up here without touching this file.
//!
//! `modelId` is the `provider/model` pair the rest of the CLI already
//! understands (`parse_pair`), so `session/set_model` can route a provider
//! switch instead of guessing.

use serde_json::{Value, json};

use crate::config::{Config, types::ProviderConfig};

/// Slash commands for the ACP `available_commands_update` push: the built-in
/// table the TUI autocompletes from, the installed skills, and the user's
/// commands.toml entries. Names are normalised to ACP shape (no leading
/// slash), deduped in declaration order. Channel-only commands are excluded:
/// they dispatch on chat surfaces, not on an editor harness.
pub fn commands_payload() -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |name: &str, description: &str| {
        let name = name.trim().trim_start_matches('/');
        if name.is_empty() || name.contains([' ', '/', '\\']) || !seen.insert(name.to_string()) {
            return;
        }
        out.push(json!({ "name": name, "description": description }));
    };
    for cmd in crate::tui::app::state::SLASH_COMMANDS {
        push(cmd.name, cmd.description);
    }
    for skill in crate::brain::skills::load_all_skills() {
        push(&skill.slash_name, &skill.description);
    }
    let brain_path = crate::brain::BrainLoader::resolve_path();
    for cmd in crate::brain::CommandLoader::from_brain_path(&brain_path).load() {
        push(&cmd.name, &cmd.description);
    }
    out
}

/// Build the ACP `models` payload: `{ availableModels, currentModelId }`.
///
/// `current_override` is the ACP session's pinned pair (`--model` or a prior
/// `session/set_model`); when absent the first usable configured provider's
/// default model is reported, mirroring `resolve_provider_from_config`.
pub fn models_payload(config: &Config, current_override: Option<&str>) -> Value {
    let mut available: Vec<Value> = Vec::new();
    let mut first_pair: Option<String> = None;

    for (id, display, requires_api_key, cfg) in config.providers.provider_registry() {
        let Some(c) = cfg else { continue };
        if !c.enabled || (requires_api_key && c.api_key.is_none()) {
            continue;
        }
        push_provider_models(&mut available, id, display, c, &mut first_pair);
    }
    if let Some((name, cfg)) = config.providers.active_custom() {
        push_provider_models(&mut available, name, name, cfg, &mut first_pair);
    }

    let current = current_override
        .map(str::to_string)
        .or(first_pair)
        .unwrap_or_default();
    json!({
        "availableModels": available,
        "currentModelId": current,
    })
}

/// Emit one entry per configured model, falling back to the provider's
/// default model when the runtime list is empty. `first_pair` records the
/// first emitted pair so the caller can name a current model.
fn push_provider_models(
    available: &mut Vec<Value>,
    id: &str,
    display: &str,
    cfg: &ProviderConfig,
    first_pair: &mut Option<String>,
) {
    let mut models: Vec<&str> = cfg
        .models
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .collect();
    if models.is_empty()
        && let Some(d) = cfg.default_model.as_deref().map(str::trim)
        && !d.is_empty()
    {
        models.push(d);
    }
    for model in models {
        let pair = format!("{id}/{model}");
        if first_pair.is_none() {
            *first_pair = Some(pair.clone());
        }
        available.push(json!({
            "modelId": pair,
            "name": format!("{display} / {model}"),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(toml: &str) -> Config {
        toml::from_str(toml).expect("test config parses")
    }

    #[test]
    fn commands_payload_normalises_and_dedupes() {
        let commands = commands_payload();
        // Built-ins land whatever the host's skills/commands.toml hold.
        assert!(commands.iter().any(|c| c["name"] == "help"));
        for cmd in &commands {
            let name = cmd["name"].as_str().unwrap();
            assert!(!name.is_empty());
            assert!(!name.contains(['/', '\\', ' ']), "bad name: {name}");
        }
        let mut names: Vec<&str> = commands
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate command names");
    }

    #[test]
    fn empty_config_yields_empty_catalog() {
        let payload = models_payload(&Config::default(), None);
        assert_eq!(payload["availableModels"], json!([]));
        assert_eq!(payload["currentModelId"], json!(""));
    }

    #[test]
    fn enabled_keyed_provider_lists_its_models() {
        let cfg = config_with(
            r#"
            [providers.anthropic]
            enabled = true
            api_key = "sk-test"
            default_model = "claude-opus-4-8"
            models = ["claude-opus-4-8", "claude-haiku-4-5"]
            "#,
        );
        let payload = models_payload(&cfg, None);
        let models = payload["availableModels"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["modelId"], json!("anthropic/claude-opus-4-8"));
        assert_eq!(
            payload["currentModelId"],
            json!("anthropic/claude-opus-4-8")
        );
    }

    #[test]
    fn enabled_but_keyless_keyed_provider_is_skipped() {
        let cfg = config_with(
            r#"
            [providers.anthropic]
            enabled = true
            default_model = "claude-opus-4-8"
            "#,
        );
        let payload = models_payload(&cfg, None);
        assert_eq!(payload["availableModels"], json!([]));
    }

    #[test]
    fn empty_models_list_falls_back_to_default_model() {
        let cfg = config_with(
            r#"
            [providers.ollama]
            enabled = true
            default_model = "qwen3:8b"
            "#,
        );
        let payload = models_payload(&cfg, None);
        assert_eq!(
            payload["availableModels"][0]["modelId"],
            json!("ollama/qwen3:8b")
        );
    }

    #[test]
    fn session_override_wins_current_model() {
        let cfg = config_with(
            r#"
            [providers.anthropic]
            enabled = true
            api_key = "sk-test"
            default_model = "claude-opus-4-8"
            "#,
        );
        let payload = models_payload(&cfg, Some("ollama/qwen3:8b"));
        assert_eq!(payload["currentModelId"], json!("ollama/qwen3:8b"));
    }
}
