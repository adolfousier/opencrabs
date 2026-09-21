//! Image-generation provider-chain tests (#1672).
//!
//! Generation candidate resolution must mirror the vision contract
//! (#1318): session provider first, then the ordered
//! `[providers.fallback] generation` chain, endpoints never guessed,
//! and the global Gemini `[image.generation]` leg strictly last and
//! the only one vetoed by `image.generation.enabled`.

use std::collections::BTreeMap;

use crate::brain::provider::factory::{
    any_provider_generation, effective_generation_model, generation_candidates_for,
};
use crate::brain::tools::Tool;
use crate::brain::tools::generate_image::GenerateImageTool;
use crate::config::{
    Config, FallbackProviderConfig, ImageConfig, ImageGenerationConfig, ProviderConfig,
    ProviderConfigs,
};

fn qwen_custom() -> ProviderConfig {
    ProviderConfig {
        enabled: true,
        api_key: Some("dashscope-key".into()),
        base_url: Some("https://dashscope.aliyuncs.com/compatible-mode/v1".into()),
        default_model: Some("qwen3-coder".into()),
        generation_model: Some("qwen-image-2.0".into()),
        ..Default::default()
    }
}

fn openai_builtin() -> ProviderConfig {
    ProviderConfig {
        enabled: true,
        api_key: Some("oa-key".into()),
        default_model: Some("gpt-5".into()),
        generation_model: Some("dall-e-3".into()),
        ..Default::default()
    }
}

