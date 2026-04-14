//! Dialogs — model selector, onboarding wizard, file/directory pickers.

use super::events::{AppMode, TuiEvent};
use super::onboarding::{OnboardingStep, WizardAction};
use super::*;
use crate::brain::provider::{ContentBlock, LLMRequest};
use crate::tui::provider_selector::{CUSTOM_INSTANCES_START, CUSTOM_PROVIDER_IDX};
use anyhow::Result;
use std::path::PathBuf;

impl App {
    /// Spawn the Qwen OAuth device flow task. Shared by `/onboard` (wizard)
    /// and `/models` (dialog). Emits `TuiEvent::QwenDeviceCode` /
    /// `QwenOAuthComplete` / `QwenOAuthError` which both rendering paths
    /// consume via `self.ps.qwen_device_flow_status`.
    pub(crate) fn spawn_qwen_device_flow(&self) {
        use crate::brain::provider::qwen;
        let sender = self.event_sender();
        tokio::spawn(async move {
            // Retry loop: on poll failure (expired code, WAF, etc.), generate
            // a fresh device code and URL automatically. The user can also
            // hit Enter to restart (which spawns a new task, orphaning this one).
            const MAX_RETRIES: u32 = 5;
            for attempt in 0..MAX_RETRIES {
                if attempt > 0 {
                    tracing::info!(
                        "Qwen OAuth: auto-retrying with new device code (attempt {})",
                        attempt + 1
                    );
                }

                let pkce = qwen::PkcePair::generate();
                let device = match qwen::start_device_flow(&pkce).await {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = sender.send(TuiEvent::QwenOAuthError(e.to_string()));
                        // Brief pause before retry
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                };

                let url = device
                    .verification_uri_complete
                    .clone()
                    .unwrap_or_else(|| device.verification_uri.clone());
                let _ = qwen::open_browser(&url);

                let _ = sender.send(TuiEvent::QwenDeviceCode {
                    user_code: device.user_code.clone(),
                    verification_uri: url,
                });

                match qwen::poll_for_token(&device.device_code, &pkce).await {
                    Ok(creds) => {
                        if let Err(e) = creds.persist_to_keys() {
                            let _ = sender.send(TuiEvent::QwenOAuthError(format!(
                                "Authorized but failed to save credentials: {}",
                                e
                            )));
                            return;
                        }
                        let _ = sender.send(TuiEvent::QwenOAuthComplete);
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("Qwen OAuth poll failed: {} — will retry", e);
                        // Don't send error event on non-final attempts
                        if attempt == MAX_RETRIES - 1 {
                            let _ = sender.send(TuiEvent::QwenOAuthError(e.to_string()));
                        }
                    }
                }
            }
        });
    }

    /// Detect existing API key for the currently selected provider in model selector.
    /// Delegates to `ProviderSelectorState::detect_existing_key()`.
    pub(crate) fn detect_model_selector_key_for_provider(&mut self) {
        tracing::debug!(
            "[detect_key] provider_idx={}, custom_names={:?}",
            self.ps.selected_provider,
            self.ps.custom_names,
        );
        self.ps.detect_existing_key();
    }

