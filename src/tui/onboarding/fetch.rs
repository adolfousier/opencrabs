use crossterm::event::{KeyCode, KeyEvent};

use super::types::*;
use super::wizard::OnboardingWizard;

impl OnboardingWizard {
    pub(super) fn handle_gateway_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.focused_field {
            0 => {
                // Port
                match event.code {
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        self.gateway_port.push(c);
                    }
                    KeyCode::Backspace => {
                        self.gateway_port.pop();
                    }
                    KeyCode::Tab | KeyCode::Enter => {
                        self.focused_field = 1;
                    }
                    _ => {}
                }
            }
            1 => {
                // Bind address
                match event.code {
                    KeyCode::Char(c) => {
                        self.gateway_bind.push(c);
                    }
                    KeyCode::Backspace => {
                        self.gateway_bind.pop();
                    }
                    KeyCode::Tab | KeyCode::Enter => {
                        self.focused_field = 2;
                    }
                    KeyCode::BackTab => {
                        self.focused_field = 0;
                    }
                    _ => {}
                }
            }
            2 => {
                // Auth mode
                match event.code {
                    KeyCode::Up | KeyCode::Down | KeyCode::Char(' ') => {
                        self.gateway_auth = if self.gateway_auth == 0 { 1 } else { 0 };
                    }
                    KeyCode::Enter => {
                        self.next_step();
                    }
                    KeyCode::BackTab => {
                        self.focused_field = 1;
                    }
                    _ => {}
                }
            }
            _ => {
                if event.code == KeyCode::Enter {
                    self.next_step();
                }
            }
        }
        WizardAction::None
    }

    pub(super) fn handle_voice_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.voice_field {
            VoiceField::GroqApiKey => match event.code {
                KeyCode::Char(c) => {
                    if self.has_existing_groq_key() {
                        self.groq_api_key_input.clear();
                    }
                    self.groq_api_key_input.push(c);
                }
                KeyCode::Backspace => {
                    if self.has_existing_groq_key() {
                        self.groq_api_key_input.clear();
                    } else {
                        self.groq_api_key_input.pop();
                    }
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.voice_field = VoiceField::TtsToggle;
                }
                _ => {}
            },
            VoiceField::TtsToggle => match event.code {
                KeyCode::Char(' ') | KeyCode::Up | KeyCode::Down => {
                    self.tts_enabled = !self.tts_enabled;
                }
                KeyCode::BackTab => {
                    self.voice_field = VoiceField::GroqApiKey;
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
            KeyCode::Enter => {
                if self.health_complete {
                    self.next_step();
                    return WizardAction::None;
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                // Re-run health checks
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
        || config.providers.gemini.as_ref().is_some_and(|p| p.enabled)
        || config
            .providers
            .openrouter
            .as_ref()
            .is_some_and(|p| p.enabled)
        || config.providers.minimax.as_ref().is_some_and(|p| p.enabled)
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

    // Handle Minimax specially - no /models API, must use config
    if provider_index == 4 {
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
        return vec!["MiniMax-M2.5".to_string(), "MiniMax-M2.1".to_string()];
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
        3 => {
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
