//! Centralized model pricing table
//!
//! Loaded from `~/.opencrabs/usage_pricing.toml` at runtime.
//! Falls back to compiled-in defaults if the file is missing.
//! Users can edit the file live — changes take effect on next `/usage` open.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

/// A single model pricing entry.
/// `prefix` is matched as a substring of the model name (case-insensitive).
/// First match wins, so put more specific prefixes before general ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingEntry {
    pub prefix: String,
    pub input_per_m: f64,
    pub output_per_m: f64,
}

/// The full pricing table, keyed by provider name (for display only).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PricingConfig {
    #[serde(default)]
    pub providers: HashMap<String, Vec<PricingEntry>>,
}

impl PricingConfig {
    /// Calculate cost for a model + token counts.
    /// Searches all providers, matches by prefix (case-insensitive, first match wins).
    /// Returns 0.0 if no match found.
    pub fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        let m = model.to_lowercase();
        for entries in self.providers.values() {
            for entry in entries {
                if m.contains(&entry.prefix.to_lowercase()) {
                    let input = (input_tokens as f64 / 1_000_000.0) * entry.input_per_m;
                    let output = (output_tokens as f64 / 1_000_000.0) * entry.output_per_m;
                    return input + output;
                }
            }
        }
        0.0
    }

    /// Estimate cost from a combined token count using an 80/20 input/output split.
    /// Returns None if model is unknown.
    pub fn estimate_cost(&self, model: &str, token_count: i64) -> Option<f64> {
        let m = model.to_lowercase();
        for entries in self.providers.values() {
            for entry in entries {
                if m.contains(&entry.prefix.to_lowercase()) {
                    let input = (token_count as f64 * 0.80 / 1_000_000.0) * entry.input_per_m;
                    let output = (token_count as f64 * 0.20 / 1_000_000.0) * entry.output_per_m;
                    return Some(input + output);
                }
            }
        }
        None
    }

    /// Load from ~/.opencrabs/usage_pricing.toml.
    /// Returns compiled-in defaults if file is missing or unreadable.
    pub fn load() -> Self {
        let path = crate::config::opencrabs_home().join("usage_pricing.toml");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = toml::from_str::<PricingConfig>(&content) {
                return cfg;
            }
        }
        Self::defaults()
    }

    /// Write the default pricing file to ~/.opencrabs/usage_pricing.toml if it doesn't exist.
    pub fn write_defaults_if_missing() {
        let path = crate::config::opencrabs_home().join("usage_pricing.toml");
        if !path.exists() {
            let _ = std::fs::write(&path, DEFAULT_PRICING_TOML);
        }
    }

    /// Compiled-in defaults — used as fallback if file missing.
    pub fn defaults() -> Self {
        toml::from_str(DEFAULT_PRICING_TOML).unwrap_or_default()
    }
}

/// Global pricing instance — reloaded fresh on each `/usage` open via `pricing()`.
/// Not a true singleton; callers should use `PricingConfig::load()` directly for freshness.
static PRICING: OnceLock<PricingConfig> = OnceLock::new();

/// Returns the global pricing config, initialized once per process.
/// For live-reload behavior, call `PricingConfig::load()` directly instead.
pub fn pricing() -> &'static PricingConfig {
    PRICING.get_or_init(PricingConfig::load)
}

// ─────────────────────────────────────────────────────────────────────────────
// Default pricing table (compiled in as fallback)
// Rates verified via OpenRouter API 2026-02-25
// ─────────────────────────────────────────────────────────────────────────────
const DEFAULT_PRICING_TOML: &str = r#"
# OpenCrabs Usage Pricing Table
# Edit this file to customize pricing or add new models.
# Changes take effect immediately — no restart needed.
#
# Rules:
#   - `prefix` is matched as a case-insensitive substring of the model name
#   - First match within each provider wins — put specific prefixes before general ones
#   - Costs are per 1 million tokens (USD)

[providers.anthropic]
entries = [
  # Claude Opus 4.x — $5/$25 per M tokens
  { prefix = "claude-opus-4",      input_per_m = 5.0,  output_per_m = 25.0 },
  # Claude Opus 3 (legacy) — $15/$75
  { prefix = "claude-3-opus",      input_per_m = 15.0, output_per_m = 75.0 },
  # Claude Sonnet 4.x — $3/$15
  { prefix = "claude-sonnet-4",    input_per_m = 3.0,  output_per_m = 15.0 },
  # Claude 3.7 Sonnet — $3/$15
  { prefix = "claude-3-7-sonnet",  input_per_m = 3.0,  output_per_m = 15.0 },
  # Claude 3.5 Sonnet — $3/$15
  { prefix = "claude-3-5-sonnet",  input_per_m = 3.0,  output_per_m = 15.0 },
  # Claude 3 Sonnet (legacy) — $3/$15
  { prefix = "claude-3-sonnet",    input_per_m = 3.0,  output_per_m = 15.0 },
  # Claude Haiku 4.x — $1/$5
  { prefix = "claude-haiku-4",     input_per_m = 1.0,  output_per_m = 5.0  },
  # Claude 3.5 Haiku — $0.80/$4
  { prefix = "claude-3-5-haiku",   input_per_m = 0.80, output_per_m = 4.0  },
  # Claude 3 Haiku (legacy) — $0.25/$1.25
  { prefix = "claude-3-haiku",     input_per_m = 0.25, output_per_m = 1.25 },
]

