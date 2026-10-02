//! ACP live model catalog and command discovery payloads (#1673): provider
//! registry walk, pair-id normalisation, and the `available_commands_update`
//! push shape.

use crate::acp::catalog::{commands_payload, config_options_payload, models_payload};
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

#[test]
fn config_options_expose_one_model_selector() {
    let cfg = config_with(
        r#"
        [providers.anthropic]
        enabled = true
        api_key = "sk-test"
        default_model = "claude-opus-4-8"
        models = ["claude-opus-4-8", "claude-haiku-4-5"]
        "#,
    );
    let options = config_options_payload(&cfg, None);
    let arr = options.as_array().expect("configOptions is an array");
    // One option only: this is the model picker, not a grab bag of settings.
    assert_eq!(arr.len(), 1);
    let option = &arr[0];
    assert_eq!(option["id"], json!("model"));
    assert_eq!(option["type"], json!("select"));
    // A spec-reserved category, so a client can place it with its other model
    // selectors instead of guessing from the label.
    assert_eq!(option["category"], json!("model"));
    assert_eq!(option["name"], json!("Model"));
    assert_eq!(option["currentValue"], json!("anthropic/claude-opus-4-8"));
    let groups = option["options"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["group"], json!("anthropic"));
    let values: Vec<&str> = groups[0]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["value"].as_str().unwrap())
        .collect();
    assert_eq!(
        values,
        vec!["anthropic/claude-opus-4-8", "anthropic/claude-haiku-4-5"]
    );
}

#[test]
fn config_options_group_per_provider() {
    let cfg = config_with(
        r#"
        [providers.anthropic]
        enabled = true
        api_key = "sk-test"
        models = ["claude-opus-4-8"]

        [providers.openai]
        enabled = true
        api_key = "sk-test"
        models = ["gpt-5-mini"]
        "#,
    );
    let option = &config_options_payload(&cfg, None)[0];
    let groups = option["options"].as_array().unwrap();
    assert_eq!(groups.len(), 2, "one group per provider");
    let names: Vec<&str> = groups
        .iter()
        .map(|g| g["group"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"anthropic"));
    assert!(names.contains(&"openai"));
    for group in groups {
        // Every grouped value has to carry a label, or the picker renders blank.
        assert!(!group["name"].as_str().unwrap().is_empty());
        assert!(!group["options"].as_array().unwrap().is_empty());
    }
}

#[test]
fn config_options_and_models_catalog_never_drift() {
    let cfg = config_with(
        r#"
        [providers.anthropic]
        enabled = true
        api_key = "sk-test"
        models = ["claude-opus-4-8", "claude-haiku-4-5"]

        [providers.ollama]
        enabled = true
        models = ["qwen3:8b"]
        "#,
    );
    let models = models_payload(&cfg, None);
    let model_ids: Vec<&str> = models["availableModels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["modelId"].as_str().unwrap())
        .collect();
    let options = config_options_payload(&cfg, None);
    let mut values: Vec<&str> = Vec::new();
    for group in options[0]["options"].as_array().unwrap() {
        for option in group["options"].as_array().unwrap() {
            values.push(option["value"].as_str().unwrap());
        }
    }
    // Both shapes walk the same registry, so a set difference here means one of
    // them gained a private filter. Compare as sets: grouping reorders.
    let mut a = model_ids.clone();
    let mut b = values.clone();
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b, "configOptions values vs availableModels ids");
    assert_eq!(
        model_ids.len(),
        values.len(),
        "duplicate values between groups"
    );
}

#[test]
fn unusable_providers_are_absent_from_config_options() {
    // Same gate as the models catalog: enabled but keyless never gets listed,
    // so a strict client is never handed a value that would fail when chosen.
    let cfg = config_with(
        r#"
        [providers.anthropic]
        enabled = true
        default_model = "claude-opus-4-8"
        "#,
    );
    assert_eq!(config_options_payload(&cfg, None), json!([]));
    assert_eq!(config_options_payload(&Config::default(), None), json!([]));
}

#[test]
fn config_options_current_value_follows_the_session_pin() {
    let cfg = config_with(
        r#"
        [providers.anthropic]
        enabled = true
        api_key = "sk-test"
        default_model = "claude-opus-4-8"
        "#,
    );
    let option = &config_options_payload(&cfg, Some("ollama/qwen3:8b"))[0];
    assert_eq!(option["currentValue"], json!("ollama/qwen3:8b"));
}
