//! Cached brain prefix + uncached runtime suffix (#658).
//!
//! The stable brain must be byte-stable across sessions so the provider prompt
//! cache reuses it; the per-session Runtime Info block rides uncached after the
//! cache breakpoint. These lock in the split (`split_runtime_suffix` /
//! `with_system` / `system_full`) and the qwen request encoding (2-part system
//! with cache_control on the stable part only).

use crate::brain::prompt_builder::split_runtime_suffix;
use crate::brain::provider::custom_openai_compatible::OpenAIProvider;
use crate::brain::provider::qwen::qwen_body_transform;
use crate::brain::provider::{LLMRequest, Message};
use crate::brain::tools::catalog::tool_access_prompt;

const BRAIN: &str = "--- SOUL.md ---\nbe kind\n\n--- Runtime Info ---\n\
     Model: qwen3.8-max-preview\nProvider: modelstudio\nCurrent date: 2026-07-21 (UTC)\n\n\
     --- AGENTS.md ---\nrules here\n";

// ── split_runtime_suffix ────────────────────────────────────────────

#[test]
fn split_extracts_runtime_block_and_leaves_stable_prefix() {
    let (stable, suffix) = split_runtime_suffix(BRAIN);
    assert!(!stable.contains("--- Runtime Info ---"));
    assert!(!stable.contains("Current date"));
    assert!(stable.contains("SOUL.md") && stable.contains("AGENTS.md"));
    let suffix = suffix.expect("runtime block present");
    assert!(suffix.starts_with("--- Runtime Info ---"));
    assert!(suffix.contains("Model: qwen3.8-max-preview") && suffix.contains("Current date"));
}

#[test]
fn split_is_noop_without_runtime_block() {
    // No Runtime Info block -> input returned verbatim (a true no-op).
    let (stable, suffix) = split_runtime_suffix("--- SOUL.md ---\njust soul\n");
    assert_eq!(stable, "--- SOUL.md ---\njust soul\n");
    assert!(suffix.is_none());
}

// ── with_system / system_full ───────────────────────────────────────

#[test]
fn with_system_keeps_whole_system_no_split() {
    // #693: the #658 cache split is DISABLED — with_system stores the WHOLE
    // system as one string (Runtime Info in its original place) and sets no
    // suffix, so every provider sends a single-string system the model reliably
    // calls tools with. The array-shaped 2-part system broke tool-calling.
    let req = LLMRequest::new("m", vec![]).with_system(BRAIN);
    assert_eq!(req.system.as_deref(), Some(BRAIN));
    assert!(
        req.system_suffix.is_none(),
        "no runtime suffix — split is off"
    );
    assert!(req.system.as_ref().unwrap().contains("Runtime Info"));
    // system_full is the same single string.
    assert_eq!(req.system_full().as_deref(), Some(BRAIN));
}

#[test]
fn lazy_tools_block_stays_in_cached_prefix_runtime_in_suffix() {
    // Mirror live_system_brain: stable sections, a Runtime Info block mid-brain,
    // then the lazy-tools block appended at the very end. The split must keep the
    // deterministic lazy block in the CACHED prefix and pull only the volatile
    // Runtime Info into the suffix (#662). Guards against a refactor pushing
    // volatile content into the cached prefix or lazy content into the suffix.
    let lazy = tool_access_prompt(false);
    let brain = format!(
        "--- SOUL.md ---\nbe kind\n\n\
         --- Runtime Info ---\nModel: qwen3.8-max-preview\nCurrent date: 2026-07-21 (UTC)\n\n\
         --- AGENTS.md ---\nrules\n\n{lazy}"
    );
    let (stable, suffix) = split_runtime_suffix(&brain);
    // (split trims the reconstructed tail's edge whitespace, so match the
    // trimmed lazy block.)
    assert!(
        stable.contains(lazy.trim()),
        "the lazy-tools block must remain in the cached prefix"
    );
    assert!(
        !stable.contains("Current date"),
        "volatile runtime info must not be in the cached prefix"
    );
    let suffix = suffix.expect("runtime block present");
    assert!(suffix.contains("--- Runtime Info ---") && suffix.contains("Current date"));
}

#[test]
fn system_full_handles_absent_parts() {
    assert!(LLMRequest::new("m", vec![]).system_full().is_none());
    let only_system = LLMRequest::new("m", vec![]).with_system("no marker here");
    assert_eq!(only_system.system_full().as_deref(), Some("no marker here"));
}

// ── qwen request encoding ───────────────────────────────────────────

fn qwen_provider() -> OpenAIProvider {
    OpenAIProvider::with_base_url(
        "k".to_string(),
        "https://ws-x.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/chat/completions"
            .to_string(),
    )
    .with_name("modelstudio")
}

#[test]
fn qwen_request_encodes_system_as_single_string() {
    // #693: split disabled — to_openai_request emits the qwen system as a single
    // STRING (not a 2-part array). qwen_body_transform later stamps cache_control
    // as a 1-element array (the pre-#658 shape), but the encoder itself must not
    // produce an array-shaped system, which is what broke tool-calling.
    let req = LLMRequest::new("qwen3.8-max-preview", vec![Message::user("hi")]).with_system(BRAIN);
    let encoded = qwen_provider().to_openai_request(req);
    let val = serde_json::to_value(&encoded).unwrap();
    let sys = val["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "system")
        .expect("system message");
    assert!(
        sys["content"].is_string(),
        "qwen system content must be a single string, not an array: {}",
        sys["content"]
    );
    let s = sys["content"].as_str().unwrap();
    assert!(s.contains("SOUL.md") && s.contains("Runtime Info"));
}

#[test]
fn qwen_body_transform_caches_the_stable_part_not_the_suffix() {
    let body = serde_json::json!({
        "model": "qwen3.8-max-preview",
        "stream": true,
        "messages": [
            { "role": "system", "content": [
                { "type": "text", "text": "stable brain" },
                { "type": "text", "text": "--- Runtime Info ---\nModel: x" }
            ]},
            { "role": "user", "content": "hi" }
        ]
    });
    let out = qwen_body_transform(body);
    let sys = &out["messages"][0];
    assert_eq!(
        sys["content"][0]["cache_control"]["type"], "ephemeral",
        "stable part must carry the cache breakpoint"
    );
    assert!(
        sys["content"][1].get("cache_control").is_none()
            || sys["content"][1]["cache_control"].is_null(),
        "the runtime suffix part must stay uncached"
    );
}
