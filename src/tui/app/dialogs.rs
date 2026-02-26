//! Dialogs — model selector, onboarding wizard, file/directory pickers.

use super::events::{AppMode, TuiEvent};
use super::onboarding::WizardAction;
use super::*;
use crate::brain::provider::{ContentBlock, LLMRequest};
use anyhow::Result;
use std::path::PathBuf;

/// Sentinel for model selector - same as onboarding
const MODEL_SELECTOR_EXISTING_KEY_SENTINEL: &str = "__EXISTING_KEY__";

impl App {
    /// Detect existing API key for the currently selected provider in model selector.
    /// Uses sentinel pattern - doesn't load actual key into memory.
    pub(crate) fn detect_model_selector_key_for_provider(&mut self) {
        let provider_idx = self.model_selector_provider_selected;

        if let Ok(config) = crate::config::Config::load() {
            let has_key = match provider_idx {
                0 => config
                    .providers
                    .anthropic
                    .as_ref()
                    .is_some_and(|p| p.api_key.as_ref().is_some_and(|k| !k.is_empty())),
                1 => config
                    .providers
                    .openai
                    .as_ref()
                    .is_some_and(|p| p.api_key.as_ref().is_some_and(|k| !k.is_empty())),
                2 => config
                    .providers
                    .gemini
                    .as_ref()
                    .is_some_and(|p| p.api_key.as_ref().is_some_and(|k| !k.is_empty())),
                3 => config
                    .providers
                    .openrouter
                    .as_ref()
                    .is_some_and(|p| p.api_key.as_ref().is_some_and(|k| !k.is_empty())),
                4 => config
                    .providers
                    .minimax
                    .as_ref()
                    .is_some_and(|p| p.api_key.as_ref().is_some_and(|k| !k.is_empty())),
                5 => {
                    // Custom provider - also load base_url, model, and name
                    if let Some((name, c)) = config.providers.active_custom() {
                        self.model_selector_custom_name = name.to_string();
                        self.model_selector_base_url = c.base_url.clone().unwrap_or_default();
                        self.model_selector_custom_model =
                            c.default_model.clone().unwrap_or_default();
                        if c.api_key.as_ref().is_some_and(|k| !k.is_empty()) {
                            self.model_selector_api_key =
                                MODEL_SELECTOR_EXISTING_KEY_SENTINEL.to_string();
                        }
                        c.base_url.as_ref().is_some_and(|u| !u.is_empty())
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if provider_idx != 5 {
                if has_key {
                    self.model_selector_api_key = MODEL_SELECTOR_EXISTING_KEY_SENTINEL.to_string();
                } else {
                    self.model_selector_api_key.clear();
                }
            }
        }

        // Clear model selection when provider changes
        self.model_selector_selected = 0;
        self.model_selector_filter.clear();
    }

    /// Open the model selector dialog - load from config and fetch models
    pub(crate) async fn open_model_selector(&mut self) {
        tracing::debug!("[open_model_selector] Opening model selector");

        // Load config to get enabled provider
        let config = crate::config::Config::load().unwrap_or_default();

        // Determine which provider is enabled
        // Indices: 0=Anthropic, 1=OpenAI, 2=Gemini, 3=OpenRouter, 4=Minimax, 5=Custom
        let (provider_idx, api_key) = if config
            .providers
            .anthropic
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            tracing::debug!("[open_model_selector] Anthropic enabled");
            (
                0,
                config
                    .providers
                    .anthropic
                    .as_ref()
                    .and_then(|p| p.api_key.clone()),
            )
        } else if config.providers.openai.as_ref().is_some_and(|p| p.enabled) {
            if let Some(base_url) = config
                .providers
                .openai
                .as_ref()
                .and_then(|p| p.base_url.as_ref())
            {
                if base_url.contains("openrouter") {
                    tracing::debug!("[open_model_selector] OpenAI (OpenRouter) enabled");
                    (
                        3,
                        config
                            .providers
                            .openai
                            .as_ref()
                            .and_then(|p| p.api_key.clone()),
                    )
                } else if base_url.contains("minimax") {
                    tracing::debug!("[open_model_selector] OpenAI (MiniMax) enabled");
                    (
                        4,
                        config
                            .providers
                            .openai
                            .as_ref()
                            .and_then(|p| p.api_key.clone()),
                    )
                } else {
                    tracing::debug!(
                        "[open_model_selector] OpenAI (Custom) enabled, base_url={}",
                        base_url
                    );
                    (
                        5,
                        config
                            .providers
                            .openai
                            .as_ref()
                            .and_then(|p| p.api_key.clone()),
                    )
                }
            } else {
                tracing::debug!("[open_model_selector] OpenAI enabled");
                (
                    1,
                    config
                        .providers
                        .openai
                        .as_ref()
                        .and_then(|p| p.api_key.clone()),
                )
            }
        } else if config.providers.gemini.as_ref().is_some_and(|p| p.enabled) {
            tracing::debug!("[open_model_selector] Gemini enabled");
            (
                2,
                config
                    .providers
                    .gemini
                    .as_ref()
                    .and_then(|p| p.api_key.clone()),
            )
        } else if config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled)
        {
            tracing::debug!("[open_model_selector] OpenRouter enabled");
            (
                3,
                config
                    .providers
                    .openrouter
                    .as_ref()
                    .and_then(|p| p.api_key.clone()),
            )
        } else if config.providers.minimax.as_ref().is_some_and(|p| p.enabled) {
            tracing::debug!("[open_model_selector] MiniMax enabled");
            (
                4,
                config
                    .providers
                    .minimax
                    .as_ref()
                    .and_then(|p| p.api_key.clone()),
            )
        } else if let Some((name, custom_cfg)) = config.providers.active_custom() {
            tracing::debug!("[open_model_selector] Custom provider '{}' enabled", name);
            if let Some(base_url) = &custom_cfg.base_url {
                self.model_selector_base_url = base_url.clone();
            }
            // Load existing model name so /models doesn't lose it
            self.model_selector_custom_model =
                custom_cfg.default_model.clone().unwrap_or_default();
            // Remember the custom provider name for saving
            self.model_selector_custom_name = name.to_string();
            (5, custom_cfg.api_key.clone())
        } else {
            tracing::debug!("[open_model_selector] No provider enabled, defaulting to Anthropic");
            (0, None) // Default
        };

        tracing::debug!(
            "[open_model_selector] provider_idx={}, has_api_key={}",
            provider_idx,
            api_key.is_some()
        );

        self.model_selector_provider_selected = provider_idx;

        // Use sentinel pattern - don't load actual key into memory, just signal "key exists"
        // The actual key stays in keys.toml, we only need to know if it exists
        if api_key.is_some() {
            self.model_selector_api_key = MODEL_SELECTOR_EXISTING_KEY_SENTINEL.to_string();
        } else {
            self.model_selector_api_key.clear();
        }

        // Fetch models from enabled provider using config's API key
        tracing::debug!(
            "[open_model_selector] Fetching models for provider_idx={}",
            provider_idx
        );
        self.model_selector_models =
            super::onboarding::fetch_provider_models(provider_idx, api_key.as_deref()).await;
        tracing::debug!(
            "[open_model_selector] Fetched {} models",
            self.model_selector_models.len()
        );

        // Pre-select current model from config
        let current = config
            .providers
            .openai
            .as_ref()
            .and_then(|p| p.default_model.as_deref())
            .or_else(|| {
                config
                    .providers
                    .anthropic
                    .as_ref()
                    .and_then(|p| p.default_model.as_deref())
            })
            .or_else(|| {
                config
                    .providers
                    .gemini
                    .as_ref()
                    .and_then(|p| p.default_model.as_deref())
            })
            .or_else(|| {
                config
                    .providers
                    .openrouter
                    .as_ref()
                    .and_then(|p| p.default_model.as_deref())
            })
            .or_else(|| {
                config
                    .providers
                    .minimax
                    .as_ref()
                    .and_then(|p| p.default_model.as_deref())
            })
            .or_else(|| {
                config
                    .providers
                    .active_custom()
                    .and_then(|(_, p)| p.default_model.as_deref())
            })
            .unwrap_or("default")
            .to_string();

        tracing::debug!(
            "[open_model_selector] Current model from config: {}",
            current
        );

        self.model_selector_selected = self
            .model_selector_models
            .iter()
            .position(|m| m == &current)
            .unwrap_or(0);

        // Reset view state
        self.model_selector_showing_providers = false;
        self.model_selector_filter.clear();
        self.model_selector_focused_field = 0;

        self.mode = AppMode::ModelSelector;
    }

    /// Handle keys in model selector mode
    pub(crate) async fn handle_model_selector_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;
        use super::onboarding::PROVIDERS;

        if keys::is_cancel(&event) {
            self.switch_mode(AppMode::Chat).await?;
        } else if event.code == crossterm::event::KeyCode::Tab {
            // Tab cycles through fields:
            // - Normal providers: provider(0) -> api_key(1) -> model(2) -> provider(0)
            // - Custom provider: provider(0) -> base_url(1) -> api_key(2) -> model(3) -> provider(0)
            let is_custom = self.model_selector_provider_selected == 5; // Custom provider index
            let max_field = if is_custom { 4 } else { 3 };
            self.model_selector_focused_field = (self.model_selector_focused_field + 1) % max_field;
            // If moving to provider, enable provider list; otherwise show model list
            self.model_selector_showing_providers = self.model_selector_focused_field == 0;
        } else if self.model_selector_focused_field == 0 {
            // Provider selection (focused)
            let provider_changed = match event.code {
                crossterm::event::KeyCode::Up => {
                    self.model_selector_provider_selected =
                        self.model_selector_provider_selected.saturating_sub(1);
                    true
                }
                crossterm::event::KeyCode::Down => {
                    self.model_selector_provider_selected =
                        (self.model_selector_provider_selected + 1).min(PROVIDERS.len() - 1);
                    true
                }
                _ => false,
            };

            // If provider changed, detect existing key for the new provider
            if provider_changed {
                self.detect_model_selector_key_for_provider();
            }
        } else if self.model_selector_focused_field == 1
            && self.model_selector_provider_selected == 5
        {
            // Base URL input for Custom provider (field 1)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    self.model_selector_base_url.push(c);
                }
                crossterm::event::KeyCode::Backspace => {
                    self.model_selector_base_url.pop();
                }
                _ => {}
            }
        } else if (self.model_selector_focused_field == 1
            && self.model_selector_provider_selected != 5)
            || (self.model_selector_focused_field == 2
                && self.model_selector_provider_selected == 5)
        {
            // API key input (field 1 for non-Custom, field 2 for Custom)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    self.model_selector_api_key.push(c);
                }
                crossterm::event::KeyCode::Backspace => {
                    self.model_selector_api_key.pop();
                }
                _ => {}
            }
        } else if self.model_selector_focused_field == 3
            && self.model_selector_provider_selected == 5
        {
            // Custom provider: free-text model name input (field 3)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    self.model_selector_custom_model.push(c);
                }
                crossterm::event::KeyCode::Backspace => {
                    self.model_selector_custom_model.pop();
                }
                _ => {}
            }
        } else if self.model_selector_focused_field == 2
            && self.model_selector_provider_selected != 5
        {
            // Non-custom: filter/search model list (field 2)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    // Type to filter models
                    self.model_selector_filter.push(c);
                    self.model_selector_selected = 0;
                }
                crossterm::event::KeyCode::Backspace => {
                    self.model_selector_filter.pop();
                    // Keep selection valid after filter change
                    let filter = self.model_selector_filter.to_lowercase();
                    let count = if self.model_selector_models.is_empty() {
                        PROVIDERS[self.model_selector_provider_selected]
                            .models
                            .len()
                    } else {
                        self.model_selector_models
                            .iter()
                            .filter(|m| m.to_lowercase().contains(&filter))
                            .count()
                    };
                    if self.model_selector_selected >= count && count > 0 {
                        self.model_selector_selected = count - 1;
                    }
                }
                crossterm::event::KeyCode::Esc => {
                    // Clear filter on Escape
                    self.model_selector_filter.clear();
                    self.model_selector_selected = 0;
                }
                _ => {
                    if keys::is_up(&event) {
                        self.model_selector_selected =
                            self.model_selector_selected.saturating_sub(1);
                    } else if keys::is_down(&event) {
                        // Get filtered count
                        let filter = self.model_selector_filter.to_lowercase();
                        let max_models = if self.model_selector_models.is_empty() {
                            PROVIDERS[self.model_selector_provider_selected]
                                .models
                                .len()
                        } else {
                            self.model_selector_models
                                .iter()
                                .filter(|m| m.to_lowercase().contains(&filter))
                                .count()
                        };
                        if max_models > 0 {
                            self.model_selector_selected =
                                (self.model_selector_selected + 1).min(max_models - 1);
                        }
                    }
                }
            }
        }

        // Enter to confirm - move to next field
        if keys::is_enter(&event) {
            let is_custom = self.model_selector_provider_selected == 5;

            if self.model_selector_focused_field == 0 {
                // On provider field - save config, DON'T close dialog
                if let Err(e) = self
                    .save_provider_selection_internal(
                        self.model_selector_provider_selected,
                        false,
                        false,
                    )
                    .await
                {
                    self.push_system_message(format!("Error: {}", e));
                } else {
                    self.model_selector_focused_field = 1;
                }
            } else if self.model_selector_focused_field == 1 && is_custom {
                // Custom provider: field 1 is base_url, move to field 2 (api_key)
                self.model_selector_focused_field = 2;
            } else if (self.model_selector_focused_field == 1 && !is_custom)
                || (self.model_selector_focused_field == 2 && is_custom)
            {
                // On API key field (field 1 for non-Custom, field 2 for Custom)
                let provider_idx = self.model_selector_provider_selected;

                // Check if sentinel (existing key) or user-typed key
                let is_existing_key =
                    self.model_selector_api_key == MODEL_SELECTOR_EXISTING_KEY_SENTINEL;
                let api_key = if is_existing_key {
                    // Don't save anything - key already exists in config
                    None
                } else if self.model_selector_api_key.is_empty() {
                    None
                } else {
                    Some(self.model_selector_api_key.clone())
                };

                // Skip saving if key already exists (sentinel) - no need to update
                let key_changed = !is_existing_key && !self.model_selector_api_key.is_empty();

                // Save provider config - DON'T close
                if let Err(e) = self
                    .save_provider_selection_internal(provider_idx, false, key_changed)
                    .await
                {
                    self.push_system_message(format!("Error: {}", e));
                } else {
                    // Fetch live models from the provider (for non-Custom)
                    if !is_custom {
                        self.model_selector_models = super::onboarding::fetch_provider_models(
                            provider_idx,
                            api_key.as_deref(),
                        )
                        .await;
                    }
                    self.model_selector_selected = 0;

                    // Move to model selection field (field 2 for non-Custom, field 3 for Custom)
                    self.model_selector_focused_field = if is_custom { 3 } else { 2 };
                }
            } else {
                // On model field - save and close (this one CAN close)
                self.save_provider_selection(self.model_selector_provider_selected, true)
                    .await?;
            }
        }

        Ok(())
    }

    /// Save provider selection to config and reload agent service
    /// If `close_dialog` is false, stays in model selector (for step 1 and 2)
    async fn save_provider_selection(
        &mut self,
        provider_idx: usize,
        key_changed: bool,
    ) -> Result<()> {
        self.save_provider_selection_internal(provider_idx, true, key_changed)
            .await
    }

    /// Internal: save provider with option to close dialog
    /// `key_changed` - true if user typed a new key (needs to be saved)
    async fn save_provider_selection_internal(
        &mut self,
        provider_idx: usize,
        close_dialog: bool,
        key_changed: bool,
    ) -> Result<()> {
        use super::onboarding::PROVIDERS;
        use crate::config::ProviderConfig;

        let provider = &PROVIDERS[provider_idx];

        // Load existing config to merge
        let mut config = crate::config::Config::load().unwrap_or_default();

        // Disable all providers first - we'll enable only the selected one
        if let Some(ref mut p) = config.providers.anthropic {
            p.enabled = false;
        }
        if let Some(ref mut p) = config.providers.openai {
            p.enabled = false;
        }
        if let Some(ref mut p) = config.providers.gemini {
            p.enabled = false;
        }
        if let Some(ref mut p) = config.providers.openrouter {
            p.enabled = false;
        }
        if let Some(ref mut p) = config.providers.minimax {
            p.enabled = false;
        }

        // Get existing key from config if not changing
        let existing_key = match provider_idx {
            0 => config
                .providers
                .anthropic
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            1 => config
                .providers
                .openai
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            2 => config
                .providers
                .gemini
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            3 => config
                .providers
                .openrouter
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            4 => config
                .providers
                .minimax
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            5 => config
                .providers
                .active_custom()
                .and_then(|(_, p)| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            _ => None,
        };

        // Use existing key if not changed, otherwise use user input
        let api_key = if key_changed {
            if self.model_selector_api_key.is_empty() {
                None
            } else {
                Some(self.model_selector_api_key.clone())
            }
        } else {
            existing_key
        };

        // Log what's being saved (hide key)
        tracing::info!(
            "Saving provider config: idx={}, has_api_key={}",
            provider_idx,
            api_key.is_some()
        );

        // Build provider config based on selection
        let default_model = provider.models.first().copied().unwrap_or("default");
        match provider_idx {
            0 => {
                // Anthropic
                config.providers.anthropic = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                });
            }
            1 => {
                // OpenAI
                config.providers.openai = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                });
            }
            2 => {
                // Gemini
                config.providers.gemini = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                });
            }
            3 => {
                // OpenRouter
                config.providers.openrouter = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: Some("https://openrouter.ai/api/v1/chat/completions".to_string()),
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                });
            }
            4 => {
                // Minimax
                config.providers.minimax = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: Some("https://api.minimax.io/v1".to_string()),
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                });
            }
            5 => {
                // Custom OpenAI-compatible (named provider)
                let custom_model = self.model_selector_custom_model.clone();
                let custom_name = if self.model_selector_custom_name.is_empty() {
                    "default".to_string()
                } else {
                    self.model_selector_custom_name.clone()
                };
                let mut customs = config.providers.custom.unwrap_or_default();
                customs.insert(
                    custom_name,
                    ProviderConfig {
                        enabled: true,
                        api_key: api_key.clone(),
                        base_url: Some(self.model_selector_base_url.clone()),
                        default_model: Some(custom_model),
                        models: vec![],
                    },
                );
                config.providers.custom = Some(customs);
            }
            _ => {}
        }

        // Save provider config via merge (write_key) — never overwrite entire config.toml
        let custom_section;
        let section = match provider_idx {
            0 => "providers.anthropic",
            1 => "providers.openai",
            2 => "providers.gemini",
            3 => "providers.openrouter",
            4 => "providers.minimax",
            5 => {
                let name = if self.model_selector_custom_name.is_empty() {
                    "default"
                } else {
                    &self.model_selector_custom_name
                };
                custom_section = format!("providers.custom.{}", name);
                &custom_section
            }
            _ => {
                custom_section = "providers.custom.default".to_string();
                &custom_section
            }
        };

        if let Err(e) = crate::config::Config::write_key(section, "enabled", "true") {
            tracing::warn!("Failed to write {}.enabled: {}", section, e);
        }

        // Write base_url if applicable
        match provider_idx {
            3 => {
                let _ = crate::config::Config::write_key(
                    section,
                    "base_url",
                    "https://openrouter.ai/api/v1/chat/completions",
                );
            }
            4 => {
                let _ = crate::config::Config::write_key(
                    section,
                    "base_url",
                    "https://api.minimax.io/v1",
                );
            }
            5 => {
                if !self.model_selector_base_url.is_empty() {
                    let _ = crate::config::Config::write_key(
                        section,
                        "base_url",
                        &self.model_selector_base_url,
                    );
                }
            }
            _ => {}
        }

        // Save API key to keys.toml via merge
        if let Some(ref key) = api_key
            && !key.is_empty()
            && let Err(e) = crate::config::write_secret_key(section, "api_key", key)
        {
            tracing::warn!("Failed to save API key to keys.toml: {}", e);
        }

        // Rebuild agent service with new provider
        if let Err(e) = self.rebuild_agent_service().await {
            // If rebuild fails, check if it's due to missing API key
            if api_key.is_none() && provider_idx == 5 {
                // Need API key - show message and stay in provider mode
                self.push_system_message(format!(
                    "API key required for {}. Type it and press Enter.",
                    provider
                        .name
                        .split('(')
                        .next()
                        .unwrap_or(provider.name)
                        .trim()
                ));
                return Ok(());
            }
            return Err(e);
        }

        // Get the selected model
        let is_custom = provider_idx == 5;
        let selected_model = if is_custom {
            // Custom provider: use free-text model name — must be set
            self.model_selector_custom_model.clone()
        } else if !self.model_selector_models.is_empty() {
            // Non-custom: use filtered display list to get actual model name
            let filter = self.model_selector_filter.to_lowercase();
            let filtered: Vec<_> = self
                .model_selector_models
                .iter()
                .filter(|m| m.to_lowercase().contains(&filter))
                .collect();
            if let Some(model) = filtered.get(self.model_selector_selected) {
                model.to_string()
            } else {
                self.model_selector_models.first().cloned().unwrap_or_else(
                    || self.default_model_name.clone(),
                )
            }
        } else if let Some(model) = provider.models.get(self.model_selector_selected) {
            model.to_string()
        } else if let Some(model) = provider.models.first() {
            model.to_string()
        } else {
            self.default_model_name.clone()
        };

        // Save the model to config
        let custom_section2;
        let section = match provider_idx {
            0 => "providers.anthropic",
            1 => "providers.openai",
            2 => "providers.gemini",
            3 => "providers.openrouter",
            4 => "providers.minimax",
            5 => {
                let name = if self.model_selector_custom_name.is_empty() {
                    "default"
                } else {
                    &self.model_selector_custom_name
                };
                custom_section2 = format!("providers.custom.{}", name);
                &custom_section2
            }
            _ => "providers.anthropic",
        };

        if let Err(e) = crate::config::Config::write_key(section, "default_model", &selected_model)
        {
            tracing::warn!("Failed to persist model to config: {}", e);
        }

        // Update app state
        self.default_model_name = selected_model.clone();

        // Only close dialog if explicitly requested
        if close_dialog {
            let provider_name = provider
                .name
                .split('(')
                .next()
                .unwrap_or(provider.name)
                .trim();
            self.push_system_message(format!(
                "Provider: {}, Model: {}",
                provider_name, selected_model
            ));
            self.mode = AppMode::Chat;
        }

        Ok(())
    }

    /// Handle keys in onboarding wizard mode
    pub(crate) async fn handle_onboarding_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        if let Some(ref mut wizard) = self.onboarding {
            let action = wizard.handle_key(event);
            match action {
                WizardAction::Cancel => {
                    self.onboarding = None;
                    self.switch_mode(AppMode::Chat).await?;
                }
                WizardAction::Complete => {
                    // Apply wizard config before transitioning
                    if let Some(ref wizard) = self.onboarding {
                        match wizard.apply_config() {
                            Ok(()) => {
                                let (provider_name, model_name) =
                                    if wizard.is_custom_provider() {
                                        (
                                            format!(
                                                "Custom ({})",
                                                wizard.custom_provider_name
                                            ),
                                            wizard.custom_model.clone(),
                                        )
                                    } else {
                                        (
                                            super::onboarding::PROVIDERS
                                                [wizard.selected_provider]
                                                .name
                                                .to_string(),
                                            wizard.selected_model_name().to_string(),
                                        )
                                    };
                                self.push_system_message(format!(
                                    "Setup complete! Provider: {} | Model: {}",
                                    provider_name, model_name
                                ));
                                // Rebuild agent service with new provider
                                if let Err(e) = self.rebuild_agent_service().await {
                                    tracing::warn!("Failed to rebuild agent service: {}", e);
                                    self.push_system_message(format!(
                                        "Warning: Failed to reload provider: {}",
                                        e
                                    ));
                                }
                            }
                            Err(e) => {
                                self.push_system_message(format!(
                                    "Setup finished with warnings: {}",
                                    e
                                ));
                            }
                        }
                    }
                    self.onboarding = None;
                    self.switch_mode(AppMode::Chat).await?;
                }
                WizardAction::FetchModels => {
                    let provider_idx = wizard.selected_provider;
                    // Resolve API key from config (keys.toml) or raw input
                    let api_key = if wizard.has_existing_key() {
                        let provider_name = super::onboarding::PROVIDERS[provider_idx].name;
                        let loaded = crate::config::Config::load().ok();
                        match provider_name {
                            "Anthropic Claude" => loaded
                                .as_ref()
                                .and_then(|c| c.providers.anthropic.as_ref())
                                .and_then(|p| p.api_key.clone()),
                            "OpenAI" => loaded
                                .as_ref()
                                .and_then(|c| c.providers.openai.as_ref())
                                .and_then(|p| p.api_key.clone()),
                            "Google Gemini" => loaded
                                .as_ref()
                                .and_then(|c| c.providers.gemini.as_ref())
                                .and_then(|p| p.api_key.clone()),
                            "OpenRouter" => loaded
                                .as_ref()
                                .and_then(|c| c.providers.openrouter.as_ref())
                                .and_then(|p| p.api_key.clone()),
                            "Minimax" => loaded
                                .as_ref()
                                .and_then(|c| c.providers.minimax.as_ref())
                                .and_then(|p| p.api_key.clone()),
                            _ => None,
                        }
                    } else if !wizard.api_key_input.is_empty() {
                        Some(wizard.api_key_input.clone())
                    } else {
                        None
                    };
                    wizard.models_fetching = true;

                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        let models = super::onboarding::fetch_provider_models(
                            provider_idx,
                            api_key.as_deref(),
                        )
                        .await;
                        let _ = sender.send(TuiEvent::OnboardingModelsFetched(models));
                    });
                }
                WizardAction::WhatsAppConnect => {
                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        // Timeout the pairing setup itself (may hang if session DB is locked)
                        let pairing_result = tokio::time::timeout(
                            std::time::Duration::from_secs(15),
                            crate::brain::tools::whatsapp_connect::start_whatsapp_pairing(),
                        )
                        .await;

                        match pairing_result {
                            Ok(Ok(handle)) => {
                                // Forward QR codes to the TUI
                                let qr_sender = sender.clone();
                                let mut qr_rx = handle.qr_rx;
                                tokio::spawn(async move {
                                    while let Some(qr) = qr_rx.recv().await {
                                        let _ = qr_sender.send(TuiEvent::WhatsAppQrCode(qr));
                                    }
                                });
                                // Wait for connection (2 minute timeout)
                                match tokio::time::timeout(
                                    std::time::Duration::from_secs(120),
                                    handle.connected_rx,
                                )
                                .await
                                {
                                    Ok(Ok(())) => {
                                        let _ = sender.send(TuiEvent::WhatsAppConnected);
                                    }
                                    Ok(Err(_)) => {
                                        let _ = sender.send(TuiEvent::WhatsAppError(
                                            "Connection channel closed unexpectedly".into(),
                                        ));
                                    }
                                    Err(_) => {
                                        let _ = sender.send(TuiEvent::WhatsAppError(
                                            "Connection timed out (2 minutes)".into(),
                                        ));
                                    }
                                }
                            }
                            Ok(Err(e)) => {
                                let _ = sender.send(TuiEvent::WhatsAppError(e.to_string()));
                            }
                            Err(_) => {
                                let _ = sender.send(TuiEvent::WhatsAppError(
                                    "WhatsApp bridge startup timed out. Is another instance running?".into(),
                                ));
                            }
                        }
                    });
                }
                WizardAction::TestTelegram => {
                    wizard.channel_test_status = super::onboarding::ChannelTestStatus::Testing;
                    let token = if wizard.has_existing_telegram_token() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.telegram.token.clone())
                            .unwrap_or_default()
                    } else {
                        wizard.telegram_token_input.clone()
                    };
                    let user_id_str = if wizard.has_existing_telegram_user_id() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.telegram.allowed_users.first().copied())
                            .map(|id| id.to_string())
                            .unwrap_or_default()
                    } else {
                        wizard.telegram_user_id_input.clone()
                    };
                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        let result = test_telegram_connection(&token, &user_id_str).await;
                        let _ = sender.send(TuiEvent::ChannelTestResult {
                            channel: "telegram".to_string(),
                            success: result.is_ok(),
                            error: result.err(),
                        });
                    });
                }
                WizardAction::TestDiscord => {
                    wizard.channel_test_status = super::onboarding::ChannelTestStatus::Testing;
                    let token = if wizard.has_existing_discord_token() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.discord.token.clone())
                            .unwrap_or_default()
                    } else {
                        wizard.discord_token_input.clone()
                    };
                    let channel_id = if wizard.has_existing_discord_channel_id() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.discord.allowed_channels.first().cloned())
                            .unwrap_or_default()
                    } else {
                        wizard.discord_channel_id_input.clone()
                    };
                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        let result = test_discord_connection(&token, &channel_id).await;
                        let _ = sender.send(TuiEvent::ChannelTestResult {
                            channel: "discord".to_string(),
                            success: result.is_ok(),
                            error: result.err(),
                        });
                    });
                }
                WizardAction::TestSlack => {
                    wizard.channel_test_status = super::onboarding::ChannelTestStatus::Testing;
                    let token = if wizard.has_existing_slack_bot_token() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.slack.token.clone())
                            .unwrap_or_default()
                    } else {
                        wizard.slack_bot_token_input.clone()
                    };
                    let channel_id = if wizard.has_existing_slack_channel_id() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.slack.allowed_channels.first().cloned())
                            .unwrap_or_default()
                    } else {
                        wizard.slack_channel_id_input.clone()
                    };
                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        let result = test_slack_connection(&token, &channel_id).await;
                        let _ = sender.send(TuiEvent::ChannelTestResult {
                            channel: "slack".to_string(),
                            success: result.is_ok(),
                            error: result.err(),
                        });
                    });
                }
                WizardAction::GenerateBrain => {
                    self.generate_brain_files().await;
                }
                WizardAction::None => {
                    // Stay in onboarding
                }
            }
        }
        Ok(())
    }

    /// Generate personalized brain files via the AI provider
    async fn generate_brain_files(&mut self) {
        // Extract what we need before borrowing wizard mutably
        let prompt = {
            let Some(ref wizard) = self.onboarding else {
                return;
            };
            wizard.build_brain_prompt()
        };

        // Mark as generating
        if let Some(ref mut wizard) = self.onboarding {
            wizard.brain_generating = true;
            wizard.brain_error = None;
        }

        // Get provider and model from the wizard's selected provider
        let provider = self.agent_service.provider().clone();
        let model = self.agent_service.provider_model().to_string();

        // Build LLM request
        let request = LLMRequest::new(model, vec![crate::brain::provider::Message::user(prompt)])
            .with_max_tokens(65536);

        // Call the provider
        match provider.complete(request).await {
            Ok(response) => {
                // Extract text from response
                let text: String = response
                    .content
                    .iter()
                    .filter_map(|block| {
                        if let ContentBlock::Text { text } = block {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();

                if let Some(ref mut wizard) = self.onboarding {
                    wizard.apply_generated_brain(&text);
                    // Auto-advance to Complete if generation succeeded
                    if wizard.brain_generated {
                        wizard.step = super::onboarding::OnboardingStep::Complete;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Brain generation failed: {}", e);
                if let Some(ref mut wizard) = self.onboarding {
                    wizard.brain_generating = false;
                    wizard.brain_error = Some(format!("Generation failed: {}", e));
                }
            }
        }
    }

    /// Open file picker and populate file list
    pub(crate) async fn open_file_picker(&mut self) -> Result<()> {
        // Get list of files in current directory
        let mut files = Vec::new();

        // Add parent directory option if not at root
        if self.file_picker_current_dir.parent().is_some() {
            files.push(self.file_picker_current_dir.join(".."));
        }

        // Read directory entries
        if let Ok(entries) = std::fs::read_dir(&self.file_picker_current_dir) {
            for entry in entries.flatten() {
                files.push(entry.path());
            }
        }

        // Sort: directories first, then files, alphabetically
        files.sort_by(|a, b| {
            let a_is_dir = a.is_dir();
            let b_is_dir = b.is_dir();
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.file_name().cmp(&b.file_name()),
            }
        });

        self.file_picker_files = files;
        self.file_picker_selected = 0;
        self.file_picker_scroll_offset = 0;
        self.switch_mode(AppMode::FilePicker).await?;

        Ok(())
    }

    /// Handle keys in file picker mode
    pub(crate) async fn handle_file_picker_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;
        use crossterm::event::KeyCode;

        if keys::is_cancel(&event) {
            // Cancel file picker and return to chat
            self.switch_mode(AppMode::Chat).await?;
        } else if keys::is_up(&event) {
            // Move selection up
            self.file_picker_selected = self.file_picker_selected.saturating_sub(1);

            // Adjust scroll offset if needed
            if self.file_picker_selected < self.file_picker_scroll_offset {
                self.file_picker_scroll_offset = self.file_picker_selected;
            }
        } else if keys::is_down(&event) {
            // Move selection down
            if self.file_picker_selected + 1 < self.file_picker_files.len() {
                self.file_picker_selected += 1;

                // Adjust scroll offset if needed (assuming 20 visible items)
                let visible_items = 20;
                if self.file_picker_selected >= self.file_picker_scroll_offset + visible_items {
                    self.file_picker_scroll_offset = self.file_picker_selected - visible_items + 1;
                }
            }
        } else if keys::is_enter(&event) || event.code == KeyCode::Char(' ') || keys::is_tab(&event)
        {
            // Select file or navigate into directory
            if let Some(selected_path) = self.file_picker_files.get(self.file_picker_selected) {
                if selected_path.is_dir() {
                    // Navigate into directory
                    if selected_path.ends_with("..") {
                        // Go to parent directory
                        if let Some(parent) = self.file_picker_current_dir.parent() {
                            self.file_picker_current_dir = parent.to_path_buf();
                        }
                    } else {
                        self.file_picker_current_dir = selected_path.clone();
                    }
                    // Refresh file list
                    self.open_file_picker().await?;
                } else {
                    // Insert file path into input buffer at cursor
                    let path_str = selected_path.to_string_lossy().to_string();
                    self.input_buffer
                        .insert_str(self.cursor_position, &path_str);
                    self.cursor_position += path_str.len();
                    self.switch_mode(AppMode::Chat).await?;
                }
            }
        }

        Ok(())
    }

    /// Open directory picker (reuses file picker state, dirs only)
    pub(crate) async fn open_directory_picker(&mut self) -> Result<()> {
        let mut files = Vec::new();

        // Add parent directory option if not at root
        if self.file_picker_current_dir.parent().is_some() {
            files.push(self.file_picker_current_dir.join(".."));
        }

        // Read directory entries — directories only
        if let Ok(entries) = std::fs::read_dir(&self.file_picker_current_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    files.push(path);
                }
            }
        }

        // Sort alphabetically
        files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        self.file_picker_files = files;
        self.file_picker_selected = 0;
        self.file_picker_scroll_offset = 0;
        self.switch_mode(AppMode::DirectoryPicker).await?;

        Ok(())
    }

    /// Handle keys in directory picker mode
    pub(crate) async fn handle_directory_picker_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;
        use crossterm::event::KeyCode;

        if keys::is_cancel(&event) {
            self.switch_mode(AppMode::Chat).await?;
        } else if keys::is_up(&event) {
            self.file_picker_selected = self.file_picker_selected.saturating_sub(1);
            if self.file_picker_selected < self.file_picker_scroll_offset {
                self.file_picker_scroll_offset = self.file_picker_selected;
            }
        } else if keys::is_down(&event) {
            if self.file_picker_selected + 1 < self.file_picker_files.len() {
                self.file_picker_selected += 1;
                let visible_items = 20;
                if self.file_picker_selected >= self.file_picker_scroll_offset + visible_items {
                    self.file_picker_scroll_offset = self.file_picker_selected - visible_items + 1;
                }
            }
        } else if keys::is_enter(&event) {
            // Enter navigates into directory
            if let Some(selected_path) = self
                .file_picker_files
                .get(self.file_picker_selected)
                .cloned()
            {
                if selected_path.ends_with("..") {
                    if let Some(parent) = self.file_picker_current_dir.parent() {
                        self.file_picker_current_dir = parent.to_path_buf();
                    }
                } else {
                    self.file_picker_current_dir = selected_path;
                }
                self.open_directory_picker().await?;
            }
        } else if event.code == KeyCode::Tab || event.code == KeyCode::Char(' ') {
            // Tab/Space selects the current directory as working dir
            let selected_dir = self.file_picker_current_dir.clone();
            let canonical = selected_dir
                .canonicalize()
                .unwrap_or_else(|_| selected_dir.clone());

            // Update App working directory
            self.working_directory = canonical.clone();

            // Update AgentService working directory (runtime)
            self.agent_service.set_working_directory(canonical.clone());

            // Persist to config.toml
            let _ = crate::config::Config::write_key(
                "agent",
                "working_directory",
                &canonical.to_string_lossy(),
            );

            self.push_system_message(format!(
                "Working directory changed to: {}",
                canonical.display()
            ));

            // Queue context hint so the next message to the LLM knows about the cd
            self.pending_context.push(format!(
                "[User changed working directory to: {}]",
                canonical.display()
            ));

            self.switch_mode(AppMode::Chat).await?;
        }

        Ok(())
    }
}

