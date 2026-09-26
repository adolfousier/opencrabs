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
