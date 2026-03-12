//! Tests for channel model switching and provider section resolution.

use super::commands::provider_section;

// ── provider_section ──────────────────────────────────────────────────

#[test]
fn provider_section_known_providers() {
    assert_eq!(
        provider_section("anthropic"),
        Some("providers.anthropic".to_string())
    );
    assert_eq!(
        provider_section("openai"),
        Some("providers.openai".to_string())
    );
    assert_eq!(
        provider_section("github"),
        Some("providers.github".to_string())
    );
    assert_eq!(
        provider_section("gemini"),
        Some("providers.gemini".to_string())
    );
    assert_eq!(
        provider_section("google"),
        Some("providers.gemini".to_string())
    );
    assert_eq!(
        provider_section("openrouter"),
        Some("providers.openrouter".to_string())
    );
    assert_eq!(
        provider_section("minimax"),
        Some("providers.minimax".to_string())
    );
}

#[test]
fn provider_section_custom() {
    assert_eq!(
        provider_section("custom:deepseek"),
        Some("providers.custom.deepseek".to_string())
    );
    assert_eq!(
        provider_section("custom:local-llm"),
        Some("providers.custom.local-llm".to_string())
    );
}

#[test]
fn provider_section_custom_parenthesized() {
    assert_eq!(
        provider_section("custom(nvidia)"),
        Some("providers.custom.nvidia".to_string())
    );
}

#[test]
fn provider_section_unknown_returns_none() {
    assert_eq!(provider_section("mystery"), None);
    assert_eq!(provider_section(""), None);
}

#[test]
fn provider_section_case_insensitive() {
    assert_eq!(
        provider_section("Anthropic"),
        Some("providers.anthropic".to_string())
    );
    assert_eq!(
        provider_section("OPENAI"),
        Some("providers.openai".to_string())
    );
    assert_eq!(
        provider_section("GitHub Copilot"),
        Some("providers.github".to_string())
    );
    assert_eq!(
        provider_section("MiniMax"),
        Some("providers.minimax".to_string())
    );
}
