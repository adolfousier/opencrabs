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

/// One configured model: the provider it belongs to, how that provider is
/// labelled, the model name, and the `provider/model` pair a client sends back.
struct ModelEntry {
    provider: String,
    display: String,
    model: String,
    pair: String,
}

/// Walk the provider registry once and drop anything unusable. Both payload
/// shapes render from this list, so `models` (the field MonoCode reads) and
/// `configOptions` (the field v1 defines) cannot drift apart.
fn collect_models(config: &Config) -> Vec<ModelEntry> {
    let mut entries: Vec<ModelEntry> = Vec::new();
    for (id, display, requires_api_key, cfg) in config.providers.provider_registry() {
        let Some(c) = cfg else { continue };
        if !c.enabled || (requires_api_key && c.api_key.is_none()) {
            continue;
        }
        push_provider_models(&mut entries, id, display, c);
    }
    if let Some((name, cfg)) = config.providers.active_custom() {
        push_provider_models(&mut entries, name, name, cfg);
    }
    entries
}

/// The session's current pair: an explicit pin if there is one, otherwise the
/// first usable configured model.
fn current_pair(entries: &[ModelEntry], current_override: Option<&str>) -> String {
    current_override
        .map(str::to_string)
        .or_else(|| entries.first().map(|e| e.pair.clone()))
        .unwrap_or_default()
}

/// Build the ACP `models` payload: `{ availableModels, currentModelId }`.
///
/// `current_override` is the ACP session's pinned pair (`--model` or a prior
/// model switch); when absent the first usable configured provider's default
/// model is reported, mirroring `resolve_provider_from_config`.
///
/// `models` is not a v1 field: `NewSessionResponse` knows `sessionId`, `modes`,
/// `configOptions` and `_meta` only. It stays because MonoCode renders its
/// picker from it, and dropping it would break a client that already works.
/// Schema-clean clients read `config_options_payload` instead (#1815 F4).
pub fn models_payload(config: &Config, current_override: Option<&str>) -> Value {
    let entries = collect_models(config);
    let available: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "modelId": e.pair,
                "name": format!("{} / {}", e.display, e.model),
            })
        })
        .collect();
    json!({
        "availableModels": available,
        "currentModelId": current_pair(&entries, current_override),
    })
}

/// Build the official v1 `configOptions` payload for the same catalog: a single
/// select option with `category: "model"`, values grouped per provider.
///
/// Emitted alongside `models` because a client generated strictly from the
/// schema drops unknown root fields, which made the model picker invisible
/// outside the MonoCode pairing (#1815 F4). Selection still travels through
/// `_opencrabs/set_model`: the official counterpart `session/set_config_option`
/// is not implemented (F5), so this is the display half of the contract and
/// does not pretend to be the write half.
pub fn config_options_payload(config: &Config, current_override: Option<&str>) -> Value {
    let entries = collect_models(config);
    if entries.is_empty() {
        return json!([]);
    }
    let mut groups: Vec<Value> = Vec::new();
    for e in &entries {
        let option = json!({ "value": e.pair, "name": e.model });
        if let Some(existing) = groups
            .iter_mut()
            .find(|g| g["group"].as_str() == Some(&e.provider))
        {
            existing["options"]
                .as_array_mut()
                .expect("group built with an options array")
                .push(option);
        } else {
            groups.push(json!({
                "group": e.provider,
                "name": e.display,
                "options": [option],
            }));
        }
    }
    json!([{
        "id": "model",
        "name": "Model",
        "description": "Provider and model used for this session",
        "category": "model",
        "type": "select",
        "currentValue": current_pair(&entries, current_override),
        "options": groups,
    }])
}

/// Emit one entry per configured model, falling back to the provider's
/// default model when the runtime list is empty.
fn push_provider_models(
    entries: &mut Vec<ModelEntry>,
    id: &str,
    display: &str,
    cfg: &ProviderConfig,
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
        entries.push(ModelEntry {
            provider: id.to_string(),
            display: display.to_string(),
            model: model.to_string(),
            pair: format!("{id}/{model}"),
        });
    }
}
