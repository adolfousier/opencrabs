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
