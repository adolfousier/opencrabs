use crossterm::event::{KeyCode, KeyEvent};

use super::types::*;
use super::wizard::OnboardingWizard;

impl OnboardingWizard {
    pub(super) fn handle_voice_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        super::voice::handle_key(self, event)
    }

    pub(super) fn handle_image_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        let either_enabled = self.image_vision_enabled || self.image_generation_enabled;

        match self.image_field {
            ImageField::VisionToggle => match event.code {
                KeyCode::Char(' ') | KeyCode::Up | KeyCode::Down => {
                    self.image_vision_enabled = !self.image_vision_enabled;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.image_field = ImageField::GenerationToggle;
                }
                _ => {}
            },
            ImageField::GenerationToggle => match event.code {
                KeyCode::Char(' ') | KeyCode::Up | KeyCode::Down => {
                    self.image_generation_enabled = !self.image_generation_enabled;
                }
                KeyCode::BackTab => {
                    self.image_field = ImageField::VisionToggle;
                }
                KeyCode::Tab | KeyCode::Enter => {
                    if either_enabled {
                        self.image_field = ImageField::ApiKey;
                    } else {
                        self.next_step();
                    }
                }
                _ => {}
            },
            ImageField::ApiKey => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_image_key() {
                        self.image_api_key_input.clear();
                    }
                    self.image_api_key_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_image_key() {
                        self.image_api_key_input.clear();
                    } else {
                        self.image_api_key_input.pop();
                    }
                }
                KeyCode::BackTab => {
                    self.image_field = ImageField::GenerationToggle;
                }
                KeyCode::Enter => {
                    self.next_step();
                }
                _ => {}
            },
        }
        WizardAction::None
    }

    pub(super) fn handle_daemon_key(&mut self, event: KeyEvent) -> WizardAction {
        match event.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Char(' ') => {
                self.install_daemon = !self.install_daemon;
            }
            KeyCode::Enter => {
                self.next_step();
            }
            _ => {}
        }
        WizardAction::None
    }

    pub(super) fn handle_health_check_key(&mut self, event: KeyEvent) -> WizardAction {
        match event.code {
            KeyCode::Enter if self.quick_jump && self.health_complete => {
                // Re-run checks on Enter after complete
                self.start_health_check();
            }
            KeyCode::Enter if self.health_complete => {
                self.next_step();
                return WizardAction::None;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.start_health_check();
            }
            _ => {}
        }
        WizardAction::None
    }
}

/// First-time detection: no config file AND no API keys in environment.
/// Once config.toml is written (by onboarding or manually), this returns false forever.
/// If any API key env var is set, the user has already configured auth — skip onboarding.
/// To re-run the wizard, use `opencrabs onboard`, `--onboard` flag, or `/onboard`.
pub fn is_first_time() -> bool {
    tracing::debug!("[is_first_time] checking if first time setup needed...");

    // Check if config exists
    let config_path = crate::config::opencrabs_home().join("config.toml");
    if !config_path.exists() {
        tracing::debug!("[is_first_time] no config found, need onboarding");
        return true;
    }

    // Config exists - check if any provider is actually enabled
    let config = match crate::config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(
                "[is_first_time] failed to load config: {}, need onboarding",
                e
            );
            return true;
        }
    };

    let has_enabled_provider = config
        .providers
        .anthropic
        .as_ref()
        .is_some_and(|p| p.enabled)
        || config.providers.openai.as_ref().is_some_and(|p| p.enabled)
        || config.providers.github.as_ref().is_some_and(|p| p.enabled)
        || config.providers.gemini.as_ref().is_some_and(|p| p.enabled)
        || config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled)
        || config.providers.minimax.as_ref().is_some_and(|p| p.enabled)
        || config
            .providers
            .claude_cli
            .as_ref()
            .is_some_and(|p| p.enabled)
        || config.providers.active_custom().is_some();

    tracing::debug!(
        "[is_first_time] has_enabled_provider={}, result={}",
        has_enabled_provider,
        !has_enabled_provider
    );
    !has_enabled_provider
}

/// Fetch models from provider API. No API key needed for most providers.
/// If api_key is provided, includes it (some endpoints filter by access level).
/// Returns empty vec on failure (callers fall back to static list).
pub async fn fetch_provider_models(provider_index: usize, api_key: Option<&str>) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    // Claude CLI — models are fixed (sonnet/opus/haiku), no API needed
    if provider_index == 6 {
        return vec![
            "sonnet".to_string(),
            "opus".to_string(),
            "haiku".to_string(),
        ];
    }

    // Handle Minimax specially - no /models API, must use config
    if provider_index == 5 {
        // Minimax — NO /models API endpoint, must use config.models
        if let Ok(config) = crate::config::Config::load()
            && let Some(p) = &config.providers.minimax
        {
            if !p.models.is_empty() {
                return p.models.clone();
            }
            // Fall back to default_model if no models list
            if let Some(model) = &p.default_model {
                return vec![model.clone()];
            }
        }
        // Return hardcoded defaults if no config
        return vec![
            "MiniMax-M2.7".to_string(),
            "MiniMax-M2.5".to_string(),
            "MiniMax-M2.1".to_string(),
        ];
    }

    let client = reqwest::Client::new();

    let result = match provider_index {
        0 => {
            // Anthropic — /v1/models is public
            let mut req = client
                .get("https://api.anthropic.com/v1/models")
                .header("anthropic-version", "2023-06-01");

            // Include key if available (may show more models)
            if let Some(key) = api_key {
                if key.starts_with("sk-ant-oat") {
                    req = req
                        .header("Authorization", format!("Bearer {}", key))
                        .header("anthropic-beta", "oauth-2025-04-20");
                } else if !key.is_empty() {
                    req = req.header("x-api-key", key);
                }
            }

            req.send().await
        }
        1 => {
            // OpenAI — /v1/models
            let mut req = client.get("https://api.openai.com/v1/models");
            if let Some(key) = api_key
                && !key.is_empty()
            {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            req.send().await
        }
        2 => {
            // GitHub Copilot — fetch from Copilot API using OAuth token
            if let Some(key) = api_key
                && !key.is_empty()
            {
                match crate::brain::provider::copilot::fetch_copilot_models(key).await {
                    Ok(models) if !models.is_empty() => return models,
                    Ok(_) => tracing::debug!("Copilot models endpoint returned empty list"),
                    Err(e) => tracing::debug!("Copilot models fetch failed: {}", e),
                }
            }
            // Fall back to config or defaults
            if let Ok(config) = crate::config::Config::load()
                && let Some(p) = &config.providers.github
            {
                if !p.models.is_empty() {
                    return p.models.clone();
                }
                if let Some(model) = &p.default_model {
                    return vec![model.clone()];
                }
            }
            return OnboardingWizard::load_default_models(2);
        }
        4 => {
            // OpenRouter — /api/v1/models
            let mut req = client.get("https://openrouter.ai/api/v1/models");
            if let Some(key) = api_key
                && !key.is_empty()
            {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            req.send().await
        }
        _ => return Vec::new(),
    };

    match result {
        Ok(resp) if resp.status().is_success() => match resp.json::<ModelsResponse>().await {
            Ok(body) => {
                let mut models: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
                models.sort();
                models
            }
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    }
}
