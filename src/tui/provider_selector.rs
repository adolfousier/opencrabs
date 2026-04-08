//! Shared provider + model selection state and logic.
//!
//! Used by both the `/models` dialog and the `/onboard` wizard to avoid
//! duplicate code that falls out of sync.

use crate::config::ProviderConfig;

/// Sentinel value stored in api_key_input when a key was loaded from config.
/// The actual key is never held in memory — this just signals "key exists".
pub const EXISTING_KEY_SENTINEL: &str = "__EXISTING_KEY__";

/// Provider definitions (index → info).
/// Last entry is always "Custom OpenAI-Compatible".
pub use crate::tui::onboarding::{PROVIDERS, ProviderInfo};

/// Index of the "Custom OpenAI-Compatible" sentinel (always last in PROVIDERS).
pub const CUSTOM_PROVIDER_IDX: usize = PROVIDERS.len() - 1;

/// First index used for existing custom provider instances (stored in config).
pub const CUSTOM_INSTANCES_START: usize = PROVIDERS.len();

/// Shared state for provider + model selection.
/// Both `/models` dialog and `/onboard` wizard embed this struct.
#[derive(Default)]
pub struct ProviderSelectorState {
    /// Currently selected provider index (0..CUSTOM_PROVIDER_IDX = static,
    /// CUSTOM_PROVIDER_IDX = new custom, CUSTOM_INSTANCES_START+ = existing customs)
    pub selected_provider: usize,
    /// Cached list of existing custom provider names
    pub custom_names: Vec<String>,
    /// Whether a key exists in config (boolean flag only — never load actual key into UI)
    pub has_existing_key: bool,
    /// User-typed API key, or EXISTING_KEY_SENTINEL when loaded from config
    pub api_key_input: String,
    /// Cursor position in api_key_input
    pub api_key_cursor: usize,
    /// Models fetched live from provider API
    pub models: Vec<String>,
    /// Models loaded from config.toml (fallback when API fetch not available)
    pub config_models: Vec<String>,
    /// Currently selected model index in filtered list
    pub selected_model: usize,
    /// Live search filter for models (case-insensitive substring match)
    pub model_filter: String,
    /// Whether an async model fetch is in progress
    pub models_fetching: bool,
    /// z.ai GLM endpoint type: 0=API, 1=Coding
    pub zhipu_endpoint_type: usize,
    /// Base URL for custom providers
    pub base_url: String,
    /// Model name for custom providers (free-text)
    pub custom_model: String,
    /// Identifier name for custom provider (e.g. "nvidia", "ollama")
    pub custom_name: String,
    /// Context window size for custom providers (digits only)
    pub context_window: String,
    /// Which field is currently focused (numbering varies by provider type)
    pub focused_field: usize,
    /// Whether the provider list is expanded/visible
    pub showing_providers: bool,
}

impl ProviderSelectorState {
    /// Get provider info for the currently selected provider.
    pub fn current_provider(&self) -> &ProviderInfo {
        let idx = if self.selected_provider >= CUSTOM_PROVIDER_IDX {
            CUSTOM_PROVIDER_IDX
        } else {
            self.selected_provider
        };
        &PROVIDERS[idx]
    }

    pub fn is_custom(&self) -> bool {
        self.selected_provider >= CUSTOM_PROVIDER_IDX
    }

    pub fn is_cli(&self) -> bool {
        let id = self.provider_id();
        id == "claude-cli" || id == "opencode-cli" || id == "qwen-code-cli"
    }

    pub fn is_zhipu(&self) -> bool {
        self.provider_id() == "zhipu"
    }