/// Download WhisperCrabs binary if not cached, return the path to the binary.
pub(crate) async fn ensure_whispercrabs() -> Result<PathBuf> {
    let bin_dir = crate::config::opencrabs_home().join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    let binary_name = if cfg!(target_os = "windows") {
        "whispercrabs.exe"
    } else {
        "whispercrabs"
    };
    let binary_path = bin_dir.join(binary_name);

    if binary_path.exists() {
        return Ok(binary_path);
    }

    // Detect platform
    let (os_name, ext) = match std::env::consts::OS {
        "linux" => ("linux", "tar.gz"),
        "macos" => ("macos", "tar.gz"),
        "windows" => ("windows", "zip"),
        other => anyhow::bail!("Unsupported OS: {}", other),
    };
    let arch = std::env::consts::ARCH; // "x86_64" or "aarch64"

    // Download latest release via GitHub API
    let client = reqwest::Client::new();
    let release_url = "https://api.github.com/repos/adolfousier/whispercrabs/releases/latest";
    let release: serde_json::Value = client
        .get(release_url)
        .header("User-Agent", "opencrabs")
        .send()
        .await?
        .json()
        .await?;

    // Find matching asset
    let pattern = format!("whispercrabs-{}-{}", os_name, arch);
    let asset = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|a| a["name"].as_str().is_some_and(|n| n.contains(&pattern)))
        })
        .ok_or_else(|| anyhow::anyhow!("No release found for {}-{}", os_name, arch))?;

    let download_url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing download URL in release asset"))?;

    // Download the archive
    let bytes = client
        .get(download_url)
        .header("User-Agent", "opencrabs")
        .send()
        .await?
        .bytes()
        .await?;

    // Extract (tar.gz for Linux/macOS, zip for Windows)
    let tmp = bin_dir.join("whispercrabs_download");
    std::fs::write(&tmp, &bytes)?;

    if ext == "tar.gz" {
        let output = tokio::process::Command::new("tar")
            .args([
                "xzf",
                &tmp.to_string_lossy(),
                "-C",
                &bin_dir.to_string_lossy(),
            ])
            .output()
            .await?;
        if !output.status.success() {
            let _ = std::fs::remove_file(&tmp);
            anyhow::bail!("Failed to extract archive");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    // Clean up temp file
    let _ = std::fs::remove_file(&tmp);

    if !binary_path.exists() {
        anyhow::bail!("Binary not found after extraction — archive may use a different layout");
    }

    Ok(binary_path)
}

/// Test Telegram connection by sending a message via the bot API.
#[cfg(feature = "telegram")]
async fn test_telegram_connection(token: &str, user_id_str: &str) -> Result<(), String> {
    use teloxide::prelude::Requester;

    let user_id: i64 = user_id_str
        .parse()
        .map_err(|_| format!("Invalid user ID: {}", user_id_str))?;
    let bot = teloxide::Bot::new(token);
    bot.send_message(
        teloxide::types::ChatId(user_id),
        "OpenCrabs connected! Your Telegram bot is ready.",
    )
    .await
    .map_err(|e| format!("Telegram API error: {}", e))?;
    Ok(())
}

#[cfg(not(feature = "telegram"))]
async fn test_telegram_connection(_token: &str, _user_id_str: &str) -> Result<(), String> {
    Err("Telegram feature not enabled".to_string())
}

/// Test Discord connection by sending a message to a channel.
#[cfg(feature = "discord")]
async fn test_discord_connection(token: &str, channel_id_str: &str) -> Result<(), String> {
    let channel_id: u64 = channel_id_str
        .parse()
        .map_err(|_| format!("Invalid channel ID: {}", channel_id_str))?;
    let http = serenity::http::Http::new(token);
    let channel = serenity::model::id::ChannelId::new(channel_id);
    channel
        .say(&http, "OpenCrabs connected! Your Discord bot is ready.")
        .await
        .map_err(|e| format!("Discord API error: {}", e))?;
    Ok(())
}

#[cfg(not(feature = "discord"))]
async fn test_discord_connection(_token: &str, _channel_id_str: &str) -> Result<(), String> {
    Err("Discord feature not enabled".to_string())
}

/// Test Slack connection by posting a message to a channel.
#[cfg(feature = "slack")]
async fn test_slack_connection(token: &str, channel_id: &str) -> Result<(), String> {
    use slack_morphism::prelude::*;

    let client = SlackClient::new(
        SlackClientHyperConnector::new().map_err(|e| format!("Slack client error: {}", e))?,
    );
    let api_token = SlackApiToken::new(SlackApiTokenValue::from(token.to_string()));
    let session = client.open_session(&api_token);
    let request = SlackApiChatPostMessageRequest::new(
        SlackChannelId::new(channel_id.to_string()),
        SlackMessageContent::new()
            .with_text("OpenCrabs connected! Your Slack bot is ready.".to_string()),
    );
    session
        .chat_post_message(&request)
        .await
        .map_err(|e| format!("Slack API error: {}", e))?;
    Ok(())
}

#[cfg(not(feature = "slack"))]
async fn test_slack_connection(_token: &str, _channel_id: &str) -> Result<(), String> {
    Err("Slack feature not enabled".to_string())
}
