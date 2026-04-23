//! Token Tracking Tests
//!
//! Comprehensive tests for token counting, cache-aware pricing, and usage tracking
//! across all provider types (API, Claude CLI, OpenCode CLI).

use crate::brain::provider::types::TokenUsage;
use crate::usage::pricing::{PricingConfig, PricingEntry, ProviderBlock};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// TokenUsage struct tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn token_usage_default_has_zero_cache() {
    let usage = TokenUsage::default();
    assert_eq!(usage.input_tokens, 0);
    assert_eq!(usage.output_tokens, 0);
    assert_eq!(usage.cache_creation_tokens, 0);
    assert_eq!(usage.cache_read_tokens, 0);
}

#[test]
fn token_usage_total_excludes_cache() {
    let usage = TokenUsage {
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 80_000,
        cache_read_tokens: 15_000,
        ..Default::default()
    };
    // total() = non-cached input + output only — for context tracking
    assert_eq!(usage.total(), 150);
}

#[test]
fn token_usage_billable_input_includes_cache() {
    let usage = TokenUsage {
        input_tokens: 3,
        output_tokens: 500,
        cache_creation_tokens: 83_000,
        cache_read_tokens: 14_000,
        ..Default::default()
    };
    // billable_input = input + cache_creation + cache_read (falls back to context fields)
    assert_eq!(usage.billable_input(), 97_003);
}

#[test]
fn token_usage_billable_total_includes_everything() {
    let usage = TokenUsage {
        input_tokens: 3,
        output_tokens: 500,
        cache_creation_tokens: 83_000,
        cache_read_tokens: 14_000,
        ..Default::default()
    };
    assert_eq!(usage.billable_total(), 97_503);
}

#[test]
fn token_usage_no_cache_billable_equals_total() {
    let usage = TokenUsage {
        input_tokens: 1000,
        output_tokens: 500,
        ..Default::default()
    };
    assert_eq!(usage.total(), usage.billable_total());
    assert_eq!(usage.billable_input(), usage.input_tokens);
}

#[test]
fn token_usage_serde_roundtrip_with_cache() {
    let usage = TokenUsage {
        input_tokens: 100,
        output_tokens: 200,
        cache_creation_tokens: 50_000,
        cache_read_tokens: 10_000,
        ..Default::default()
    };
    let json = serde_json::to_string(&usage).unwrap();
    let deserialized: TokenUsage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.input_tokens, 100);
    assert_eq!(deserialized.output_tokens, 200);
    assert_eq!(deserialized.cache_creation_tokens, 50_000);
    assert_eq!(deserialized.cache_read_tokens, 10_000);
}

#[test]
fn token_usage_serde_missing_cache_defaults_to_zero() {
    // Simulates API providers that don't send cache fields
    let json = r#"{"input_tokens": 1000, "output_tokens": 500}"#;
    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 500);
    assert_eq!(usage.cache_creation_tokens, 0);
    assert_eq!(usage.cache_read_tokens, 0);
}

#[test]
fn token_usage_serde_with_anthropic_field_names() {
    // Anthropic API sends these exact field names
    let json = r#"{
        "input_tokens": 3,
        "output_tokens": 501,
        "cache_creation_tokens": 83129,
        "cache_read_tokens": 13981
    }"#;
    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.input_tokens, 3);
    assert_eq!(usage.output_tokens, 501);
    assert_eq!(usage.cache_creation_tokens, 83129);
    assert_eq!(usage.cache_read_tokens, 13981);
    assert_eq!(usage.billable_input(), 97113);
    assert_eq!(usage.billable_total(), 97614);
}

// ─────────────────────────────────────────────────────────────────────────────
// Cache-aware pricing tests
// ─────────────────────────────────────────────────────────────────────────────

