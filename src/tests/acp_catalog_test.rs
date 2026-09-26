//! ACP live model catalog and command discovery payloads (#1673): provider
//! registry walk, pair-id normalisation, and the `available_commands_update`
//! push shape.

use crate::acp::catalog::{commands_payload, models_payload};
use crate::config::Config;
use serde_json::json;

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