    /// Get the canonical provider id for the current selection.
    pub fn provider_id(&self) -> &'static str {
        if self.selected_provider < CUSTOM_PROVIDER_IDX {
            PROVIDERS[self.selected_provider].id
        } else {
            "" // custom
        }
    }

    /// Whether the current provider supports live model fetching from API.
    pub fn supports_model_fetch(&self) -> bool {
        matches!(
            self.provider_id(),
            "anthropic" | "openai" | "github" | "gemini" | "openrouter" | "zhipu" | "opencode-cli"
        )
    }

    /// Maximum number of fields for the current provider type.
    pub fn max_field(&self) -> usize {
        if self.is_custom() {
            6 // provider(0), base_url(1), api_key(2), model(3), name(4), context_window(5)
        } else if self.is_zhipu() {
            4 // provider(0), endpoint_type(1), api_key(2), model(3)
        } else {
            3 // provider(0), api_key(1), model(2)
        }
    }

    /// Whether the current api_key_input holds a pre-existing key sentinel.
    pub fn has_existing_key_sentinel(&self) -> bool {
        self.api_key_input == EXISTING_KEY_SENTINEL
    }

    /// Visual display order: named providers sorted alphabetically,
    /// then existing custom instances, then "+ New Custom" last.
    pub fn provider_display_order(&self) -> Vec<usize> {
        let num_customs = self.custom_names.len();
        // Named providers: everything except the last "Custom" sentinel
        let mut static_indices: Vec<usize> = (0..CUSTOM_PROVIDER_IDX).collect();
        static_indices.sort_by_key(|&i| PROVIDERS[i].name.to_ascii_lowercase());
        static_indices
            .into_iter()
            .chain(CUSTOM_INSTANCES_START..CUSTOM_INSTANCES_START + num_customs)
            .chain(std::iter::once(CUSTOM_PROVIDER_IDX))
            .collect()
    }

    /// Detect if an API key exists in config for the current provider.
    /// Sets `has_existing_key` flag and `api_key_input` sentinel. Never loads actual key.
    pub fn detect_existing_key(&mut self) {
        fn has_nonempty_key(p: Option<&ProviderConfig>) -> bool {
            p.and_then(|p| p.api_key.as_ref())
                .is_some_and(|k| !k.is_empty())
        }

        self.api_key_input.clear();
        self.has_existing_key = false;

        if let Ok(config) = crate::config::Config::load() {
            let has_key = if self.selected_provider < CUSTOM_PROVIDER_IDX {
                let id = PROVIDERS[self.selected_provider].id;
                if self.is_cli() {
                    false // CLI providers — no API key
                } else {
                    has_nonempty_key(crate::utils::providers::config_for(&config.providers, id))
                }
            } else if self.selected_provider == CUSTOM_PROVIDER_IDX {
                // New custom — start with blank fields
                self.custom_name.clear();
                self.base_url.clear();
                self.custom_model.clear();
                self.context_window.clear();
                false
            } else {
                // Existing custom provider
                let custom_idx = self.selected_provider - CUSTOM_INSTANCES_START;
                if let Some(cname) = self.custom_names.get(custom_idx).cloned() {
                    if let Some(c) = config.providers.custom_by_name(&cname) {
                        self.custom_name = cname;
                        self.base_url = c.base_url.clone().unwrap_or_default();
                        self.custom_model = c.default_model.clone().unwrap_or_default();
                        self.context_window = c
                            .context_window
                            .map(|cw| cw.to_string())
                            .unwrap_or_default();
                        c.api_key.as_ref().is_some_and(|k| !k.is_empty())
                    } else {
                        false
                    }
                } else {
                    false
                }
            };

            self.has_existing_key = has_key;
            if has_key {
                self.api_key_input = EXISTING_KEY_SENTINEL.to_string();
                self.api_key_cursor = 0;
            }
        }

        // Clear model selection when provider changes
        self.selected_model = 0;
        self.model_filter.clear();
    }

    /// Load custom provider fields when navigating to an existing custom (10+),
    /// clear fields for new custom (9), load zhipu endpoint type for index 6.
    pub fn load_custom_fields(&mut self) {
        if self.is_zhipu()
            && let Ok(config) = crate::config::Config::load()
            && let Some(zhipu) = &config.providers.zhipu
        {
            self.zhipu_endpoint_type = match zhipu.endpoint_type.as_deref() {
                Some("coding") => 1,
                _ => 0,
            };
        }
        if self.selected_provider == CUSTOM_PROVIDER_IDX {
            self.custom_name.clear();
            self.base_url.clear();
            self.custom_model.clear();
            self.context_window.clear();
        } else if self.selected_provider >= CUSTOM_INSTANCES_START {
            let custom_idx = self.selected_provider - CUSTOM_INSTANCES_START;
            if let Some(cname) = self.custom_names.get(custom_idx).cloned()
                && let Ok(config) = crate::config::Config::load()
                && let Some(c) = config.providers.custom_by_name(&cname)
            {
                self.custom_name = cname;
                self.base_url = c.base_url.clone().unwrap_or_default();
                self.custom_model = c.default_model.clone().unwrap_or_default();
                self.context_window = c
                    .context_window
                    .map(|cw| cw.to_string())
                    .unwrap_or_default();
                if c.api_key.as_ref().is_some_and(|k| !k.is_empty()) {
                    self.api_key_input = EXISTING_KEY_SENTINEL.to_string();
                }
            }
        }
    }

    /// Load the actual API key value from config for the current provider.
    /// Used when making API calls (fetch models, save config). Returns None if no key.
    pub fn load_api_key_from_config(&self) -> Option<String> {
        let config = crate::config::Config::load().ok()?;
        if self.selected_provider < CUSTOM_PROVIDER_IDX {
            crate::utils::providers::config_for(
                &config.providers,
                PROVIDERS[self.selected_provider].id,
            )
            .and_then(|p| p.api_key.clone())
        } else if self.selected_provider >= CUSTOM_INSTANCES_START {
            let custom_idx = self.selected_provider - CUSTOM_INSTANCES_START;
            self.custom_names.get(custom_idx).and_then(|name| {
                config
                    .providers
                    .custom_by_name(name)
                    .and_then(|p| p.api_key.clone())
            })
        } else {
            None
        }
        .filter(|k| !k.is_empty())
    }

    /// Resolve the effective API key: user-typed key if present, else config key.
    pub fn resolve_api_key(&self) -> Option<String> {
        if !self.api_key_input.is_empty() && self.api_key_input != EXISTING_KEY_SENTINEL {
            Some(self.api_key_input.clone())
        } else {
            self.load_api_key_from_config()
        }
    }

    /// Zhipu endpoint type as string for API calls.
    pub fn zhipu_endpoint_str(&self) -> Option<String> {
        if self.is_zhipu() {
            Some(
                if self.zhipu_endpoint_type == 1 {
                    "coding"
                } else {
                    "api"
                }
                .to_string(),
            )
        } else {
            None
        }
    }

    // ── Model list management ───────────────────────────────────────

    /// Reload config_models for the currently selected provider.
    pub fn reload_config_models(&mut self) {
        self.config_models.clear();
        if let Ok(config) = crate::config::Config::load() {
            if self.is_cli() {
                return; // CLI — static or fetched, no config models
            }
            if self.selected_provider < CUSTOM_PROVIDER_IDX {
                let id = PROVIDERS[self.selected_provider].id;
                if let Some(p) = crate::utils::providers::config_for(&config.providers, id)
                    && !p.models.is_empty()
                {
                    self.config_models = p.models.clone();
                    return;
                }
            } else if self.selected_provider >= CUSTOM_PROVIDER_IDX
                && let Some((_name, p)) = config.providers.active_custom()
                && !p.models.is_empty()
            {
                self.config_models = p.models.clone();
                return;
            }
        }
        self.config_models = load_default_models(self.provider_id());
    }

    /// All model names for the current provider (fetched → config → static fallback).
    pub fn all_model_names(&self) -> Vec<&str> {
        if !self.models.is_empty() {
            self.models.iter().map(|s| s.as_str()).collect()
        } else if !self.config_models.is_empty() {
            self.config_models.iter().map(|s| s.as_str()).collect()
        } else {
            self.current_provider().models.to_vec()
        }
    }

    /// Model names filtered by `model_filter` (case-insensitive substring match).
    pub fn filtered_model_names(&self) -> Vec<&str> {
        let all = self.all_model_names();
        if self.model_filter.is_empty() {
            all
        } else {
            let q = self.model_filter.to_lowercase();
            all.into_iter()
                .filter(|m| m.to_lowercase().contains(&q))
                .collect()
        }
    }

    /// Number of models available after applying the current filter.
    pub fn model_count(&self) -> usize {
        self.filtered_model_names().len()
    }

    /// Get the selected model name (resolves through filter).
    pub fn selected_model_name(&self) -> &str {
        let filtered = self.filtered_model_names();
        if let Some(name) = filtered.get(self.selected_model) {
            name
        } else {
            self.all_model_names().first().copied().unwrap_or("")
        }
    }

    /// Resolve `selected_model` index from `custom_model` string.
    pub fn resolve_selected_model_index(&mut self) {
        if self.custom_model.is_empty() {
            return;
        }
        let all = self.all_model_names();
        if let Some(idx) = all.iter().position(|m| *m == self.custom_model) {
            self.selected_model = idx;
        }
    }

    /// Cache existing custom provider names from config.
    pub fn load_custom_names(&mut self) {
        self.custom_names = crate::config::Config::load()
            .ok()
            .and_then(|c| c.providers.custom.map(|m| m.keys().cloned().collect()))
            .unwrap_or_default();
    }
}