    /// Open the model selector dialog - load from config and fetch models
    pub(crate) async fn open_model_selector(&mut self) {
        tracing::debug!("[open_model_selector] Opening model selector");

        // Load config to get enabled provider
        let config = match crate::config::Config::load() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to load config for model selector: {}", e);
                self.push_system_message(format!(
                    "⚠️ Could not load config.toml: {}. Model selector unavailable.",
                    e
                ));
                return;
            }
        };

        // Cache existing custom provider names early (needed for index mapping)
        self.ps.custom_names = config
            .providers
            .custom
            .as_ref()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();

        // Try session's provider first, then fall back to config
        let session_provider = self
            .current_session
            .as_ref()
            .and_then(|s| s.provider_name.as_deref());

        // Resolve provider index + API key from session provider name
        let from_session: Option<(usize, Option<String>)> = session_provider.map(|name| {
            use crate::utils::providers;
            // Try known provider first
            if let Some(idx) = providers::tui_index_for_id(name) {
                let api_key =
                    providers::config_for(&config.providers, name).and_then(|p| p.api_key.clone());
                (idx, api_key)
            } else {
                // Custom provider (e.g. "nvidia", "ollama")
                let api_key = config
                    .providers
                    .custom_by_name(name)
                    .and_then(|p| p.api_key.clone());
                if let Some(c) = config.providers.custom_by_name(name) {
                    self.ps.base_url = c.base_url.clone().unwrap_or_default();
                    self.ps.custom_model = c.default_model.clone().unwrap_or_default();
                    self.ps.context_window = c
                        .context_window
                        .map(|cw| cw.to_string())
                        .unwrap_or_default();
                    self.ps.custom_name = name.to_string();
                }
                let idx = self
                    .ps
                    .custom_names
                    .iter()
                    .position(|n| n == name)
                    .map(|pos| CUSTOM_INSTANCES_START + pos)
                    .unwrap_or(CUSTOM_PROVIDER_IDX);
                (idx, api_key)
            }
        });

        // Determine which provider is enabled — iterate PROVIDERS using shared utility
        let (provider_idx, api_key) = if let Some(resolved) = from_session {
            tracing::debug!("[open_model_selector] From session: {:?}", session_provider);
            resolved
        } else {
            use crate::tui::onboarding::PROVIDERS;
            use crate::utils::providers as prov;

            let mut found: Option<(usize, Option<String>)> = None;

            // Check known providers by iterating PROVIDERS (skip custom sentinel)
            for (idx, info) in PROVIDERS.iter().enumerate().take(CUSTOM_PROVIDER_IDX) {
                if let Some(cfg) = prov::config_for(&config.providers, info.id)
                    && cfg.enabled
                {
                    // Special case: OpenAI with custom base_url may actually be another provider
                    if info.id == "openai"
                        && let Some(base_url) = &cfg.base_url
                    {
                        if base_url.contains("openrouter") {
                            if let Some(oi) = prov::tui_index_for_id("openrouter") {
                                found = Some((oi, cfg.api_key.clone()));
                            }
                        } else if base_url.contains("minimax") {
                            if let Some(mi) = prov::tui_index_for_id("minimax") {
                                found = Some((mi, cfg.api_key.clone()));
                            }
                        } else {
                            found = Some((CUSTOM_PROVIDER_IDX, cfg.api_key.clone())); // custom-like
                        }
                        break;
                    }
                    tracing::debug!("[open_model_selector] {} enabled", info.name);
                    found = Some((idx, cfg.api_key.clone()));
                    break;
                }
            }

            // Check custom providers if no known provider is enabled
            if found.is_none()
                && let Some((name, custom_cfg)) = config.providers.active_custom()
            {
                tracing::debug!("[open_model_selector] Custom provider '{}' enabled", name);
                if let Some(base_url) = &custom_cfg.base_url {
                    self.ps.base_url = base_url.clone();
                }
                self.ps.custom_model = custom_cfg.default_model.clone().unwrap_or_default();
                self.ps.context_window = custom_cfg
                    .context_window
                    .map(|cw| cw.to_string())
                    .unwrap_or_default();
                self.ps.custom_name = name.to_string();
                let idx = self
                    .ps
                    .custom_names
                    .iter()
                    .position(|n| n == name)
                    .map(|pos| CUSTOM_INSTANCES_START + pos)
                    .unwrap_or(CUSTOM_PROVIDER_IDX);
                found = Some((idx, custom_cfg.api_key.clone()));
            }

            found.unwrap_or_else(|| {
                tracing::debug!(
                    "[open_model_selector] No provider enabled, defaulting to Anthropic"
                );
                (0, None)
            })
        };

        tracing::debug!(
            "[open_model_selector] provider_idx={}, has_api_key={}",
            provider_idx,
            api_key.is_some()
        );

        self.ps.selected_provider = provider_idx;

        tracing::info!(
            "[open_model_selector] resolved: provider_idx={}, has_key={}, custom_names={:?}",
            provider_idx,
            api_key.is_some(),
            self.ps.custom_names,
        );

        // Load zhipu endpoint type from config
        self.ps.zhipu_endpoint_type = config
            .providers
            .zhipu
            .as_ref()
            .and_then(|p| p.endpoint_type.as_deref())
            .map(|et| if et == "coding" { 1 } else { 0 })
            .unwrap_or(0);

        // Track whether key exists — never load the actual key into UI state
        self.ps.has_existing_key = api_key.is_some();
        self.ps.api_key_input.clear();

        // Load Qwen rotation state from persisted config (single source of truth)
        self.ps.load_qwen_rotation_from_config();

        // Spawn async model fetch — dialog opens immediately, models arrive via event
        let sender = self.event_sender();
        tokio::spawn(async move {
            let models =
                super::onboarding::fetch_provider_models(provider_idx, api_key.as_deref(), None)
                    .await;
            let _ = sender.send(TuiEvent::ModelSelectorModelsFetched(provider_idx, models));
        });

        // Clear models until fetch completes
        self.ps.models.clear();

        // Reset view state
        self.ps.showing_providers = false;
        self.ps.model_filter.clear();
        self.ps.focused_field = 0;
        self.ps.selected_model = 0;

        self.mode = AppMode::ModelSelector;
    }

    /// Handle keys in model selector mode
    pub(crate) async fn handle_model_selector_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;
        use super::onboarding::PROVIDERS;

        let is_zhipu = self.ps.selected_provider == 6;

        if keys::is_cancel(&event) {
            self.switch_mode(AppMode::Chat).await?;
        } else if event.code == crossterm::event::KeyCode::Tab {
            // Tab cycles through fields:
            // - Normal providers: provider(0) -> api_key(1) -> model(2) -> provider(0)
            // - Zhipu: provider(0) -> endpoint_type(1) -> api_key(2) -> model(3) -> provider(0)
            // - Custom provider: provider(0) -> base_url(1) -> api_key(2) -> model(3) -> provider(0)
            let is_custom = self.ps.selected_provider >= CUSTOM_PROVIDER_IDX; // Custom provider index
            let max_field = if is_custom {
                5
            } else if is_zhipu {
                4
            } else {
                3
            };
            self.ps.focused_field = (self.ps.focused_field + 1) % max_field;
            // If moving to provider, enable provider list; otherwise show model list
            self.ps.showing_providers = self.ps.focused_field == 0;
        } else if self.ps.focused_field == 0 {
            // Provider selection (focused)
            // Navigate using display order: static providers sorted alphabetically, then customs, then "+New"
            let num_customs = self.ps.custom_names.len();
            let mut static_indices: Vec<usize> = (0..CUSTOM_PROVIDER_IDX).collect();
            static_indices.sort_by_key(|&i| PROVIDERS[i].name.to_ascii_lowercase());
            let display_order: Vec<usize> = static_indices
                .into_iter()
                .chain(CUSTOM_INSTANCES_START..CUSTOM_INSTANCES_START + num_customs)
                .chain(std::iter::once(CUSTOM_PROVIDER_IDX))
                .collect();
            let provider_changed = match event.code {
                crossterm::event::KeyCode::Up => {
                    let pos = display_order
                        .iter()
                        .position(|&i| i == self.ps.selected_provider)
                        .unwrap_or(0);
                    if pos > 0 {
                        self.ps.selected_provider = display_order[pos - 1];
                    }
                    true
                }
                crossterm::event::KeyCode::Down => {
                    let pos = display_order
                        .iter()
                        .position(|&i| i == self.ps.selected_provider)
                        .unwrap_or(0);
                    if pos + 1 < display_order.len() {
                        self.ps.selected_provider = display_order[pos + 1];
                    }
                    true
                }
                _ => false,
            };

            // If provider changed, detect existing key and refresh models
            if provider_changed {
                tracing::debug!(
                    "[model_selector] provider navigated to idx={}, custom_names_len={}",
                    self.ps.selected_provider,
                    self.ps.custom_names.len(),
                );
                self.detect_model_selector_key_for_provider();

                // Clear/populate custom fields based on selected provider
                let provider_idx = self.ps.selected_provider;
                if provider_idx == CUSTOM_PROVIDER_IDX {
                    // "+ New Custom" — clear all custom fields for fresh entry
                    self.ps.custom_name.clear();
                    self.ps.custom_model.clear();
                    self.ps.base_url.clear();
                    self.ps.context_window.clear();
                    self.ps.api_key_input.clear();
                } else if provider_idx >= CUSTOM_INSTANCES_START {
                    // Existing custom provider — populate fields from config
                    let custom_idx = provider_idx - CUSTOM_INSTANCES_START;
                    if let Some(name) = self.ps.custom_names.get(custom_idx).cloned()
                        && let Ok(c) = crate::config::Config::load()
                        && let Some(cfg) = c.providers.custom_by_name(&name)
                    {
                        self.ps.custom_name = name;
                        self.ps.base_url = cfg.base_url.clone().unwrap_or_default();
                        self.ps.custom_model = cfg.default_model.clone().unwrap_or_default();
                        self.ps.context_window = cfg
                            .context_window
                            .map(|cw| cw.to_string())
                            .unwrap_or_default();
                    }
                } else {
                    // Static provider — clear custom fields
                    self.ps.custom_name.clear();
                    self.ps.custom_model.clear();
                    self.ps.base_url.clear();
                    self.ps.context_window.clear();
                }

                // Re-fetch models for the new provider — load API key from config
                let api_key = crate::config::Config::load().ok().and_then(|c| {
                    match provider_idx {
                        0 => c.providers.anthropic.and_then(|p| p.api_key),
                        1 => c.providers.openai.and_then(|p| p.api_key),
                        2 => c.providers.github.and_then(|p| p.api_key),
                        3 => c.providers.gemini.and_then(|p| p.api_key),
                        4 => c.providers.openrouter.and_then(|p| p.api_key),
                        5 => c.providers.minimax.and_then(|p| p.api_key),
                        6 => c.providers.zhipu.and_then(|p| p.api_key),
                        idx if idx >= 10 => {
                            let custom_idx = idx - 10;
                            self.ps.custom_names.get(custom_idx).and_then(|name| {
                                c.providers
                                    .custom_by_name(name)
                                    .and_then(|p| p.api_key.clone())
                            })
                        }
                        _ => None,
                    }
                    .filter(|k| !k.is_empty())
                });
                let zhipu_et = if provider_idx == 6 {
                    Some(
                        if self.ps.zhipu_endpoint_type == 1 {
                            "coding"
                        } else {
                            "api"
                        }
                        .to_string(),
                    )
                } else {
                    None
                };
                let sender = self.event_sender();
                tokio::spawn(async move {
                    let models = super::onboarding::fetch_provider_models(
                        provider_idx,
                        api_key.as_deref(),
                        zhipu_et.as_deref(),
                    )
                    .await;
                    let _ = sender.send(TuiEvent::ModelSelectorModelsFetched(provider_idx, models));
                });
                self.ps.models.clear();
                self.ps.selected_model = 0;
            }
        } else if self.ps.focused_field == 1 && self.ps.selected_provider == 10 {
            // Qwen native: auth/rotation area (field 1)
            match event.code {
                crossterm::event::KeyCode::Char(' ') => {
                    self.ps.qwen_rotation_enabled = !self.ps.qwen_rotation_enabled;
                    if self.ps.qwen_rotation_enabled && self.ps.qwen_rotation_count < 2 {
                        self.ps.qwen_rotation_count = 2;
                    }
                }
                crossterm::event::KeyCode::Char(c @ '2'..='9') if self.ps.qwen_rotation_enabled => {
                    self.ps.qwen_rotation_count = (c as usize) - ('0' as usize);
                    self.ps.qwen_rotation_count_changed();
                }
                crossterm::event::KeyCode::Char('1') if self.ps.qwen_rotation_enabled => {
                    self.ps.qwen_rotation_count = 10;
                    self.ps.qwen_rotation_count_changed();
                }
                _ => {}
            }
        } else if self.ps.focused_field == 1 && is_zhipu {
            // z.ai GLM endpoint type toggle (field 1)
            match event.code {
                crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Down => {
                    self.ps.zhipu_endpoint_type = 1 - self.ps.zhipu_endpoint_type;
                }
                _ => {}
            }
        } else if self.ps.focused_field == 1 && self.ps.selected_provider >= CUSTOM_PROVIDER_IDX {
            // Base URL input for Custom provider (field 1)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    self.ps.base_url.push(c);
                }
                crossterm::event::KeyCode::Backspace => {
                    self.ps.base_url.pop();
                }
                _ => {}
            }
        } else if (self.ps.focused_field == 1 && self.ps.selected_provider < 7 && !is_zhipu)
            || (self.ps.focused_field == 2 && is_zhipu)
            || (self.ps.focused_field == 2 && self.ps.selected_provider >= CUSTOM_PROVIDER_IDX)
        {
            // API key input (field 1 for non-Custom non-zhipu, field 2 for zhipu/Custom)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    // Clear sentinel on first keystroke so user replaces it
                    if self.ps.has_existing_key_sentinel() {
                        self.ps.api_key_input.clear();
                    }
                    self.ps.api_key_input.push(c);
                }
                crossterm::event::KeyCode::Backspace => {
                    // Clear sentinel entirely on backspace (can't partially edit masked key)
                    if self.ps.has_existing_key_sentinel() {
                        self.ps.api_key_input.clear();
                    } else {
                        self.ps.api_key_input.pop();
                    }
                }
                _ => {}
            }
        } else if self.ps.focused_field == 3 && self.ps.selected_provider >= CUSTOM_PROVIDER_IDX {
            // Custom provider: free-text model name input (field 3)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    self.ps.custom_model.push(c);
                }
                crossterm::event::KeyCode::Backspace => {
                    self.ps.custom_model.pop();
                }
                _ => {}
            }
        } else if self.ps.focused_field == 4 && self.ps.selected_provider >= CUSTOM_PROVIDER_IDX {
            // Custom provider: name identifier input (field 4)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    self.ps.custom_name.push(c);
                    self.error_message = None;
                    self.error_message_shown_at = None;
                }
                crossterm::event::KeyCode::Backspace => {
                    self.ps.custom_name.pop();
                }
                _ => {}
            }
        } else if self.ps.focused_field == 5 && self.ps.selected_provider >= CUSTOM_PROVIDER_IDX {
            // Custom provider: context window input (field 5 — last before save)
            match event.code {
                crossterm::event::KeyCode::Char(c) if c.is_ascii_digit() => {
                    self.ps.context_window.push(c);
                }
                crossterm::event::KeyCode::Backspace => {
                    self.ps.context_window.pop();
                }
                _ => {}
            }
        } else if (self.ps.focused_field == 2
            && self.ps.selected_provider < CUSTOM_PROVIDER_IDX
            && !is_zhipu)
            || (self.ps.focused_field == 3 && is_zhipu)
        {
            // Non-custom: filter/search model list (field 2, or field 3 for zhipu)
            match event.code {
                crossterm::event::KeyCode::Char(c) => {
                    // Type to filter models
                    self.ps.model_filter.push(c);
                    self.ps.selected_model = 0;
                }
                crossterm::event::KeyCode::Backspace => {
                    self.ps.model_filter.pop();
                    // Keep selection valid after filter change
                    let filter = self.ps.model_filter.to_lowercase();
                    let count = if self.ps.models.is_empty() {
                        PROVIDERS[self.ps.selected_provider].models.len()
                    } else {
                        self.ps
                            .models
                            .iter()
                            .filter(|m| m.to_lowercase().contains(&filter))
                            .count()
                    };
                    if self.ps.selected_model >= count && count > 0 {
                        self.ps.selected_model = count - 1;
                    }
                }
                crossterm::event::KeyCode::Esc => {
                    // Clear filter on Escape
                    self.ps.model_filter.clear();
                    self.ps.selected_model = 0;
                }
                _ => {
                    if keys::is_up(&event) {
                        self.ps.selected_model = self.ps.selected_model.saturating_sub(1);
                    } else if keys::is_down(&event) {
                        // Get filtered count
                        let filter = self.ps.model_filter.to_lowercase();
                        let max_models = if self.ps.models.is_empty() {
                            PROVIDERS[self.ps.selected_provider].models.len()
                        } else {
                            self.ps
                                .models
                                .iter()
                                .filter(|m| m.to_lowercase().contains(&filter))
                                .count()
                        };
                        if max_models > 0 {
                            self.ps.selected_model =
                                (self.ps.selected_model + 1).min(max_models - 1);
                        }
                    }
                }
            }
        }

        // Enter to confirm - move to next field
        if keys::is_enter(&event) {
            let is_custom = self.ps.selected_provider >= CUSTOM_PROVIDER_IDX;

            tracing::debug!(
                "[model_selector] Enter pressed: field={}, provider_idx={}, is_custom={}, custom_name='{}', base_url='{}'",
                self.ps.focused_field,
                self.ps.selected_provider,
                is_custom,
                self.ps.custom_name,
                self.ps.base_url,
            );

            let is_cli_provider = matches!(self.ps.selected_provider, 7..=9);

            if self.ps.focused_field == 0 {
                // On provider field - save config, DON'T close dialog
                // Map indices >= 12 back to 11 for save (custom provider)
                let save_idx = self.ps.selected_provider.min(11);
                if let Err(e) = self
                    .save_provider_selection_internal(save_idx, false, false)
                    .await
                {
                    self.push_system_message(format!("Error: {}", e));
                } else {
                    // CLI providers have no API key — skip straight to model field
                    self.ps.focused_field = if is_cli_provider { 2 } else { 1 };
                }
            } else if self.ps.focused_field == 1 && self.ps.selected_provider == 10 {
                // Qwen native: field 1 is auth/rotation area
                use super::onboarding::QwenDeviceFlowStatus;

                // RotationStep means we're actively starting a flow — ignore Enter
                if matches!(
                    self.ps.qwen_device_flow_status,
                    QwenDeviceFlowStatus::RotationStep { .. }
                ) {
                    return Ok(());
                }

                // WaitingForUser: user hit Enter again before verifying —
                // restart with a fresh device code/URL
                if matches!(
                    self.ps.qwen_device_flow_status,
                    QwenDeviceFlowStatus::WaitingForUser { .. }
                ) {
                    tracing::info!(
                        "Qwen OAuth: user requested new device code (Enter while waiting)"
                    );
                    if self.ps.qwen_rotation_enabled {
                        let current = self.ps.qwen_rotation_current;
                        let total = self.ps.qwen_rotation_count;
                        self.ps.qwen_device_flow_status =
                            QwenDeviceFlowStatus::RotationStep { current, total };
                    }
                    self.spawn_qwen_device_flow();
                    return Ok(());
                }

                // Handle signout confirmation during rotation
                if matches!(
                    self.ps.qwen_device_flow_status,
                    QwenDeviceFlowStatus::RotationSignout { .. }
                ) {
                    let cli_path = crate::brain::provider::qwen::QwenCredentials::qwen_cli_path();
                    let _ = std::fs::remove_file(&cli_path);
                    let current = self.ps.qwen_rotation_current;
                    let total = self.ps.qwen_rotation_count;
                    self.ps.qwen_device_flow_status =
                        QwenDeviceFlowStatus::RotationStep { current, total };
                    self.spawn_qwen_device_flow();
                    return Ok(());
                }

                // Rotation complete → advance to model
                if matches!(
                    self.ps.qwen_device_flow_status,
                    QwenDeviceFlowStatus::RotationComplete
                ) {
                    self.ps.focused_field = 2;
                    self.ps.selected_model = 0;
                    return Ok(());
                }

                // If rotation accounts already exist in config, skip auth — just
                // advance to model selection. Only re-trigger the rotation flow when
                // the user explicitly changed the count (collected is empty means
                // we haven't started a new flow this dialog session).
                if self.ps.qwen_should_skip_auth() {
                    // Accounts already set up with matching count — advance to model
                    self.ps.focused_field = 2;
                    self.ps.selected_model = 0;
                    return Ok(());
                } else if self.ps.qwen_rotation_enabled && self.ps.qwen_rotation_count >= 2 {
                    // Rotation setup — incremental if accounts already exist
                    let start_from = self.ps.qwen_rotation_prepare_flow();
                    self.ps.qwen_device_flow_status = QwenDeviceFlowStatus::RotationStep {
                        current: start_from,
                        total: self.ps.qwen_rotation_count,
                    };
                    let cli_path = crate::brain::provider::qwen::QwenCredentials::qwen_cli_path();
                    let _ = std::fs::remove_file(&cli_path);
                    self.spawn_qwen_device_flow();
                } else if matches!(
                    self.ps.qwen_device_flow_status,
                    QwenDeviceFlowStatus::Complete
                ) || self.ps.has_existing_key
                {
                    // Single-account already authenticated → advance to model
                    self.ps.focused_field = 2;
                    self.ps.selected_model = 0;
                    return Ok(());
                } else {
                    self.ps.qwen_device_flow_status = QwenDeviceFlowStatus::WaitingForUser {
                        verification_uri: String::new(),
                    };
                    self.spawn_qwen_device_flow();
                }
            } else if self.ps.focused_field == 1 && is_zhipu {
                // z.ai GLM: field 1 is endpoint type, move to field 2 (api_key)
                self.ps.focused_field = 2;
            } else if self.ps.focused_field == 1 && is_custom {
                // Custom provider: field 1 is base_url, move to field 2 (api_key)
                self.ps.focused_field = 2;
            } else if (self.ps.focused_field == 1 && !is_custom && !is_zhipu)
                || (self.ps.focused_field == 2 && (is_custom || is_zhipu))
            {
                // On API key field (field 1 for non-Custom non-zhipu, field 2 for zhipu/Custom)
                // Map >= 11 back to 11 for save (all custom providers use idx 11+)
                let provider_idx = self.ps.selected_provider.min(11);

                // User typed a new key — sentinel means key was pre-populated and not changed
                let key_changed =
                    !self.ps.api_key_input.is_empty() && !self.ps.has_existing_key_sentinel();
                let api_key = if key_changed {
                    Some(self.ps.api_key_input.clone())
                } else {
                    // Existing key untouched — load from config (merged with keys.toml)
                    crate::config::Config::load().ok().and_then(|c| {
                        match provider_idx {
                            0 => c.providers.anthropic.and_then(|p| p.api_key),
                            1 => c.providers.openai.and_then(|p| p.api_key),
                            2 => c.providers.github.and_then(|p| p.api_key),
                            3 => c.providers.gemini.and_then(|p| p.api_key),
                            4 => c.providers.openrouter.and_then(|p| p.api_key),
                            5 => c.providers.minimax.and_then(|p| p.api_key),
                            6 => c.providers.zhipu.and_then(|p| p.api_key),
                            7 => None, // Claude CLI — no API key
                            8 => None, // OpenCode CLI — no API key
                            9 => None, // Qwen Code CLI — no API key
                            10 => c.providers.qwen.and_then(|p| p.api_key),
                            11 => c
                                .providers
                                .active_custom()
                                .and_then(|(_, p)| p.api_key.clone()),
                            _ => None,
                        }
                        .filter(|k| !k.is_empty())
                    })
                };

                // Save provider config - DON'T close
                if let Err(e) = self
                    .save_provider_selection_internal(provider_idx, false, key_changed)
                    .await
                {
                    self.push_system_message(format!("Error: {}", e));
                } else {
                    // Fetch live models from the provider (for non-Custom)
                    if !is_custom {
                        let zhipu_et = if is_zhipu {
                            Some(if self.ps.zhipu_endpoint_type == 1 {
                                "coding"
                            } else {
                                "api"
                            })
                        } else {
                            None
                        };
                        self.ps.models = super::onboarding::fetch_provider_models(
                            provider_idx,
                            api_key.as_deref(),
                            zhipu_et,
                        )
                        .await;
                    }
                    self.ps.selected_model = 0;

                    // Move to model field (field 2 for non-Custom, field 3 for Custom/zhipu)
                    self.ps.focused_field = if is_custom || is_zhipu { 3 } else { 2 };
                }
            } else if is_custom && self.ps.focused_field == 3 {
                // Custom: after model, go to name field (field 4)
                self.ps.focused_field = 4;
            } else if is_custom && self.ps.focused_field == 4 {
                // Custom: on name field — validate, normalize, then go to context window
                if self.ps.custom_name.is_empty() {
                    self.error_message =
                        Some("Enter a name identifier for this provider".to_string());
                    self.error_message_shown_at = Some(std::time::Instant::now());
                } else {
                    self.ps.custom_name = crate::config::normalize_toml_key(&self.ps.custom_name);
                    self.error_message = None;
                    self.error_message_shown_at = None;
                    self.ps.focused_field = 5;
                }
            } else if is_custom && self.ps.focused_field == 5 {
                // Custom: on context window field — save
                self.save_provider_selection(self.ps.selected_provider.min(11), false)
                    .await?;
            } else {
                // Non-custom: on model field — save and close
                self.save_provider_selection(self.ps.selected_provider.min(11), false)
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

        // Load existing config to merge — CRITICAL: empty defaults would wipe all settings
        let mut config = crate::config::Config::load().map_err(|e| {
            anyhow::anyhow!(
                "Cannot save provider: config.toml failed to load ({}). Fix config first.",
                e
            )
        })?;

        // Disable all providers first - we'll enable only the selected one
        if let Some(ref mut p) = config.providers.anthropic {
            p.enabled = false;
        }
        if let Some(ref mut p) = config.providers.openai {
            p.enabled = false;
        }
        if let Some(ref mut p) = config.providers.github {
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
        if let Some(ref mut p) = config.providers.zhipu {
            p.enabled = false;
        }
        if let Some(ref mut p) = config.providers.claude_cli {
            p.enabled = false;
        }
        if let Some(ref mut p) = config.providers.opencode_cli {
            p.enabled = false;
        }

        // Get existing key from config if not changing
        // Indices: 0=Anthropic, 1=OpenAI, 2=GitHub, 3=Gemini, 4=OpenRouter, 5=Minimax, 6=z.ai GLM, 7=Claude CLI, 8=OpenCode CLI, 9=Qwen Code, 10=Qwen (native), 11=Custom
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
                .github
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            3 => config
                .providers
                .gemini
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            4 => config
                .providers
                .openrouter
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            5 => config
                .providers
                .minimax
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            6 => config
                .providers
                .zhipu
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            7 => None, // Claude CLI — no API key needed
            8 => None, // OpenCode CLI — no API key needed
            9 => None, // Qwen Code CLI — no API key needed
            10 => config
                .providers
                .qwen
                .as_ref()
                .and_then(|p| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            11 => config
                .providers
                .active_custom()
                .and_then(|(_, p)| p.api_key.as_ref())
                .filter(|k| !k.is_empty())
                .cloned(),
            _ => None,
        };

        // Only use a key if the user actually typed one — never pull from config
        let api_key = if key_changed && !self.ps.api_key_input.is_empty() {
            Some(self.ps.api_key_input.clone())
        } else {
            existing_key
        };

        // Log what's being saved (hide key)
        tracing::info!(
            "Saving provider config: idx={}, has_api_key={}",
            provider_idx,
            api_key.is_some()
        );

        // Resolve default_model:
        // - On final confirm (close_dialog=true): use the user's selected model
        // - On provider switch (close_dialog=false): preserve existing config model
        let default_model = if !close_dialog {
            // Preserve whatever is already saved in config for this provider
            crate::config::Config::load()
                .ok()
                .and_then(|c| match provider_idx {
                    0 => c.providers.anthropic.and_then(|p| p.default_model),
                    1 => c.providers.openai.and_then(|p| p.default_model),
                    2 => c.providers.github.and_then(|p| p.default_model),
                    3 => c.providers.gemini.and_then(|p| p.default_model),
                    4 => c.providers.openrouter.and_then(|p| p.default_model),
                    5 => c.providers.minimax.and_then(|p| p.default_model),
                    6 => c.providers.zhipu.and_then(|p| p.default_model),
                    7 => c.providers.claude_cli.and_then(|p| p.default_model),
                    8 => c.providers.opencode_cli.and_then(|p| p.default_model),
                    9 => c.providers.qwen_code_cli.and_then(|p| p.default_model),
                    10 => c.providers.qwen.and_then(|p| p.default_model),
                    _ => None,
                })
                .unwrap_or_else(|| {
                    provider
                        .models
                        .first()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "default".to_string())
                })
        } else if provider_idx >= CUSTOM_PROVIDER_IDX {
            self.ps.custom_model.clone()
        } else if !self.ps.models.is_empty() {
            let filter = self.ps.model_filter.to_lowercase();
            let filtered: Vec<_> = self
                .ps
                .models
                .iter()
                .filter(|m| m.to_lowercase().contains(&filter))
                .collect();
            filtered
                .get(self.ps.selected_model)
                .map(|m| m.to_string())
                .or_else(|| self.ps.models.first().cloned())
                .unwrap_or_else(|| self.default_model_name.clone())
        } else if let Some(model) = provider.models.get(self.ps.selected_model) {
            model.to_string()
        } else {
            provider
                .models
                .first()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "default".to_string())
        };
        // Indices: 0=Anthropic, 1=OpenAI, 2=GitHub, 3=Gemini, 4=OpenRouter, 5=Minimax, 6=z.ai GLM, 7=Claude CLI, 8=OpenCode CLI, 9=Qwen Code, 10=Qwen (native), 11=Custom
        match provider_idx {
            0 => {
                // Anthropic
                config.providers.anthropic = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
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
                    vision_model: None,
                    ..Default::default()
                });
            }
            2 => {
                // GitHub Copilot
                config.providers.github = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: Some("https://api.githubcopilot.com/chat/completions".to_string()),
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                });
            }
            3 => {
                // Gemini
                config.providers.gemini = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                });
            }
            4 => {
                // OpenRouter
                config.providers.openrouter = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: Some("https://openrouter.ai/api/v1/chat/completions".to_string()),
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                });
            }
            5 => {
                // Minimax
                config.providers.minimax = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: Some("https://api.minimax.io/v1".to_string()),
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                });
            }
            6 => {
                // z.ai GLM — use endpoint type from model selector state
                let endpoint_type = Some(
                    if self.ps.zhipu_endpoint_type == 1 {
                        "coding"
                    } else {
                        "api"
                    }
                    .to_string(),
                );
                config.providers.zhipu = Some(ProviderConfig {
                    enabled: true,
                    api_key: api_key.clone(),
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    endpoint_type,
                    ..Default::default()
                });
            }
            7 => {
                // Claude CLI (Max subscription) — no API key needed
                config.providers.claude_cli = Some(ProviderConfig {
                    enabled: true,
                    api_key: None,
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                });
            }
            8 => {
                // OpenCode CLI — no API key needed
                config.providers.opencode_cli = Some(ProviderConfig {
                    enabled: true,
                    api_key: None,
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                });
            }
            9 => {
                // Qwen Code CLI — no API key needed
                config.providers.qwen_code_cli = Some(ProviderConfig {
                    enabled: true,
                    api_key: None,
                    base_url: None,
                    default_model: Some(default_model.to_string()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                });
            }
            10 => {
                // Qwen (native) — OAuth credentials already stored in keys.toml
                // by the device flow handler. Here we just ensure the config
                // section exists and is enabled with the chosen default model.
                let merged = config.providers.qwen.clone().unwrap_or_default();
                config.providers.qwen = Some(ProviderConfig {
                    enabled: true,
                    default_model: Some(default_model.to_string()),
                    ..merged
                });
            }
            11 if !self.ps.custom_name.is_empty() => {
                let custom_model = self.ps.custom_model.clone();
                let custom_name = self.ps.custom_name.clone();
                let mut customs = config.providers.custom.unwrap_or_default();
                let context_window = self.ps.context_window.parse::<u32>().ok();
                customs.insert(
                    custom_name,
                    ProviderConfig {
                        enabled: true,
                        api_key: api_key.clone(),
                        base_url: Some(self.ps.base_url.clone()),
                        default_model: Some(custom_model),
                        models: vec![],
                        vision_model: None,
                        context_window,
                        ..Default::default()
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
            2 => "providers.github",
            3 => "providers.gemini",
            4 => "providers.openrouter",
            5 => "providers.minimax",
            6 => "providers.zhipu",
            7 => "providers.claude_cli",
            8 => "providers.opencode_cli",
            9 => "providers.qwen_code_cli",
            10 => "providers.qwen",
            11 => {
                // Resolve custom provider name: UI field > config active
                let cname = if !self.ps.custom_name.is_empty() {
                    self.ps.custom_name.clone()
                } else if let Some((name, _)) = config.providers.active_custom() {
                    name.to_string()
                } else {
                    // No name → can't save; fall through to anthropic
                    // (cleanup_empty_custom_providers will remove ghosts)
                    return Ok(());
                };
                custom_section = format!("providers.custom.{}", cname);
                &custom_section
            }
            _ => "providers.anthropic",
        };

        // Disable ALL other providers on disk before enabling the selected one.
        // rebuild_agent_service() reloads from disk, so this is the only source of truth.
        let mut write_errors: Vec<String> = Vec::new();
        let mut try_write = |s: &str, k: &str, v: &str| {
            if let Err(e) = crate::config::Config::write_key(s, k, v) {
                tracing::warn!("Failed to write {}.{}: {}", s, k, e);
                write_errors.push(format!("{}.{}", s, k));
            }
        };

        for s in [
            "providers.anthropic",
            "providers.openai",
            "providers.github",
            "providers.gemini",
            "providers.openrouter",
            "providers.minimax",
            "providers.zhipu",
            "providers.claude_cli",
            "providers.opencode_cli",
            "providers.qwen_code_cli",
            "providers.qwen",
        ] {
            if s != section {
                try_write(s, "enabled", "false");
            }
        }
        if let Some(ref customs) = config.providers.custom {
            for name in customs.keys() {
                let cs = format!("providers.custom.{}", name);
                if cs != section {
                    try_write(&cs, "enabled", "false");
                }
            }
        }

        try_write(section, "enabled", "true");

        // Write base_url if applicable
        match provider_idx {
            2 => {
                try_write(
                    section,
                    "base_url",
                    "https://api.githubcopilot.com/chat/completions",
                );
            }
            4 => {
                try_write(
                    section,
                    "base_url",
                    "https://openrouter.ai/api/v1/chat/completions",
                );
            }
            5 => {
                try_write(section, "base_url", "https://api.minimax.io/v1");
            }
            6 => {
                // z.ai GLM — write endpoint_type from model selector state
                let endpoint_type = if self.ps.zhipu_endpoint_type == 1 {
                    "coding"
                } else {
                    "api"
                };
                try_write(section, "endpoint_type", endpoint_type);
            }
            10 if !self.ps.base_url.is_empty() => {
                try_write(section, "base_url", &self.ps.base_url);
                if !self.ps.context_window.is_empty() {
                    try_write(section, "context_window", &self.ps.context_window);
                }
            }
            11 => {
                try_write(section, "base_url", &self.ps.base_url);
                if !self.ps.context_window.is_empty() {
                    try_write(section, "context_window", &self.ps.context_window);
                }
            }
            _ => {}
        }

        // Clean up ghost custom provider entries (empty name/url/model)
        crate::config::Config::cleanup_empty_custom_providers();

        // Write API key to keys.toml BEFORE any Config::load() calls.
        // Otherwise merge_provider_keys() won't find the key during rebuild.
        // This was the root cause of custom provider keys not persisting on first try.
        if key_changed
            && !self.ps.has_existing_key_sentinel()
            && let Some(ref key) = api_key
            && !key.is_empty()
            && let Err(e) = crate::config::write_secret_key(section, "api_key", key)
        {
            tracing::warn!("Failed to save API key to keys.toml: {}", e);
        }

        // Refresh custom provider names list after saving (so new entries appear immediately)
        if provider_idx == CUSTOM_PROVIDER_IDX
            && let Ok(fresh) = crate::config::Config::load()
        {
            self.ps.custom_names = fresh
                .providers
                .custom
                .as_ref()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            tracing::debug!(
                "[save_provider] refreshed custom_names after save: {:?}",
                self.ps.custom_names,
            );
            // Point selection to the newly saved custom provider
            if !self.ps.custom_name.is_empty()
                && let Some(pos) = self
                    .ps
                    .custom_names
                    .iter()
                    .position(|n| n == &self.ps.custom_name)
            {
                self.ps.selected_provider = CUSTOM_INSTANCES_START + pos;
                tracing::debug!(
                    "[save_provider] mapped custom '{}' to idx={}",
                    self.ps.custom_name,
                    CUSTOM_INSTANCES_START + pos,
                );
            }
        }

        // Write default_model to config BEFORE rebuild so the provider picks it up
        if let Err(e) = crate::config::Config::write_key(section, "default_model", &default_model) {
            tracing::warn!("Failed to persist model to config: {}", e);
            write_errors.push(format!("{}.default_model", section));
        }

        // Warn user if any config writes failed
        if !write_errors.is_empty() {
            self.push_system_message(format!(
                "⚠️ Failed to save some config keys: {}. Check file permissions on config.toml.",
                write_errors.join(", ")
            ));
        }

        // Rebuild agent service with new provider (now sees the correct model)
        if let Err(e) = self.rebuild_agent_service().await {
            if api_key.is_none() && provider_idx == CUSTOM_PROVIDER_IDX {
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

        // Update app state — use the ACTUAL provider's model, not the requested one.
        // If the requested provider failed (e.g. Qwen no creds) and a fallback kicked in,
        // the agent service's provider_model() reflects the real active model.
        let actual_model = self.agent_service.provider_model();
        self.default_model_name = actual_model.clone();

        // Persist provider + model to current session DB record
        let agent_provider_name = self.agent_service.provider_name();
        if let Some(ref mut session) = self.current_session {
            session.provider_name = Some(agent_provider_name.clone());
            session.model = Some(actual_model.clone());
            let session_copy = session.clone();
            if let Err(e) = self.session_service.update_session(&session_copy).await {
                tracing::warn!("Failed to persist provider to session: {}", e);
            }
        }
        // Cache the provider instance for fast session switching
        let provider_arc = self.agent_service.provider();
        self.provider_cache
            .insert(agent_provider_name, provider_arc);

        // Only close dialog if explicitly requested
        if close_dialog {
            // Use user-configured name for custom providers (e.g. "nvidia"), fall back to generic
            let provider_name =
                if provider_idx == CUSTOM_PROVIDER_IDX && !self.ps.custom_name.is_empty() {
                    self.ps.custom_name.clone()
                } else {
                    provider
                        .name
                        .split('(')
                        .next()
                        .unwrap_or(provider.name)
                        .trim()
                        .to_string()
                };

            let change_msg = format!(
                "[Model changed to {} (provider: {})]",
                default_model, provider_name
            );
            self.push_system_message(change_msg.clone());
            self.pending_context.push(change_msg);

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
                WizardAction::QuickJumpDone => {
                    // Quick-jump completed a step — save config then close
                    let mut needs_rebuild = false;
                    if let Some(ref wizard) = self.onboarding {
                        if let Err(e) = wizard.apply_config() {
                            self.push_system_message(format!(
                                "Settings saved with warnings: {}",
                                e
                            ));
                        } else {
                            // Show what changed based on the step
                            let msg = match wizard.step {
                                OnboardingStep::ProviderAuth => {
                                    needs_rebuild = true;
                                    let (pname, mname) = if wizard.ps.is_custom() {
                                        (
                                            format!("Custom ({})", wizard.ps.custom_name),
                                            wizard.ps.custom_model.clone(),
                                        )
                                    } else {
                                        (
                                            super::onboarding::PROVIDERS
                                                [wizard.ps.selected_provider]
                                                .name
                                                .to_string(),
                                            wizard.ps.selected_model_name().to_string(),
                                        )
                                    };
                                    format!("[Model changed to {} (provider: {})]", mname, pname)
                                }
                                _ => "Settings saved.".to_string(),
                            };
                            self.push_system_message(msg);
                        }
                    }
                    self.onboarding = None;
                    if needs_rebuild && let Err(e) = self.rebuild_agent_service().await {
                        tracing::warn!("Failed to rebuild agent service: {}", e);
                        self.push_system_message(format!(
                            "Warning: Failed to reload provider: {}",
                            e
                        ));
                    }
                    if needs_rebuild {
                        self.sync_session_to_provider().await;
                    }
                    self.switch_mode(AppMode::Chat).await?;
                }
                WizardAction::Complete => {
                    // Apply wizard config before transitioning
                    if let Some(ref wizard) = self.onboarding {
                        match wizard.apply_config() {
                            Ok(()) => {
                                let (provider_name, model_name) = if wizard.ps.is_custom() {
                                    (
                                        format!("Custom ({})", wizard.ps.custom_name),
                                        wizard.ps.custom_model.clone(),
                                    )
                                } else {
                                    (
                                        super::onboarding::PROVIDERS[wizard.ps.selected_provider]
                                            .name
                                            .to_string(),
                                        wizard.ps.selected_model_name().to_string(),
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
                    self.sync_session_to_provider().await;
                    self.switch_mode(AppMode::Chat).await?;
                }
                WizardAction::FetchModels => {
                    let provider_idx = wizard.ps.selected_provider;
                    // Resolve API key from config (keys.toml) or raw input
                    let api_key = if wizard.ps.has_existing_key_sentinel() {
                        let provider_name = super::onboarding::PROVIDERS
                            [provider_idx.min(super::onboarding::PROVIDERS.len() - 1)]
                        .name;
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
                            "z.ai GLM" => loaded
                                .as_ref()
                                .and_then(|c| c.providers.zhipu.as_ref())
                                .and_then(|p| p.api_key.clone()),
                            "GitHub Copilot" => loaded
                                .as_ref()
                                .and_then(|c| c.providers.github.as_ref())
                                .and_then(|p| p.api_key.clone()),
                            _ => None,
                        }
                    } else if !wizard.ps.api_key_input.is_empty() {
                        Some(wizard.ps.api_key_input.clone())
                    } else {
                        None
                    };
                    wizard.ps.models_fetching = true;

                    // Capture zhipu endpoint type from wizard state (not yet saved to config)
                    let zhipu_et = if provider_idx == 6 {
                        Some(if wizard.ps.zhipu_endpoint_type == 1 {
                            "coding".to_string()
                        } else {
                            "api".to_string()
                        })
                    } else {
                        None
                    };

                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        let models = super::onboarding::fetch_provider_models(
                            provider_idx,
                            api_key.as_deref(),
                            zhipu_et.as_deref(),
                        )
                        .await;
                        let _ = sender.send(TuiEvent::OnboardingModelsFetched(models));
                    });
                }
                WizardAction::QwenDeviceFlow => {
                    wizard.ps.qwen_device_flow_status =
                        super::onboarding::QwenDeviceFlowStatus::WaitingForUser {
                            verification_uri: String::new(),
                        };
                    self.spawn_qwen_device_flow();
                }
                WizardAction::GitHubDeviceFlow => {
                    wizard.github_device_flow_status =
                        super::onboarding::GitHubDeviceFlowStatus::WaitingForUser;
                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        // Step 1: Request device code
                        let device =
                            match crate::brain::provider::copilot::start_device_flow().await {
                                Ok(d) => d,
                                Err(e) => {
                                    let _ = sender.send(TuiEvent::GitHubOAuthError(e.to_string()));
                                    return;
                                }
                            };

                        // Send the user code for display
                        let _ = sender.send(TuiEvent::GitHubDeviceCode(device.user_code.clone()));

                        // Step 2-3: Poll until user authorizes
                        match crate::brain::provider::copilot::poll_for_oauth_token(
                            &device.device_code,
                            device.interval,
                        )
                        .await
                        {
                            Ok(token) => {
                                let _ = sender.send(TuiEvent::GitHubOAuthComplete(token));
                            }
                            Err(e) => {
                                let _ = sender.send(TuiEvent::GitHubOAuthError(e.to_string()));
                            }
                        }
                    });
                }
                WizardAction::WhatsAppConnect => {
                    // Wipe session so agent shows fresh QR, then enable so
                    // ChannelManager (re)starts the single agent bot.
                    #[cfg(feature = "whatsapp")]
                    {
                        let wa_dir = crate::config::opencrabs_home().join("whatsapp");
                        let _ = std::fs::remove_file(wa_dir.join("session.db"));
                        let _ = std::fs::remove_file(wa_dir.join("session.db-wal"));
                        let _ = std::fs::remove_file(wa_dir.join("session.db-shm"));
                        let _ = crate::config::Config::write_key(
                            "channels.whatsapp",
                            "enabled",
                            "true",
                        );
                    }

                    // Subscribe to QR/connected events from the agent bot
                    #[cfg(feature = "whatsapp")]
                    let wa_state = self.whatsapp_state.clone();
                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        #[cfg(feature = "whatsapp")]
                        {
                            let handle =
                                crate::brain::tools::whatsapp_connect::subscribe_whatsapp_pairing(
                                    &wa_state, false,
                                );
                            // Forward QR codes to the TUI
                            let qr_sender = sender.clone();
                            let mut qr_rx = handle.qr_rx;
                            tokio::spawn(async move {
                                while let Ok(qr) = qr_rx.recv().await {
                                    let _ = qr_sender.send(TuiEvent::WhatsAppQrCode(qr));
                                }
                            });
                            // Forward agent errors to the TUI
                            let err_sender = sender.clone();
                            let mut error_rx = handle.error_rx;
                            tokio::spawn(async move {
                                if let Ok(err) = error_rx.recv().await {
                                    let _ = err_sender.send(TuiEvent::WhatsAppError(err));
                                }
                            });
                            // Wait for connection (2 minute timeout)
                            let mut connected_rx = handle.connected_rx;
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(120),
                                connected_rx.recv(),
                            )
                            .await
                            {
                                Ok(Ok(())) => {
                                    let _ = sender.send(TuiEvent::WhatsAppConnected);
                                }
                                Ok(Err(e)) => {
                                    // Broadcast channel closed — agent crashed or failed to start
                                    let msg = format!(
                                        "WhatsApp agent stopped unexpectedly: {}. Check logs at ~/.opencrabs/logs/",
                                        e
                                    );
                                    tracing::error!("{}", msg);
                                    let _ = sender.send(TuiEvent::WhatsAppError(msg));
                                }
                                Err(_) => {
                                    let _ = sender.send(TuiEvent::WhatsAppError(
                                        "QR scan timed out (2 minutes). Press R to retry.".into(),
                                    ));
                                }
                            }
                        }
                    });
                }
                WizardAction::TestWhatsApp => {
                    wizard.channel_test_status = super::onboarding::ChannelTestStatus::Testing;
                    let phone = if wizard.has_existing_whatsapp_phone() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.whatsapp.allowed_phones.first().cloned())
                            .unwrap_or_default()
                    } else {
                        wizard.whatsapp_phone_input.clone()
                    };
                    #[cfg(feature = "whatsapp")]
                    let wa_state = self.whatsapp_state.clone();
                    let sender = self.event_sender();
                    let agent = self.agent_service.clone();
                    tokio::spawn(async move {
                        #[cfg(feature = "whatsapp")]
                        let result = test_whatsapp_connection(wa_state, &phone, agent).await;
                        #[cfg(not(feature = "whatsapp"))]
                        let result: Result<(), String> =
                            Err("WhatsApp feature not enabled".to_string());
                        let _ = sender.send(TuiEvent::ChannelTestResult {
                            channel: "whatsapp".to_string(),
                            success: result.is_ok(),
                            error: result.err(),
                        });
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
                    // telegram_user_id_input is never a sentinel — always the real value.
                    let user_id_str = wizard.telegram_user_id_input.clone();
                    let sender = self.event_sender();
                    let agent = self.agent_service.clone();
                    tokio::spawn(async move {
                        let result = test_telegram_connection(&token, &user_id_str, agent).await;
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
                    let agent = self.agent_service.clone();
                    tokio::spawn(async move {
                        let result = test_discord_connection(&token, &channel_id, agent).await;
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
                    let agent = self.agent_service.clone();
                    tokio::spawn(async move {
                        let result = test_slack_connection(&token, &channel_id, agent).await;
                        let _ = sender.send(TuiEvent::ChannelTestResult {
                            channel: "slack".to_string(),
                            success: result.is_ok(),
                            error: result.err(),
                        });
                    });
                }
                WizardAction::TestTrello => {
                    wizard.channel_test_status = super::onboarding::ChannelTestStatus::Testing;
                    let api_key = if wizard.has_existing_trello_api_key() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.trello.app_token.clone())
                            .unwrap_or_default()
                    } else {
                        wizard.trello_api_key_input.clone()
                    };
                    let api_token = if wizard.has_existing_trello_api_token() {
                        crate::config::Config::load()
                            .ok()
                            .and_then(|c| c.channels.trello.token.clone())
                            .unwrap_or_default()
                    } else {
                        wizard.trello_api_token_input.clone()
                    };
                    let sender = self.event_sender();
                    tokio::spawn(async move {
                        let result = test_trello_connection(&api_key, &api_token).await;
                        let _ = sender.send(TuiEvent::ChannelTestResult {
                            channel: "trello".to_string(),
                            success: result.is_ok(),
                            error: result.err(),
                        });
                    });
                }
                WizardAction::GenerateBrain => {
                    // Extract prompt and workspace path before dropping wizard.
                    // Brain generation runs in the background after entering chat.
                    let brain_context = self.onboarding.as_mut().map(|wizard| {
                        wizard.normalize_brain_inputs();
                        let prompt = wizard.build_brain_prompt();
                        let workspace = wizard.workspace_path.clone();
                        (prompt, workspace)
                    });

                    // Ensure provider is available (fresh install may still
                    // have PlaceholderProvider at this point).
                    if self.agent_service.provider_name() == "none" {
                        if let Some(ref wizard) = self.onboarding
                            && let Err(e) = wizard.apply_config()
                        {
                            tracing::warn!("Brain gen: apply_config before generation: {}", e);
                        }
                        if let Err(e) = self.rebuild_agent_service().await {
                            tracing::warn!("Brain gen: rebuild_agent_service failed: {}", e);
                        }
                    }

                    // Complete onboarding — go straight to chat
                    if let Some(ref wizard) = self.onboarding {
                        match wizard.apply_config() {
                            Ok(()) => {
                                let (provider_name, model_name) = if wizard.ps.is_custom() {
                                    (
                                        format!("Custom ({})", wizard.ps.custom_name),
                                        wizard.ps.custom_model.clone(),
                                    )
                                } else {
                                    (
                                        super::onboarding::PROVIDERS[wizard.ps.selected_provider]
                                            .name
                                            .to_string(),
                                        wizard.ps.selected_model_name().to_string(),
                                    )
                                };
                                self.push_system_message(format!(
                                    "Setup complete! Provider: {} | Model: {}",
                                    provider_name, model_name
                                ));
                                if let Err(e) = self.rebuild_agent_service().await {
                                    tracing::warn!("Failed to rebuild agent service: {}", e);
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
                    self.sync_session_to_provider().await;
                    self.switch_mode(AppMode::Chat).await?;

                    // Fire brain generation in the background
                    if let Some((prompt, workspace)) = brain_context {
                        self.push_system_message(
                            "Generating personalized brain files in the background...".to_string(),
                        );
                        self.generate_brain_files_background(prompt, workspace);
                    }
                }
                WizardAction::DownloadWhisperModel => {
                    #[cfg(feature = "local-stt")]
                    {
                        use crate::channels::voice::local_whisper::{
                            DownloadProgress, LOCAL_MODEL_PRESETS,
                        };
                        let tui_sender = self.event_sender();
                        if let Some(ref mut wizard) = self.onboarding {
                            let idx = wizard.selected_local_stt_model;
                            if idx < LOCAL_MODEL_PRESETS.len() {
                                let preset = &LOCAL_MODEL_PRESETS[idx];
                                wizard.stt_model_download_progress = Some(0.0);
                                let (progress_tx, mut progress_rx) =
                                    tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();
                                let fwd_sender = tui_sender.clone();
                                tokio::spawn(async move {
                                    while let Some(p) = progress_rx.recv().await {
                                        let frac = match p.total {
                                            Some(t) if t > 0 => p.downloaded as f64 / t as f64,
                                            _ => 0.0,
                                        };
                                        let _ = fwd_sender
                                            .send(TuiEvent::WhisperDownloadProgress(frac));
                                    }
                                });
                                tokio::spawn(async move {
                                    let result =
                                        crate::channels::voice::local_whisper::download_model(
                                            preset,
                                            progress_tx,
                                        )
                                        .await;
                                    let _ = tui_sender.send(TuiEvent::WhisperDownloadComplete(
                                        result.map(|_| ()).map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                    }
                }
                WizardAction::DownloadPiperVoice => {
                    #[cfg(feature = "local-tts")]
                    {
                        use crate::channels::voice::local_tts::{
                            DownloadProgress, PIPER_VOICES, delete_other_voices,
                        };
                        let tui_sender = self.event_sender();
                        if let Some(ref mut wizard) = self.onboarding {
                            let idx = wizard.selected_tts_voice;
                            if idx < PIPER_VOICES.len() {
                                let voice_id = PIPER_VOICES[idx].id.to_string();
                                delete_other_voices(&voice_id);
                                wizard.tts_voice_download_progress = Some(0.0);
                                let (progress_tx, mut progress_rx) =
                                    tokio::sync::mpsc::unbounded_channel::<DownloadProgress>();
                                let fwd_sender = tui_sender.clone();
                                tokio::spawn(async move {
                                    while let Some(p) = progress_rx.recv().await {
                                        let frac = match p.total {
                                            Some(t) if t > 0 => p.downloaded as f64 / t as f64,
                                            _ => 0.0,
                                        };
                                        let _ =
                                            fwd_sender.send(TuiEvent::PiperDownloadProgress(frac));
                                    }
                                });
                                tokio::spawn(async move {
                                    // Install Piper venv if not present
                                    if !crate::channels::voice::local_tts::piper_venv_exists() {
                                        let (setup_tx, mut setup_rx) =
                                            tokio::sync::mpsc::unbounded_channel::<
                                                crate::channels::voice::local_tts::SetupProgress,
                                            >();
                                        let setup_fwd = tui_sender.clone();
                                        tokio::spawn(async move {
                                            while let Some(p) = setup_rx.recv().await {
                                                tracing::info!("Piper setup: {}", p.stage);
                                                let _ = setup_fwd
                                                    .send(TuiEvent::PiperDownloadProgress(0.0));
                                            }
                                        });
                                        if let Err(e) =
                                            crate::channels::voice::local_tts::setup_piper_venv(
                                                setup_tx,
                                            )
                                            .await
                                        {
                                            let _ = tui_sender.send(
                                                TuiEvent::PiperDownloadComplete(Err(e.to_string())),
                                            );
                                            return;
                                        }
                                    }
                                    // Download voice model
                                    let result = crate::channels::voice::local_tts::download_voice(
                                        &voice_id,
                                        progress_tx,
                                    )
                                    .await;
                                    let _ = tui_sender.send(TuiEvent::PiperDownloadComplete(
                                        result.map(|_| voice_id.clone()).map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                        }
                    }
                }
                WizardAction::QwenRotationFlow => {
                    // Start multi-account rotation setup (incremental if accounts exist)
                    let start_from = wizard.ps.qwen_rotation_prepare_flow();
                    wizard.ps.qwen_device_flow_status =
                        super::onboarding::QwenDeviceFlowStatus::RotationStep {
                            current: start_from,
                            total: wizard.ps.qwen_rotation_count,
                        };
                    // Clear any stale qwen-cli creds so browser prompts fresh login
                    let cli_path = crate::brain::provider::qwen::QwenCredentials::qwen_cli_path();
                    let _ = std::fs::remove_file(&cli_path);
                    self.spawn_qwen_device_flow();
                }
                WizardAction::QwenRotationNext => {
                    // User confirmed signout — kick off next account's device flow
                    let cli_path = crate::brain::provider::qwen::QwenCredentials::qwen_cli_path();
                    let _ = std::fs::remove_file(&cli_path);
                    let current = wizard.ps.qwen_rotation_current;
                    let total = wizard.ps.qwen_rotation_count;
                    wizard.ps.qwen_device_flow_status =
                        super::onboarding::QwenDeviceFlowStatus::RotationStep { current, total };
                    self.spawn_qwen_device_flow();
                }
                WizardAction::None => {
                    // Stay in onboarding
                }
            }
        }
        Ok(())
    }

    /// Fire brain generation in the background. Onboarding is already done —
    /// the user is in chat. On success, writes brain files directly to workspace
    /// and notifies via `TuiEvent::BrainGenerationResult`.
    fn generate_brain_files_background(&self, prompt: String, workspace: String) {
        let provider = self.agent_service.provider().clone();
        let model = self.agent_service.provider_model().to_string();
        let sender = self.event_sender();

        let request = LLMRequest::new(model, vec![crate::brain::provider::Message::user(prompt)])
            .with_max_tokens(65536);

        tokio::spawn(async move {
            let result: Result<String, String> = match tokio::time::timeout(
                std::time::Duration::from_secs(120),
                provider.complete(request),
            )
            .await
            {
                Ok(Ok(response)) => {
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

                    // Parse and write directly to workspace
                    let parsed = crate::tui::onboarding::parse_brain_sections(&text);

                    let names = ["SOUL", "IDENTITY", "USER", "AGENTS", "TOOLS", "MEMORY"];
                    let found: Vec<&str> = names
                        .iter()
                        .zip(parsed.iter())
                        .filter_map(|(n, p)| p.as_ref().map(|_| *n))
                        .collect();
                    let missing: Vec<&str> = names
                        .iter()
                        .zip(parsed.iter())
                        .filter_map(|(n, p)| if p.is_none() { Some(*n) } else { None })
                        .collect();
                    tracing::info!(
                        "Brain gen parsed: found=[{}], missing=[{}]",
                        found.join(", "),
                        missing.join(", ")
                    );

                    // Need at least SOUL + IDENTITY + USER
                    if parsed[0].is_none() || parsed[1].is_none() || parsed[2].is_none() {
                        tracing::warn!(
                            "Brain gen: couldn't parse response (first 500 chars): {}",
                            &text[..text.len().min(500)]
                        );
                        Err(
                            "Couldn't parse brain files from AI response — using defaults"
                                .to_string(),
                        )
                    } else {
                        let ws = std::path::Path::new(&workspace);
                        let file_map = [
                            ("SOUL.md", &parsed[0]),
                            ("IDENTITY.md", &parsed[1]),
                            ("USER.md", &parsed[2]),
                            ("AGENTS.md", &parsed[3]),
                            ("TOOLS.md", &parsed[4]),
                            ("MEMORY.md", &parsed[5]),
                        ];
                        let mut written = 0;
                        for (filename, content) in &file_map {
                            if let Some(text) = content {
                                if let Err(e) = std::fs::write(ws.join(filename), text) {
                                    tracing::warn!(
                                        "Brain gen: failed to write {}: {}",
                                        filename,
                                        e
                                    );
                                } else {
                                    written += 1;
                                }
                            }
                        }
                        Ok(format!("{written} brain files personalized"))
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!("Brain generation failed: {}", e);
                    Err(format!("Brain generation failed: {}", e))
                }
                Err(_) => {
                    tracing::warn!("Brain generation timed out after 120s");
                    Err("Brain generation timed out — using defaults".to_string())
                }
            };
            let _ = sender.send(TuiEvent::BrainGenerationResult { result });
        });
    }

    /// Open file picker and populate file list.
    ///
    /// Starts at the session's working directory (not the app startup cwd).
    /// Call `refresh_file_picker()` to reload entries without resetting the dir.
    pub(crate) async fn open_file_picker(&mut self) -> Result<()> {
        // Start at the session's working directory
        self.file_picker_current_dir = self.working_directory.clone();
        self.file_picker_search.clear();
        self.refresh_file_picker().await
    }

    /// Reload file list for the current directory and apply search filter.
    pub(crate) async fn refresh_file_picker(&mut self) -> Result<()> {
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
        self.apply_file_picker_filter();
        self.switch_mode(AppMode::FilePicker).await?;

        Ok(())
    }

    /// Filter the file list based on the current search query.
    /// ".." is always included so the user can navigate back.
    fn apply_file_picker_filter(&mut self) {
        let query = self.file_picker_search.to_lowercase();
        self.file_picker_filtered = if query.is_empty() {
            (0..self.file_picker_files.len()).collect()
        } else {
            self.file_picker_files
                .iter()
                .enumerate()
                .filter(|(_, path)| {
                    // Always keep ".." visible
                    if path.ends_with("..") {
                        return true;
                    }
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|name| name.to_lowercase().contains(&query))
                })
                .map(|(i, _)| i)
                .collect()
        };
        self.file_picker_selected = 0;
        self.file_picker_scroll_offset = 0;
    }

    /// Handle keys in file picker mode
    pub(crate) async fn handle_file_picker_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;
        use crossterm::event::KeyCode;

        let filtered_len = self.file_picker_filtered.len();

        if keys::is_cancel(&event) {
            self.file_picker_search.clear();
            self.switch_mode(AppMode::Chat).await?;
        } else if keys::is_up(&event) {
            self.file_picker_selected = self.file_picker_selected.saturating_sub(1);
            if self.file_picker_selected < self.file_picker_scroll_offset {
                self.file_picker_scroll_offset = self.file_picker_selected;
            }
        } else if keys::is_down(&event) {
            if self.file_picker_selected + 1 < filtered_len {
                self.file_picker_selected += 1;
                let visible_items = 20;
                if self.file_picker_selected >= self.file_picker_scroll_offset + visible_items {
                    self.file_picker_scroll_offset = self.file_picker_selected - visible_items + 1;
                }
            }
        } else if keys::is_enter(&event) || keys::is_tab(&event) {
            // Resolve filtered index to actual file index
            if let Some(&file_idx) = self.file_picker_filtered.get(self.file_picker_selected)
                && let Some(selected_path) = self.file_picker_files.get(file_idx).cloned()
            {
                if selected_path.is_dir() {
                    if selected_path.ends_with("..") {
                        if let Some(parent) = self.file_picker_current_dir.parent() {
                            self.file_picker_current_dir = parent.to_path_buf();
                        }
                    } else {
                        self.file_picker_current_dir = selected_path;
                    }
                    self.file_picker_search.clear();
                    self.refresh_file_picker().await?;
                } else {
                    let path_str = selected_path.to_string_lossy().to_string();
                    self.input_buffer
                        .insert_str(self.cursor_position, &path_str);
                    self.cursor_position += path_str.len();
                    self.file_picker_search.clear();
                    self.switch_mode(AppMode::Chat).await?;
                }
            }
        } else if event.code == KeyCode::Backspace {
            if self.file_picker_search.pop().is_some() {
                self.apply_file_picker_filter();
            }
        } else if let KeyCode::Char(c) = event.code {
            self.file_picker_search.push(c);
            self.apply_file_picker_filter();
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

            // Persist to session DB so it survives session switches
            if let Some(ref session) = self.current_session {
                let _ = self
                    .session_service
                    .update_session_working_directory(
                        session.id,
                        Some(canonical.to_string_lossy().to_string()),
                    )
                    .await;
            }

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
async fn test_telegram_connection(
    token: &str,
    user_id_str: &str,
    agent: std::sync::Arc<crate::brain::agent::AgentService>,
) -> Result<(), String> {
    use teloxide::prelude::Requester;

    let trimmed = user_id_str.trim();
    if trimmed.is_empty() {
        return Err("Chat ID is empty — paste your numeric chat ID \
                    (message @userinfobot on Telegram to get it)."
            .to_string());
    }
    let user_id: i64 = trimmed
        .parse()
        .map_err(|_| format!("Invalid chat ID '{}': must be a numeric ID.", trimmed))?;

    // Reject the bot's own numeric ID. Telegram bot tokens are
    // `<bot_id>:<secret>`, so users sometimes paste the bot id into the
    // chat id field by mistake. sendMessage to that id returns success
    // on some paths but never delivers a message anywhere visible.
    if let Some((bot_id_prefix, _)) = token.split_once(':')
        && let Ok(bot_id) = bot_id_prefix.trim().parse::<i64>()
        && bot_id == user_id
    {
        return Err(
            "That's the bot's own ID, not yours. Open Telegram, message \
             @userinfobot, and paste the numeric ID it replies with."
                .to_string(),
        );
    }

    let bot = teloxide::Bot::new(token);
    let greeting = crate::channels::generate_connection_greeting(&agent, "Telegram").await;
    bot.send_message(teloxide::types::ChatId(user_id), greeting)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("chat not found") {
                format!(
                    "Telegram says 'chat not found'. You must message your bot \
                     at least once first so it can deliver messages to you. \
                     Open Telegram, find your bot, send it any message, then retry. \
                     (raw: {})",
                    msg
                )
            } else if msg.contains("bot was blocked") {
                "You blocked the bot — unblock it in Telegram and retry.".to_string()
            } else {
                format!("Telegram API error: {}", msg)
            }
        })?;
    Ok(())
}

#[cfg(not(feature = "telegram"))]
async fn test_telegram_connection(
    _token: &str,
    _user_id_str: &str,
    _agent: std::sync::Arc<crate::brain::agent::AgentService>,
) -> Result<(), String> {
    Err("Telegram feature not enabled".to_string())
}

/// Test Discord connection by sending a message to a channel.
#[cfg(feature = "discord")]
async fn test_discord_connection(
    token: &str,
    channel_id_str: &str,
    agent: std::sync::Arc<crate::brain::agent::AgentService>,
) -> Result<(), String> {
    let channel_id: u64 = channel_id_str
        .parse()
        .map_err(|_| format!("Invalid channel ID: {}", channel_id_str))?;
    let greeting = crate::channels::generate_connection_greeting(&agent, "Discord").await;
    let http = serenity::http::Http::new(token);
    let channel = serenity::model::id::ChannelId::new(channel_id);
    channel
        .say(&http, greeting)
        .await
        .map_err(|e| format!("Discord API error: {}", e))?;
    Ok(())
}

#[cfg(not(feature = "discord"))]
async fn test_discord_connection(
    _token: &str,
    _channel_id_str: &str,
    _agent: std::sync::Arc<crate::brain::agent::AgentService>,
) -> Result<(), String> {
    Err("Discord feature not enabled".to_string())
}

/// Test Slack connection by posting a message to a channel.
#[cfg(feature = "slack")]
async fn test_slack_connection(
    token: &str,
    channel_id: &str,
    agent: std::sync::Arc<crate::brain::agent::AgentService>,
) -> Result<(), String> {
    use slack_morphism::prelude::*;

    let greeting = crate::channels::generate_connection_greeting(&agent, "Slack").await;
    let client = SlackClient::new(
        SlackClientHyperConnector::new().map_err(|e| format!("Slack client error: {}", e))?,
    );
    let api_token = SlackApiToken::new(SlackApiTokenValue::from(token.to_string()));
    let session = client.open_session(&api_token);
    let request = SlackApiChatPostMessageRequest::new(
        SlackChannelId::new(channel_id.to_string()),
        SlackMessageContent::new().with_text(greeting),
    );
    session
        .chat_post_message(&request)
        .await
        .map_err(|e| format!("Slack API error: {}", e))?;
    Ok(())
}

#[cfg(not(feature = "slack"))]
async fn test_slack_connection(
    _token: &str,
    _channel_id: &str,
    _agent: std::sync::Arc<crate::brain::agent::AgentService>,
) -> Result<(), String> {
    Err("Slack feature not enabled".to_string())
}

/// Test WhatsApp connection by sending a message using the paired bot's client.
#[cfg(feature = "whatsapp")]
async fn test_whatsapp_connection(
    wa_state: std::sync::Arc<crate::channels::whatsapp::WhatsAppState>,
    phone: &str,
    agent: std::sync::Arc<crate::brain::agent::AgentService>,
) -> Result<(), String> {
    // Wait for the agent bot to be connected (up to 15 seconds)
    let client = {
        let mut client = wa_state.client().await;
        if client.is_none() {
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                client = wa_state.client().await;
                if client.is_some() {
                    break;
                }
            }
        }
        client
            .ok_or_else(|| "WhatsApp not connected. Please scan the QR code first.".to_string())?
    };

    if phone.is_empty() {
        return Err("No phone number provided.".to_string());
    }

    let jid_str = format!("{}@s.whatsapp.net", phone.trim_start_matches('+'));
    let jid: wacore_binary::jid::Jid = jid_str
        .parse()
        .map_err(|e| format!("Invalid phone number format: {}", e))?;

    let greeting = crate::channels::generate_connection_greeting(&agent, "WhatsApp").await;
    let wa_msg = waproto::whatsapp::Message {
        conversation: Some(format!(
            "{}\n\n{}",
            crate::channels::whatsapp::handler::MSG_HEADER,
            greeting
        )),
        ..Default::default()
    };

    client
        .send_message(jid, wa_msg)
        .await
        .map_err(|e| format!("WhatsApp send error: {}", e))?;

    Ok(())
}

#[cfg(feature = "trello")]
async fn test_trello_connection(api_key: &str, api_token: &str) -> Result<(), String> {
    let client = crate::channels::trello::TrelloClient::new(api_key, api_token);
    client
        .get_member_me()
        .await
        .map(|_me| ())
        .map_err(|e| format!("Trello API error: {}", e))
}

#[cfg(not(feature = "trello"))]
async fn test_trello_connection(_api_key: &str, _api_token: &str) -> Result<(), String> {
    Err("Trello feature not enabled".to_string())
}
