use super::wizard::OnboardingWizard;

impl OnboardingWizard {
    /// Reload config_models for the currently selected provider.
    /// Tries config.toml first, falls back to config.toml.example defaults.
    pub(super) fn reload_config_models(&mut self) {
        self.config_models.clear();
        // Try live config first
        if let Ok(config) = crate::config::Config::load() {
            match self.selected_provider {
                4 => {
                    if let Some(p) = &config.providers.minimax
                        && !p.models.is_empty()
                    {
                        self.config_models = p.models.clone();
                        return;
                    }
                }
                5 => {
                    if let Some((_name, p)) = config.providers.active_custom()
                        && !p.models.is_empty()
                    {
                        self.config_models = p.models.clone();
                        return;
                    }
                }
                _ => return,
            }
        }
        // Fall back to embedded config.toml.example
        self.config_models = Self::load_default_models(self.selected_provider);
    }

    /// All model names for the current provider (fetched or config or static fallback)
    pub fn all_model_names(&self) -> Vec<&str> {
        if !self.fetched_models.is_empty() {
            self.fetched_models.iter().map(|s| s.as_str()).collect()
        } else if !self.config_models.is_empty() {
            self.config_models.iter().map(|s| s.as_str()).collect()
        } else {
            self.current_provider().models.to_vec()
        }
    }

    /// Model names filtered by `model_filter` (case-insensitive substring match).
    /// Returns all models when filter is empty.
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

    /// Number of models available after applying the current filter
    pub fn model_count(&self) -> usize {
        self.filtered_model_names().len()
    }

    /// Get the selected model name (resolves through filter)
    pub fn selected_model_name(&self) -> &str {
        let filtered = self.filtered_model_names();
        if let Some(name) = filtered.get(self.selected_model) {
            name
        } else {
            // fallback: first unfiltered model
            self.all_model_names().first().copied().unwrap_or("default")
        }
    }

    /// Whether the current provider supports live model fetching
    pub fn supports_model_fetch(&self) -> bool {
        matches!(self.selected_provider, 0 | 1 | 3) // Anthropic, OpenAI, OpenRouter
    }

    /// Load default models from embedded config.toml.example for MiniMax and Custom
    pub(super) fn load_default_models(provider_index: usize) -> Vec<String> {
        // Parse the embedded config.toml.example to extract default models for a specific provider
        let config_content = include_str!("../../../config.toml.example");
        let mut models = Vec::new();

        if let Ok(config) = config_content.parse::<toml::Value>()
            && let Some(providers) = config.get("providers")
        {
            match provider_index {
                4 => {
                    // Minimax only
                    if let Some(minimax) = providers.get("minimax")
                        && let Some(models_arr) = minimax.get("models").and_then(|m| m.as_array())
                    {
                        for model in models_arr {
                            if let Some(model_str) = model.as_str() {
                                models.push(model_str.to_string());
                            }
                        }
                    }
                }
                5 => {
                    // Custom providers only
                    if let Some(custom) = providers.get("custom")
                        && let Some(custom_table) = custom.as_table()
                    {
                        for (_name, entry) in custom_table {
                            if let Some(models_arr) = entry.get("models").and_then(|m| m.as_array())
                            {
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
                }
                _ => {}
            }
        }

        tracing::debug!(
            "Loaded {} default models from config.toml.example for provider {}",
            models.len(),
            provider_index
        );
        models
    }
}