[providers.openai]
entries = [
  { prefix = "gpt-4o-mini",        input_per_m = 0.15, output_per_m = 0.60  },
  { prefix = "gpt-4o",             input_per_m = 2.50, output_per_m = 10.0  },
  { prefix = "gpt-4-turbo",        input_per_m = 10.0, output_per_m = 30.0  },
  { prefix = "gpt-4-32k",          input_per_m = 60.0, output_per_m = 120.0 },
  { prefix = "gpt-4",              input_per_m = 30.0, output_per_m = 60.0  },
  { prefix = "gpt-3.5-turbo-16k",  input_per_m = 3.0,  output_per_m = 4.0   },
  { prefix = "gpt-3.5-turbo",      input_per_m = 0.50, output_per_m = 1.50  },
  { prefix = "o3-mini",            input_per_m = 1.10, output_per_m = 4.40  },
  { prefix = "o3",                 input_per_m = 10.0, output_per_m = 40.0  },
  { prefix = "o1-mini",            input_per_m = 1.10, output_per_m = 4.40  },
  { prefix = "o1",                 input_per_m = 15.0, output_per_m = 60.0  },
]

[providers.minimax]
entries = [
  # MiniMax-M2.5 highspeed — $0.60/$2.40
  { prefix = "minimax-m2.5-high",  input_per_m = 0.60, output_per_m = 2.40  },
  # MiniMax-M2.5 standard — $0.30/$1.20
  { prefix = "minimax-m2.5",       input_per_m = 0.30, output_per_m = 1.20  },
  # MiniMax-M2.1 — $0.30/$1.20
  { prefix = "minimax-m2.1",       input_per_m = 0.30, output_per_m = 1.20  },
  # MiniMax-Text-01 — $0.20/$1.10
  { prefix = "minimax-text-01",    input_per_m = 0.20, output_per_m = 1.10  },
  # MiniMax generic fallback
  { prefix = "minimax",            input_per_m = 0.30, output_per_m = 1.20  },
]

[providers.google]
entries = [
  { prefix = "gemini-2.0-flash",   input_per_m = 0.10, output_per_m = 0.40  },
  { prefix = "gemini-1.5-pro",     input_per_m = 1.25, output_per_m = 5.0   },
  { prefix = "gemini-1.5-flash",   input_per_m = 0.075,output_per_m = 0.30  },
]

[providers.deepseek]
entries = [
  { prefix = "deepseek-r1",        input_per_m = 0.55, output_per_m = 2.19  },
  { prefix = "deepseek-v3",        input_per_m = 0.27, output_per_m = 1.10  },
  { prefix = "deepseek",           input_per_m = 0.27, output_per_m = 1.10  },
]

[providers.meta]
entries = [
  { prefix = "llama-3.3-70b",      input_per_m = 0.59, output_per_m = 0.79  },
  { prefix = "llama-3.1-405b",     input_per_m = 2.70, output_per_m = 2.70  },
  { prefix = "llama-3.1-70b",      input_per_m = 0.52, output_per_m = 0.75  },
  { prefix = "llama-3.1-8b",       input_per_m = 0.07, output_per_m = 0.07  },
]
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_parse() {
        let cfg = PricingConfig::defaults();
        assert!(!cfg.providers.is_empty());
    }

    #[test]
    fn test_calculate_cost_sonnet4() {
        let cfg = PricingConfig::defaults();
        let cost = cfg.calculate_cost("claude-sonnet-4-6", 1_000_000, 1_000_000);
        assert_eq!(cost, 18.0); // $3 + $15
    }

    #[test]
    fn test_calculate_cost_opus4() {
        let cfg = PricingConfig::defaults();
        let cost = cfg.calculate_cost("claude-opus-4-6", 1_000_000, 1_000_000);
        assert_eq!(cost, 30.0); // $5 + $25
    }

    #[test]
    fn test_calculate_cost_minimax() {
        let cfg = PricingConfig::defaults();
        let cost = cfg.calculate_cost("MiniMax-M2.5", 1_000_000, 1_000_000);
        assert_eq!(cost, 1.50); // $0.30 + $1.20
    }

    #[test]
    fn test_unknown_model_zero() {
        let cfg = PricingConfig::defaults();
        let cost = cfg.calculate_cost("some-unknown-model-xyz", 1_000_000, 1_000_000);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_estimate_cost() {
        let cfg = PricingConfig::defaults();
        // 1M tokens, 80% input = 800k @ $3, 20% output = 200k @ $15
        let est = cfg.estimate_cost("claude-sonnet-4-6", 1_000_000);
        assert!(est.is_some());
        let est = est.unwrap();
        assert!((est - 5.40).abs() < 0.001); // 0.8*3 + 0.2*15 = 2.4 + 3.0 = 5.4
    }
}
