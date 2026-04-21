//! Model pricing configuration
//!
//! Loaded from `~/.opencrabs/usage_pricing.toml` at runtime.
//! No compiled-in fallback — if the file is missing or broken, an error is returned.
//! Users can edit the file live — changes take effect on next `/usage` open.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single model pricing entry.
/// `prefix` is matched as a substring of the model name (case-insensitive).
/// First match wins, so put more specific prefixes before general ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingEntry {
    pub prefix: String,
    pub input_per_m: f64,
    pub output_per_m: f64,
    /// Cache write cost per million tokens (defaults to 1.25x input_per_m if absent)
    #[serde(default)]
    pub cache_write_per_m: Option<f64>,
    /// Cache read cost per million tokens (defaults to 0.1x input_per_m if absent)
    #[serde(default)]
    pub cache_read_per_m: Option<f64>,
}

/// Per-provider block in the TOML file.
/// TOML format: `[providers.anthropic]\nentries = [...]`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderBlock {
    #[serde(default)]
    pub entries: Vec<PricingEntry>,
}

/// The full pricing table, keyed by provider name (for display only).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PricingConfig {
    #[serde(default)]
    pub providers: HashMap<String, ProviderBlock>,
}

impl PricingConfig {
    /// Calculate cost for a model + token counts (no cache breakdown).
    /// Treats all input tokens at the regular input rate.
    pub fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        self.calculate_cost_with_cache(model, input_tokens, output_tokens, 0, 0)
    }

    /// Calculate cost with full cache breakdown.
    /// `input_tokens` = non-cached input only.
    /// Cache write defaults to 1.25x input rate, cache read to 0.1x.
    pub fn calculate_cost_with_cache(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_creation_tokens: u32,
        cache_read_tokens: u32,
    ) -> f64 {
        let m = model.to_lowercase();
        for block in self.providers.values() {
            for entry in &block.entries {
                if m.contains(&entry.prefix.to_lowercase()) {
                    let input = (input_tokens as f64 / 1_000_000.0) * entry.input_per_m;
                    let output = (output_tokens as f64 / 1_000_000.0) * entry.output_per_m;
                    let cache_write_rate =
                        entry.cache_write_per_m.unwrap_or(entry.input_per_m * 1.25);
                    let cache_read_rate = entry.cache_read_per_m.unwrap_or(entry.input_per_m * 0.1);
                    let cache_write =
                        (cache_creation_tokens as f64 / 1_000_000.0) * cache_write_rate;
                    let cache_read = (cache_read_tokens as f64 / 1_000_000.0) * cache_read_rate;
                    return input + output + cache_write + cache_read;
                }
            }
        }
        0.0
    }

    /// Estimate cost from a combined token count using an 80/20 input/output split.
    /// Returns None if model is unknown.
    pub fn estimate_cost(&self, model: &str, token_count: i64) -> Option<f64> {
        let m = model.to_lowercase();
        for block in self.providers.values() {
            for entry in &block.entries {
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
    /// Supports both the current schema (`[providers.X] entries = [...]`) and the
    /// legacy on-disk schema (`[[usage.pricing.X]]` array-of-tables).
    /// Returns an error if the file is missing, unreadable, or unparseable.
    pub fn load() -> Result<Self, String> {
        let path = crate::config::opencrabs_home().join("usage_pricing.toml");
        let content = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "usage_pricing.toml not found at {:?}.\n\
                 Copy it from the repo: cp usage_pricing.toml.example {:?}\n\
                 Error: {}",
                path, path, e
            )
        })?;

        // Try current schema first.
        if let Ok(cfg) = toml::from_str::<PricingConfig>(&content)
            && !cfg.providers.is_empty()
        {
            return Ok(cfg);
        }

        // Try legacy schema: [[usage.pricing.<provider>]] entries
        if let Ok(cfg) = Self::load_legacy(&content)
            && !cfg.providers.is_empty()
        {
            tracing::warn!(
                "usage_pricing.toml uses old format — please update it to the new schema. \
                 See usage_pricing.toml.example in the repo"
            );
            let new_content = Self::serialize_to_toml(&cfg);
            let _ = std::fs::write(&path, new_content);
            return Ok(cfg);
        }

        Err(format!(
            "usage_pricing.toml at {:?} failed to parse with both schemas.\n\
             Check the file syntax or re-copy from usage_pricing.toml.example",
            path
        ))
    }

    /// Parse the legacy `[[usage.pricing.<provider>]]` format.
    fn load_legacy(content: &str) -> Result<Self, toml::de::Error> {
        #[derive(serde::Deserialize)]
        struct LegacyRoot {
            usage: Option<LegacyUsage>,
        }
        #[derive(serde::Deserialize)]
        struct LegacyUsage {
            pricing: Option<toml::Value>,
        }

        let root: LegacyRoot = toml::from_str(content)?;
        let pricing_val = root
            .usage
            .and_then(|u| u.pricing)
            .unwrap_or(toml::Value::Table(toml::map::Map::new()));

        let mut providers: HashMap<String, ProviderBlock> = HashMap::new();
        if let toml::Value::Table(table) = pricing_val {
            for (provider_name, entries_val) in table {
                if let toml::Value::Array(arr) = entries_val {
                    let entries: Vec<PricingEntry> =
                        arr.into_iter().filter_map(|v| v.try_into().ok()).collect();
                    if !entries.is_empty() {
                        providers.insert(provider_name, ProviderBlock { entries });
                    }
                }
            }
        }

        Ok(PricingConfig { providers })
    }

    /// Serialize a PricingConfig back to the canonical TOML schema.
    fn serialize_to_toml(cfg: &PricingConfig) -> String {
        let mut out = String::from(
            "# OpenCrabs Usage Pricing — auto-migrated to current schema.\n\
             # Edit freely. Changes take effect immediately on next /usage open.\n\
             # prefix is matched case-insensitively as a substring of the model name.\n\
             # Costs are per 1 million tokens (USD).\n\n",
        );
        let mut providers: Vec<(&String, &ProviderBlock)> = cfg.providers.iter().collect();
        providers.sort_by_key(|(k, _)| k.as_str());
        for (name, block) in providers {
            out.push_str(&format!("[providers.{}]\nentries = [\n", name));
            for e in &block.entries {
                out.push_str(&format!(
                    "  {{ prefix = {:?}, input_per_m = {}, output_per_m = {} }},\n",
                    e.prefix, e.input_per_m, e.output_per_m
                ));
            }
            out.push_str("]\n\n");
        }
        out
    }

    /// Copy `usage_pricing.toml.example` to brain directory on first run only.
    /// Existing users: see release notes for instructions to diff and update their file.
    pub fn seed_from_example() {
        let path = crate::config::opencrabs_home().join("usage_pricing.toml");

        if path.exists() {
            return; // User owns this file. Never overwrite.
        }

        let example_content = include_str!("../../usage_pricing.toml.example");
        if let Err(e) = std::fs::write(&path, example_content) {
            tracing::error!("Failed to seed usage_pricing.toml from example: {}", e);
        } else {
            tracing::info!("Seeded usage_pricing.toml from example");
        }
    }
}