fn test_pricing_config() -> PricingConfig {
    let mut providers = HashMap::new();
    providers.insert(
        "anthropic".to_string(),
        ProviderBlock {
            entries: vec![
                PricingEntry {
                    prefix: "opus-4".to_string(),
                    input_per_m: 15.0,
                    output_per_m: 75.0,
                    cache_write_per_m: Some(18.75),
                    cache_read_per_m: Some(1.50),
                },
                PricingEntry {
                    prefix: "sonnet-4".to_string(),
                    input_per_m: 3.0,
                    output_per_m: 15.0,
                    cache_write_per_m: Some(3.75),
                    cache_read_per_m: Some(0.30),
                },
            ],
        },
    );
    providers.insert(
        "openai".to_string(),
        ProviderBlock {
            entries: vec![PricingEntry {
                prefix: "gpt-5".to_string(),
                input_per_m: 1.25,
                output_per_m: 10.0,
                cache_write_per_m: None,
                cache_read_per_m: None,
            }],
        },
    );
    PricingConfig { providers }
}

#[test]
fn cache_pricing_opus_default_rates() {
    let cfg = test_pricing_config();
    // opus-4: input=$15, output=$75, cache_write=$18.75, cache_read=$1.50
    let cost = cfg.calculate_cost_with_cache(
        "claude-opus-4", 1_000_000, // 1M non-cached input
        1_000_000, // 1M output
        0,         // no cache write
        0,         // no cache read
    );
    assert_eq!(cost, 90.0); // $15 + $75
}

#[test]
fn cache_pricing_opus_with_cache_write() {
    let cfg = test_pricing_config();
    let cost = cfg.calculate_cost_with_cache(
        "claude-opus-4", 3,      // 3 non-cached input tokens
        500,    // 500 output tokens
        83_000, // 83K cache write tokens
        0,      // no cache read
    );
    // input: 3/1M * $15 = ~0
    // output: 500/1M * $75 = $0.0375
    // cache_write: 83000/1M * $18.75 = $1.55625
    let expected = (3.0 / 1e6 * 15.0) + (500.0 / 1e6 * 75.0) + (83_000.0 / 1e6 * 18.75);
    assert!((cost - expected).abs() < 0.0001);
}

#[test]
fn cache_pricing_opus_with_cache_read() {
    let cfg = test_pricing_config();
    let cost = cfg.calculate_cost_with_cache(
        "claude-opus-4", 3,      // 3 non-cached input
        500,    // 500 output
        0,      // no cache write
        14_000, // 14K cache read
    );
    // cache_read: 14000/1M * $1.50 = $0.021
    let expected = (3.0 / 1e6 * 15.0) + (500.0 / 1e6 * 75.0) + (14_000.0 / 1e6 * 1.50);
    assert!((cost - expected).abs() < 0.0001);
}

#[test]
fn cache_pricing_opus_full_breakdown() {
    let cfg = test_pricing_config();
    // Realistic CLI session: 3 input, 501 output, 83K cache write, 14K cache read
    let cost = cfg.calculate_cost_with_cache("claude-opus-4", 3, 501, 83_129, 13_981);
    let expected = (3.0 / 1e6 * 15.0)
        + (501.0 / 1e6 * 75.0)
        + (83_129.0 / 1e6 * 18.75)
        + (13_981.0 / 1e6 * 1.50);
    assert!((cost - expected).abs() < 0.0001);
    // Should be roughly $0.53 — NOT $0.00 like the old broken calculation
    assert!(cost > 0.5);
}

#[test]
fn cache_pricing_sonnet_explicit_rates() {
    let cfg = test_pricing_config();
    // sonnet-4: explicit rates cache_write=$3.75, cache_read=$0.30
    let cost = cfg.calculate_cost_with_cache("sonnet-4-6", 1000, 1000, 50_000, 10_000);
    let expected = (1000.0 / 1e6 * 3.0)
        + (1000.0 / 1e6 * 15.0)
        + (50_000.0 / 1e6 * 3.75)
        + (10_000.0 / 1e6 * 0.30);
    assert!((cost - expected).abs() < 0.0001);
}

#[test]
fn cache_pricing_no_cache_matches_regular() {
    let cfg = test_pricing_config();
    let regular = cfg.calculate_cost("claude-opus-4", 1_000_000, 1_000_000);
    let with_cache = cfg.calculate_cost_with_cache("claude-opus-4", 1_000_000, 1_000_000, 0, 0);
    assert_eq!(regular, with_cache);
}

#[test]
fn cache_pricing_unknown_model_returns_zero() {
    let cfg = test_pricing_config();
    let cost = cfg.calculate_cost_with_cache("unknown-model", 1_000_000, 1_000_000, 50_000, 10_000);
    assert_eq!(cost, 0.0);
}