/// Map an API model id to a human-readable display label.
/// Returns the id itself when no special label is defined.
/// Used by /models, onboarding, and the footer to show friendly names
/// for models whose API id is an opaque alias (e.g. qwen-oauth's `coder-model`).
pub fn model_display_label(model_id: &str) -> &str {
    match model_id {
        "coder-model" => "Qwen 3.6 Plus",
        other => other,
    }
}

/// Load default models from embedded config.toml.example for a provider.
pub fn load_default_models(provider_id: &str) -> Vec<String> {
    let config_content = include_str!("../../config.toml.example");
    let mut models = Vec::new();

    if let Ok(config) = config_content.parse::<toml::Value>()
        && let Some(providers) = config.get("providers")
    {
        // Map provider id to config.toml.example section key
        // (config uses underscore: "claude_cli", but provider id uses hyphen: "claude-cli")
        let section_key = match provider_id {
            "claude-cli" => "claude_cli",
            "opencode-cli" => "opencode_cli",
            "qwen-code-cli" => "qwen_code_cli",
            "" => "custom", // empty id = custom providers
            other => other,
        };

        if section_key == "custom" {
            // Custom providers: merge models from all custom sections
            if let Some(custom) = providers.get("custom")
                && let Some(custom_table) = custom.as_table()
            {
                for (_name, entry) in custom_table {
                    if let Some(models_arr) = entry.get("models").and_then(|m| m.as_array()) {
                        for model in models_arr {
                            if let Some(model_str) = model.as_str()
                                && !models.contains(&model_str.to_string())
                            {
                                models.push(model_str.to_string());
                            }
                        }
                    }
                }
            }
        } else if let Some(section) = providers.get(section_key)
            && let Some(models_arr) = section.get("models").and_then(|m| m.as_array())
        {
            for model in models_arr {
                if let Some(model_str) = model.as_str() {
                    models.push(model_str.to_string());
                }
            }
        }
    }

    tracing::debug!(
        "Loaded {} default models from config.toml.example for provider '{}'",
        models.len(),
        provider_id
    );
    models
}