fn config_with(customs: BTreeMap<String, ProviderConfig>, chain: Vec<String>) -> Config {
    Config {
        providers: ProviderConfigs {
            custom: if customs.is_empty() {
                None
            } else {
                Some(customs)
            },
            fallback: Some(FallbackProviderConfig {
                enabled: true,
                generation: chain,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn models(cands: &[(String, String, String)]) -> Vec<String> {
    cands.iter().map(|(_, _, m)| m.clone()).collect()
}

#[test]
fn session_provider_leads_chain_and_dedups() {
    let mut customs = BTreeMap::new();
    customs.insert("qwen-image".to_string(), qwen_custom());
    let mut config = config_with(customs, vec!["custom:qwen-image".into(), "openai".into()]);
    config.providers.openai = Some(openai_builtin());

    let cands = generation_candidates_for(&config, Some("custom:qwen-image"));
    assert_eq!(models(&cands), ["qwen-image-2.0", "dall-e-3"]);
    // Qwen keeps position 1 despite also being the chain head — dedup
    // keeps the FIRST occurrence (#1318 lesson, inverted to the good path).
    assert_eq!(
        cands[0].1,
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    );
}

#[test]
fn custom_without_base_url_is_skipped_never_guessed() {
    let mut customs = BTreeMap::new();
    let mut cfg = qwen_custom();
    cfg.base_url = None;
    customs.insert("qwen-image".to_string(), cfg);
    let config = config_with(customs, vec!["custom:qwen-image".into()]);

    assert!(generation_candidates_for(&config, Some("custom:qwen-image")).is_empty());
    assert!(any_provider_generation(&config).is_empty());
}

#[test]
fn builtin_endpoint_derived_key_required_unless_local() {
    // Builtin with key: default OpenAI endpoint derived.
    let mut config = config_with(BTreeMap::new(), vec!["openai".into()]);
    config.providers.openai = Some(openai_builtin());
    let cands = generation_candidates_for(&config, None);
    assert_eq!(models(&cands), ["dall-e-3"]);
    assert_eq!(cands[0].1, "https://api.openai.com/v1");

    // Keyless non-local builtin: skipped (would 401 on every roll).
    let mut no_key = openai_builtin();
    no_key.api_key = None;
    let mut config = config_with(BTreeMap::new(), vec!["openai".into()]);
    config.providers.openai = Some(no_key);
    assert!(generation_candidates_for(&config, None).is_empty());

    // Keyless LOCAL builtin: allowed (Ollama diffusion needs no key).
    let mut ollama = ProviderConfig {
        enabled: true,
        generation_model: Some("sdxl".into()),
        ..Default::default()
    };
    ollama.api_key = None;
    let mut config = config_with(BTreeMap::new(), vec!["ollama".into()]);
    config.providers.ollama = Some(ollama);
    let cands = generation_candidates_for(&config, None);
    assert_eq!(models(&cands), ["sdxl"]);
    assert_eq!(cands[0].1, "http://localhost:11434/v1");
    assert_eq!(cands[0].0, "");
}

#[test]
fn chain_entry_resolves_even_when_not_the_active_provider() {
    // Issue #1672 gap 1: generation used to be reachable only through the
    // ACTIVE chat provider. A chain entry must resolve independently.
    let mut config = config_with(BTreeMap::new(), vec!["openai".into()]);
    config.providers.openai = Some(openai_builtin());
    // Session rides a vision-less, generation-less provider.
    let cands = generation_candidates_for(&config, Some("anthropic"));
    assert_eq!(models(&cands), ["dall-e-3"]);
    assert_eq!(effective_generation_model(&config), "dall-e-3");
}

#[test]
fn gemini_override_routes_the_gemini_wire_others_openai_wire() {
    let mut config = config_with(BTreeMap::new(), vec![]);
    config.providers.gemini = Some(ProviderConfig {
        enabled: true,
        api_key: Some("goog-key".into()),
        generation_model: Some("imagen-4.0-generate-001".into()),
        ..Default::default()
    });
    let cands = GenerateImageTool::plan_candidates(&config, Some("gemini"));
    assert_eq!(cands[0].backend_kind(), "gemini");

    let mut config = config_with(BTreeMap::new(), vec![]);
    config.providers.openrouter = Some(ProviderConfig {
        enabled: true,
        api_key: Some("or-key".into()),
        base_url: Some("https://openrouter.ai/api/v1".into()),
        generation_model: Some("black-forest-labs/flux-1.1-pro".into()),
        ..Default::default()
    });
    let cands = GenerateImageTool::plan_candidates(&config, Some("openrouter"));
    assert_eq!(cands[0].backend_kind(), "openai");
}

#[test]
fn registration_gate_decoupled_from_gemini_flag() {
    // #1672 gap 2: with Gemini globally DISABLED, a custom provider's
    // generation_model must still register generate_image.
    let mut customs = BTreeMap::new();
    customs.insert("qwen-image".to_string(), qwen_custom());
    let config = config_with(customs, vec!["custom:qwen-image".into()]);
    assert!(!config.image.generation.enabled);
    let tool = GenerateImageTool::from_config(&config).expect("provider route must register");
    assert_eq!(tool.name(), "generate_image");
}

#[test]
fn global_gemini_is_last_candidate_and_flag_gated() {
    let mut customs = BTreeMap::new();
    customs.insert("qwen-image".to_string(), qwen_custom());
    let mut config = config_with(customs, vec!["custom:qwen-image".into()]);
    config.image = ImageConfig {
        generation: ImageGenerationConfig {
            enabled: true,
            model: "gemini-3.1-flash-image-preview".into(),
            api_key: Some("GOOGLE_KEY".into()),
        },
        ..Default::default()
    };
    let cands = GenerateImageTool::plan_candidates(&config, None);
    assert_eq!(cands.len(), 2);
    assert_eq!(cands[0].backend_kind(), "openai"); // qwen leads
    assert_eq!(cands[1].backend_kind(), "gemini"); // global last
    assert_eq!(cands[1].model(), "gemini-3.1-flash-image-preview");

    // Flag off → global leg gone, provider route stays.
    config.image.generation.enabled = false;
    let cands = GenerateImageTool::plan_candidates(&config, None);
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].backend_kind(), "openai");
}

#[test]
fn nothing_configured_means_no_registration() {
    let config = Config::default();
    assert!(any_provider_generation(&config).is_empty());
    assert!(GenerateImageTool::from_config(&config).is_none());
}