#[test]
fn cache_pricing_defaults_load_and_compute() {
    // Load from the embedded example file (same as runtime)
    let cfg = PricingConfig::load().expect("usage_pricing.toml should load");

    // Opus with cache — real scenario
    let cost = cfg.calculate_cost_with_cache("claude-opus-4", 3, 501, 83_129, 13_981);
    assert!(cost > 0.0, "Cost should be > 0 for opus with cache tokens");

    // No cache — should match regular calculate_cost
    let regular = cfg.calculate_cost("claude-opus-4", 1000, 500);
    let with_cache = cfg.calculate_cost_with_cache("claude-opus-4", 1000, 500, 0, 0);
    assert_eq!(regular, with_cache);
}

// ─────────────────────────────────────────────────────────────────────────────
// CliUsage → TokenUsage flow tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cli_usage_total_input_excludes_cache() {
    // Simulates what claude_cli.rs does: total_input() returns non-cached only
    // Cache tokens are carried in separate TokenUsage fields
    let input_tokens: u32 = 3;
    let cache_creation: u32 = 83_129;
    let cache_read: u32 = 13_981;
    let output_tokens: u32 = 501;

    // What gets set as TokenUsage.input_tokens
    let total_input = input_tokens; // non-cached only

    let usage = TokenUsage {
        input_tokens: total_input,
        output_tokens,
        cache_creation_tokens: cache_creation,
        cache_read_tokens: cache_read,
        ..Default::default()
    };

    // Context tracking: small (non-cached)
    assert_eq!(usage.total(), 504); // 3 + 501

    // Billing: full (falls back to context cache fields when billing fields are 0)
    assert_eq!(usage.billable_input(), 97_113); // 3 + 83129 + 13981
    assert_eq!(usage.billable_total(), 97_614); // 97113 + 501
}

#[test]
fn cli_usage_no_cache_total_equals_billable() {
    // OpenCode CLI doesn't report cache tokens
    let usage = TokenUsage {
        input_tokens: 5000,
        output_tokens: 1200,
        ..Default::default()
    };
    assert_eq!(usage.total(), 6200);
    assert_eq!(usage.billable_total(), 6200);
    assert_eq!(usage.billable_input(), 5000);
}

// ─────────────────────────────────────────────────────────────────────────────
// Billable input calculation (matches tool_loop.rs logic)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn billable_input_multi_iteration_accumulation() {
    // Simulates tool_loop accumulating across iterations
    let mut total_input_tokens = 0u32;
    let mut total_output_tokens = 0u32;
    let mut total_cache_creation = 0u32;
    let mut total_cache_read = 0u32;

    // Iteration 1: fresh cache write
    let iter1 = TokenUsage {
        input_tokens: 3,
        output_tokens: 200,
        cache_creation_tokens: 80_000,
        cache_read_tokens: 0,
        ..Default::default()
    };
    total_input_tokens += iter1.input_tokens;
    total_output_tokens += iter1.output_tokens;
    total_cache_creation += iter1.cache_creation_tokens;
    total_cache_read += iter1.cache_read_tokens;

    // Iteration 2: cache read (system prompt already cached)
    let iter2 = TokenUsage {
        input_tokens: 5,
        output_tokens: 300,
        cache_creation_tokens: 0,
        cache_read_tokens: 80_000,
        ..Default::default()
    };
    total_input_tokens += iter2.input_tokens;
    total_output_tokens += iter2.output_tokens;
    total_cache_creation += iter2.cache_creation_tokens;
    total_cache_read += iter2.cache_read_tokens;

    let billable_input = total_input_tokens + total_cache_creation + total_cache_read;
    let total_tokens = billable_input + total_output_tokens;

    assert_eq!(total_input_tokens, 8);
    assert_eq!(total_cache_creation, 80_000);
    assert_eq!(total_cache_read, 80_000);
    assert_eq!(billable_input, 160_008);
    assert_eq!(total_tokens, 160_508);
}

#[test]
fn billable_input_api_provider_no_cache() {
    // API providers (non-CLI) don't report cache — all tokens are regular input
    let usage = TokenUsage {
        input_tokens: 5000,
        output_tokens: 1000,
        ..Default::default()
    };

    let billable_input = usage.input_tokens + usage.cache_creation_tokens + usage.cache_read_tokens;
    assert_eq!(billable_input, 5000); // Same as input_tokens
}

// ─────────────────────────────────────────────────────────────────────────────
// Cost accuracy regression tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cost_not_zero_for_cli_with_cache_tokens() {
    // THE BUG: was returning $0.00 because cache tokens were excluded
    let cfg = PricingConfig::load().expect("usage_pricing.toml should load");
    let cost = cfg.calculate_cost_with_cache(
        "claude-opus-4", 3,      // non-cached input
        501,    // output
        83_129, // cache write
        13_981, // cache read
    );
    assert!(
        cost > 0.0,
        "Cost must not be zero when cache tokens are present"
    );
    assert!(cost > 0.01, "Cost should be non-trivial for 97K tokens");
}

#[test]
fn cost_cache_write_more_expensive_than_regular() {
    let cfg = test_pricing_config();
    // 1M tokens as regular input
    let regular_cost = cfg.calculate_cost_with_cache("claude-opus-4", 1_000_000, 0, 0, 0);
    // 1M tokens as cache write
    let cache_write_cost = cfg.calculate_cost_with_cache("claude-opus-4", 0, 0, 1_000_000, 0);
    // Cache write (1.25x) should be more expensive than regular input
    assert!(cache_write_cost > regular_cost);
}

#[test]
fn cost_cache_read_cheaper_than_regular() {
    let cfg = test_pricing_config();
    // 1M tokens as regular input
    let regular_cost = cfg.calculate_cost_with_cache("claude-opus-4", 1_000_000, 0, 0, 0);
    // 1M tokens as cache read
    let cache_read_cost = cfg.calculate_cost_with_cache("claude-opus-4", 0, 0, 0, 1_000_000);
    // Cache read (0.1x) should be cheaper than regular input
    assert!(cache_read_cost < regular_cost);
}

#[test]
fn cost_old_vs_new_calculation_difference() {
    let cfg = PricingConfig::load().expect("usage_pricing.toml should load");

    // OLD (broken): treated 3 input + 501 output only → ~$0.00
    let old_cost = cfg.calculate_cost("claude-opus-4", 3, 501);

    // NEW (correct): includes cache breakdown
    let new_cost = cfg.calculate_cost_with_cache("claude-opus-4", 3, 501, 83_129, 13_981);

    // New cost should be orders of magnitude higher
    assert!(new_cost > old_cost * 10.0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider-specific token format tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn anthropic_api_usage_format() {
    // Anthropic API returns usage in message_start and message_delta
    let json = r#"{
        "input_tokens": 25000,
        "output_tokens": 1500,
        "cache_creation_tokens": 0,
        "cache_read_tokens": 0
    }"#;
    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.billable_total(), 26_500);
}

#[test]
fn claude_cli_usage_format_with_cache() {
    // Claude CLI reports cache separately
    let json = r#"{
        "input_tokens": 3,
        "output_tokens": 501,
        "cache_creation_tokens": 83129,
        "cache_read_tokens": 13981
    }"#;
    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.input_tokens, 3);
    assert_eq!(usage.billable_input(), 97_113);
    assert_eq!(usage.billable_total(), 97_614);
    // Context tracking should NOT be 97K
    assert_eq!(usage.total(), 504);
}

#[test]
fn opencode_cli_usage_format_no_cache() {
    // OpenCode CLI doesn't report cache fields
    let json = r#"{
        "input_tokens": 5000,
        "output_tokens": 1200
    }"#;
    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.cache_creation_tokens, 0);
    assert_eq!(usage.cache_read_tokens, 0);
    assert_eq!(usage.billable_total(), 6_200);
    assert_eq!(usage.total(), 6_200);
}

#[test]
fn gemini_usage_format_no_cache() {
    // Gemini API doesn't report cache
    let json = r#"{
        "input_tokens": 8000,
        "output_tokens": 2000
    }"#;
    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.billable_total(), 10_000);
}

#[test]
fn openai_usage_format_no_cache() {
    // OpenAI API doesn't report cache
    let json = r#"{
        "input_tokens": 12000,
        "output_tokens": 3000
    }"#;
    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.billable_total(), 15_000);
}
