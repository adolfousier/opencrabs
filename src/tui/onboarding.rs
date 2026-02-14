//! Onboarding Wizard
//!
//! A 7-step TUI-based onboarding wizard for first-time OpenCrabs users.
//! Handles mode selection, provider/auth setup, workspace, gateway,
//! channels, daemon installation, and health check.

use crate::config::{
    ChannelConfig, ChannelsConfig, Config, GatewayConfig, ProviderConfig, QwenProviderConfig,
};
use crate::config::secrets::SecretString;
use chrono::Local;

/// Sentinel value stored in api_key_input when a key was loaded from env/keyring.
/// The actual key is never held in memory — this just signals "key exists".
const EXISTING_KEY_SENTINEL: &str = "__EXISTING_KEY__";
use crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;

/// Provider definitions: (display name, env var key, keyring key, available models, help text)
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        name: "Anthropic Claude (recommended)",
        env_vars: &["ANTHROPIC_MAX_SETUP_TOKEN", "ANTHROPIC_API_KEY"],
        keyring_key: "anthropic_api_key",
        models: &[
            "claude-opus-4-6",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
        ],
        key_label: "Setup Token",
        help_lines: &[
            "Claude Max / Code subscribers:",
            "  Run: claude setup-token",
            "  Copy the token (starts with sk-ant-oat...)",
            "",
            "Or paste an API key from console.anthropic.com",
        ],
    },
    ProviderInfo {
        name: "OpenAI",
        env_vars: &["OPENAI_API_KEY"],
        keyring_key: "openai_api_key",
        models: &["gpt-5.1-codex-mini"],
        key_label: "API Key",
        help_lines: &[
            "Get your API key from platform.openai.com",
        ],
    },
    ProviderInfo {
        name: "Google Gemini",
        env_vars: &["GEMINI_API_KEY"],
        keyring_key: "gemini_api_key",
        models: &["gemini-3-flash-preview"],
        key_label: "API Key",
        help_lines: &[
            "Get your API key from aistudio.google.com",
        ],
    },
    ProviderInfo {
        name: "Qwen/DashScope",
        env_vars: &["DASHSCOPE_API_KEY"],
        keyring_key: "dashscope_api_key",
        models: &["qwen3-coder-next"],
        key_label: "API Key",
        help_lines: &[
            "Get your API key from dashscope.console.aliyun.com",
        ],
    },
    ProviderInfo {
        name: "Custom (OpenAI-compatible)",
        env_vars: &[],
        keyring_key: "",
        models: &[],
        key_label: "API Key",
        help_lines: &[
            "Enter base URL and model for any OpenAI-compatible API",
        ],
    },
];

pub struct ProviderInfo {
    pub name: &'static str,
    pub env_vars: &'static [&'static str],
    pub keyring_key: &'static str,
    pub models: &'static [&'static str],
    pub key_label: &'static str,
    pub help_lines: &'static [&'static str],
}

/// Channel definitions for step 5
pub const CHANNEL_NAMES: &[&str] = &[
    "Telegram",
    "Discord",
    "WhatsApp",
    "Signal",
    "Google Chat",
    "iMessage",
];

/// Template files to seed in the workspace
const TEMPLATE_FILES: &[(&str, &str)] = &[
    ("SOUL.md", include_str!("../docs/reference/templates/SOUL.md")),
    (
        "IDENTITY.md",
        include_str!("../docs/reference/templates/IDENTITY.md"),
    ),
    ("USER.md", include_str!("../docs/reference/templates/USER.md")),
    (
        "AGENTS.md",
        include_str!("../docs/reference/templates/AGENTS.md"),
    ),
    (
        "TOOLS.md",
        include_str!("../docs/reference/templates/TOOLS.md"),
    ),
    (
        "MEMORY.md",
        include_str!("../docs/reference/templates/MEMORY.md"),
    ),
];

/// Current step in the onboarding wizard
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    ModeSelect,
    ProviderAuth,
    Workspace,
    Gateway,
    Channels,
    Daemon,
    HealthCheck,
    BrainSetup,
    Complete,
}

impl OnboardingStep {
    /// Step number (1-based)
    pub fn number(&self) -> usize {
        match self {
            Self::ModeSelect => 1,
            Self::ProviderAuth => 2,
            Self::Workspace => 3,
            Self::Gateway => 4,
            Self::Channels => 5,
            Self::Daemon => 6,
            Self::HealthCheck => 7,
            Self::BrainSetup => 8,
            Self::Complete => 9,
        }
    }

    /// Total number of steps (excluding Complete)
    pub fn total() -> usize {
        8
    }

    /// Step title
    pub fn title(&self) -> &'static str {
        match self {
            Self::ModeSelect => "Mode Selection",
            Self::ProviderAuth => "Model & Authentication",
            Self::Workspace => "Workspace Setup",
            Self::Gateway => "Gateway Configuration",
            Self::Channels => "Channel Integrations",
            Self::Daemon => "Background Service",
            Self::HealthCheck => "Health Check",
            Self::BrainSetup => "Brain Personalization",
            Self::Complete => "Complete",
        }
    }

    /// Step subtitle
    pub fn subtitle(&self) -> &'static str {
        match self {
            Self::ModeSelect => "Choose your setup experience",
            Self::ProviderAuth => "Choose your AI provider and authenticate",
            Self::Workspace => "Set up your brain workspace directory",
            Self::Gateway => "Configure the HTTP API gateway",
            Self::Channels => "Enable messaging integrations",
            Self::Daemon => "Install background service",
            Self::HealthCheck => "Verify your setup",
            Self::BrainSetup => "Let your agent actually know who you are",
            Self::Complete => "Setup complete!",
        }
    }
}

/// Wizard mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardMode {
    QuickStart,
    Advanced,
}

/// Health check status for individual checks
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Pending,
    Running,
    Pass,
    Fail(String),
}

/// Which field is being actively edited in ProviderAuth step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthField {
    Provider,
    ApiKey,
    Model,
    CustomBaseUrl,
    CustomModel,
}

/// Which text area is focused in BrainSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrainField {
    AboutMe,
    AboutAgent,
}

/// Main onboarding wizard state
pub struct OnboardingWizard {
    pub step: OnboardingStep,
    pub mode: WizardMode,

    // Step 2: Provider/Auth
    pub selected_provider: usize,
    pub api_key_input: String,
    pub api_key_cursor: usize,
    pub selected_model: usize,
    pub auth_field: AuthField,
    pub custom_base_url: String,
    pub custom_model: String,

    // Step 3: Workspace
    pub workspace_path: String,
    pub seed_templates: bool,

    // Step 4: Gateway
    pub gateway_port: String,
    pub gateway_bind: String,
    pub gateway_auth: usize, // 0=Token, 1=None

    // Step 5: Channels
    pub channel_toggles: Vec<(String, bool)>,

    // Step 6: Daemon
    pub install_daemon: bool,

    // Step 7: Health check
    pub health_results: Vec<(String, HealthStatus)>,
    pub health_running: bool,
    pub health_complete: bool,

    // Step 8: Brain Setup
    pub brain_field: BrainField,
    pub about_me: String,
    pub about_agent: String,
    pub brain_generating: bool,
    pub brain_generated: bool,
    pub brain_error: Option<String>,
    pub generated_soul: Option<String>,
    pub generated_identity: Option<String>,
    pub generated_user: Option<String>,
    pub generated_agents: Option<String>,
    pub generated_tools: Option<String>,
    pub generated_memory: Option<String>,

    // Navigation
    pub focused_field: usize,
    pub error_message: Option<String>,
}

impl Default for OnboardingWizard {
    fn default() -> Self {
        Self::new()
    }
}

impl OnboardingWizard {
    /// Create a new wizard with default state
    pub fn new() -> Self {
        let default_workspace = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join("opencrabs")
            .join("brain")
            .join("workspace");

        Self {
            step: OnboardingStep::ModeSelect,
            mode: WizardMode::QuickStart,

            selected_provider: 0,
            api_key_input: String::new(),
            api_key_cursor: 0,
            selected_model: 0,
            auth_field: AuthField::Provider,
            custom_base_url: String::new(),
            custom_model: String::new(),

            workspace_path: default_workspace.to_string_lossy().to_string(),
            seed_templates: true,

            gateway_port: "18789".to_string(),
            gateway_bind: "127.0.0.1".to_string(),
            gateway_auth: 0,

            channel_toggles: CHANNEL_NAMES
                .iter()
                .map(|name| (name.to_string(), false))
                .collect(),

            install_daemon: false,

            health_results: Vec::new(),
            health_running: false,
            health_complete: false,

            brain_field: BrainField::AboutMe,
            about_me: String::new(),
            about_agent: String::new(),
            brain_generating: false,
            brain_generated: false,
            brain_error: None,
            generated_soul: None,
            generated_identity: None,
            generated_user: None,
            generated_agents: None,
            generated_tools: None,
            generated_memory: None,

            focused_field: 0,
            error_message: None,
        }
    }

    /// Get provider info for currently selected provider
    pub fn current_provider(&self) -> &ProviderInfo {
        &PROVIDERS[self.selected_provider]
    }

    /// Check if the current provider is the "Custom" option
    pub fn is_custom_provider(&self) -> bool {
        self.selected_provider == PROVIDERS.len() - 1
    }

    /// Whether the current api_key_input holds a pre-existing key (from env/keyring)
    pub fn has_existing_key(&self) -> bool {
        self.api_key_input == EXISTING_KEY_SENTINEL
    }

    /// Try to load an existing API key for the currently selected provider.
    /// Checks keyring first, then all env vars. If found, sets sentinel.
    pub fn detect_existing_key(&mut self) {
        let provider = &PROVIDERS[self.selected_provider];
        if provider.env_vars.is_empty() && provider.keyring_key.is_empty() {
            return;
        }
        // Try keyring first
        if !provider.keyring_key.is_empty()
            && SecretString::from_keyring_optional(provider.keyring_key).is_some() {
                self.api_key_input = EXISTING_KEY_SENTINEL.to_string();
                self.api_key_cursor = 0;
                return;
            }
        // Then try each env var
        for env_var in provider.env_vars {
            if SecretString::from_env_optional(env_var).is_some() {
                self.api_key_input = EXISTING_KEY_SENTINEL.to_string();
                self.api_key_cursor = 0;
                return;
            }
        }
    }

    /// Advance to the next step
    pub fn next_step(&mut self) {
        self.error_message = None;
        self.focused_field = 0;

        match self.step {
            OnboardingStep::ModeSelect => {
                self.step = OnboardingStep::ProviderAuth;
                self.auth_field = AuthField::Provider;
                self.detect_existing_key();
            }
            OnboardingStep::ProviderAuth => {
                // Validate API key is provided
                if self.api_key_input.is_empty() && !self.is_custom_provider() {
                    self.error_message = Some("API key is required".to_string());
                    return;
                }
                if self.is_custom_provider()
                    && (self.custom_base_url.is_empty() || self.custom_model.is_empty())
                {
                    self.error_message =
                        Some("Base URL and model name are required for custom provider".to_string());
                    return;
                }
                self.step = OnboardingStep::Workspace;
            }
            OnboardingStep::Workspace => {
                if self.mode == WizardMode::QuickStart {
                    // Skip steps 4-6, go straight to health check
                    self.step = OnboardingStep::HealthCheck;
                    self.start_health_check();
                } else {
                    self.step = OnboardingStep::Gateway;
                }
            }
            OnboardingStep::Gateway => {
                self.step = OnboardingStep::Channels;
            }
            OnboardingStep::Channels => {
                self.step = OnboardingStep::Daemon;
            }
            OnboardingStep::Daemon => {
                self.step = OnboardingStep::HealthCheck;
                self.start_health_check();
            }
            OnboardingStep::HealthCheck => {
                self.step = OnboardingStep::BrainSetup;
                self.brain_field = BrainField::AboutMe;
            }
            OnboardingStep::BrainSetup => {
                if self.brain_generated || self.brain_error.is_some() {
                    self.step = OnboardingStep::Complete;
                }
                // Otherwise wait for generation to finish or user to trigger it
            }
            OnboardingStep::Complete => {
                // Already complete
            }
        }
    }

    /// Go back to the previous step
    pub fn prev_step(&mut self) -> bool {
        self.error_message = None;
        self.focused_field = 0;

        match self.step {
            OnboardingStep::ModeSelect => {
                // Can't go back further — return true to signal "cancel wizard"
                return true;
            }
            OnboardingStep::ProviderAuth => {
                self.step = OnboardingStep::ModeSelect;
            }
            OnboardingStep::Workspace => {
                self.step = OnboardingStep::ProviderAuth;
                self.auth_field = AuthField::Provider;
            }
            OnboardingStep::Gateway => {
                self.step = OnboardingStep::Workspace;
            }
            OnboardingStep::Channels => {
                self.step = OnboardingStep::Gateway;
            }
            OnboardingStep::Daemon => {
                self.step = OnboardingStep::Channels;
            }
            OnboardingStep::HealthCheck => {
                if self.mode == WizardMode::QuickStart {
                    self.step = OnboardingStep::Workspace;
                } else {
                    self.step = OnboardingStep::Daemon;
                }
            }
            OnboardingStep::BrainSetup => {
                self.step = OnboardingStep::HealthCheck;
                self.brain_generating = false;
                self.brain_error = None;
            }
            OnboardingStep::Complete => {
                self.step = OnboardingStep::BrainSetup;
                self.brain_field = BrainField::AboutMe;
            }
        }
        false
    }

    /// Initialize health check results
    fn start_health_check(&mut self) {
        self.health_results = vec![
            ("API Key Present".to_string(), HealthStatus::Pending),
            ("Config File".to_string(), HealthStatus::Pending),
            ("Workspace Directory".to_string(), HealthStatus::Pending),
            ("Template Files".to_string(), HealthStatus::Pending),
        ];
        self.health_running = true;
        self.health_complete = false;

        // Run health checks synchronously (they're fast local checks)
        self.run_health_checks();
    }

    /// Execute all health checks
    fn run_health_checks(&mut self) {
        // Check 1: API key present
        self.health_results[0].1 = if !self.api_key_input.is_empty()
            || (self.is_custom_provider() && !self.custom_base_url.is_empty())
        {
            HealthStatus::Pass
        } else {
            HealthStatus::Fail("No API key provided".to_string())
        };

        // Check 2: Config path writable
        let config_path = dirs::config_dir()
            .map(|d| d.join("opencrabs").join("config.toml"))
            .unwrap_or_else(|| PathBuf::from("~/.config/opencrabs/config.toml"));
        self.health_results[1].1 = if let Some(parent) = config_path.parent() {
            if parent.exists() || std::fs::create_dir_all(parent).is_ok() {
                HealthStatus::Pass
            } else {
                HealthStatus::Fail(format!("Cannot create {}", parent.display()))
            }
        } else {
            HealthStatus::Fail("Invalid config path".to_string())
        };

        // Check 3: Workspace directory
        let workspace = PathBuf::from(&self.workspace_path);
        self.health_results[2].1 = if workspace.exists() || std::fs::create_dir_all(&workspace).is_ok() {
            HealthStatus::Pass
        } else {
            HealthStatus::Fail(format!("Cannot create {}", workspace.display()))
        };

        // Check 4: Template files available (they're compiled in, always present)
        self.health_results[3].1 = HealthStatus::Pass;

        self.health_running = false;
        self.health_complete = true;
    }

    /// Check if all health checks passed
    pub fn all_health_passed(&self) -> bool {
        self.health_complete
            && self.health_results.iter().all(|(_, s)| matches!(s, HealthStatus::Pass))
    }

    /// Handle key events for the current step
    /// Returns `WizardAction` indicating what the app should do
    pub fn handle_key(&mut self, event: KeyEvent) -> WizardAction {
        // Global: Escape goes back
        if event.code == KeyCode::Esc {
            if self.prev_step() {
                return WizardAction::Cancel;
            }
            return WizardAction::None;
        }

        match self.step {
            OnboardingStep::ModeSelect => self.handle_mode_select_key(event),
            OnboardingStep::ProviderAuth => self.handle_provider_auth_key(event),
            OnboardingStep::Workspace => self.handle_workspace_key(event),
            OnboardingStep::Gateway => self.handle_gateway_key(event),
            OnboardingStep::Channels => self.handle_channels_key(event),
            OnboardingStep::Daemon => self.handle_daemon_key(event),
            OnboardingStep::HealthCheck => self.handle_health_check_key(event),
            OnboardingStep::BrainSetup => self.handle_brain_setup_key(event),
            OnboardingStep::Complete => WizardAction::Complete,
        }
    }

    // --- Step-specific key handlers ---

    fn handle_mode_select_key(&mut self, event: KeyEvent) -> WizardAction {
        match event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.mode = WizardMode::QuickStart;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.mode = WizardMode::Advanced;
            }
            KeyCode::Char('1') => {
                self.mode = WizardMode::QuickStart;
            }
            KeyCode::Char('2') => {
                self.mode = WizardMode::Advanced;
            }
            KeyCode::Enter => {
                self.next_step();
            }
            _ => {}
        }
        WizardAction::None
    }

    fn handle_provider_auth_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.auth_field {
            AuthField::Provider => match event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.selected_provider = self.selected_provider.saturating_sub(1);
                    self.selected_model = 0;
                    self.api_key_input.clear();
                    self.detect_existing_key();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selected_provider =
                        (self.selected_provider + 1).min(PROVIDERS.len() - 1);
                    self.selected_model = 0;
                    self.api_key_input.clear();
                    self.detect_existing_key();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.detect_existing_key();
                    if self.is_custom_provider() {
                        self.auth_field = AuthField::CustomBaseUrl;
                    } else {
                        self.auth_field = AuthField::ApiKey;
                    }
                }
                _ => {}
            },
            AuthField::ApiKey => match event.code {
                KeyCode::Char(c) => {
                    // If existing key is loaded and user starts typing, clear it (replace mode)
                    if self.has_existing_key() {
                        self.api_key_input.clear();
                    }
                    self.api_key_input.push(c);
                    self.api_key_cursor = self.api_key_input.len();
                }
                KeyCode::Backspace => {
                    // If existing key sentinel, clear entirely on backspace
                    if self.has_existing_key() {
                        self.api_key_input.clear();
                    } else {
                        self.api_key_input.pop();
                    }
                    self.api_key_cursor = self.api_key_input.len();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.auth_field = AuthField::Model;
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::Provider;
                }
                _ => {}
            },
            AuthField::Model => match event.code {
                KeyCode::Up | KeyCode::Char('k')
                    if event.modifiers.is_empty() || event.code == KeyCode::Up =>
                {
                    self.selected_model = self.selected_model.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j')
                    if event.modifiers.is_empty() || event.code == KeyCode::Down =>
                {
                    let models = self.current_provider().models;
                    if !models.is_empty() {
                        self.selected_model =
                            (self.selected_model + 1).min(models.len() - 1);
                    }
                }
                KeyCode::Enter => {
                    self.next_step();
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::ApiKey;
                }
                KeyCode::Tab => {
                    self.next_step();
                }
                _ => {}
            },
            AuthField::CustomBaseUrl => match event.code {
                KeyCode::Char(c) => {
                    self.custom_base_url.push(c);
                }
                KeyCode::Backspace => {
                    self.custom_base_url.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.auth_field = AuthField::CustomModel;
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::Provider;
                }
                _ => {}
            },
            AuthField::CustomModel => match event.code {
                KeyCode::Char(c) => {
                    self.custom_model.push(c);
                }
                KeyCode::Backspace => {
                    self.custom_model.pop();
                }
                KeyCode::Enter => {
                    self.next_step();
                }
                KeyCode::BackTab => {
                    self.auth_field = AuthField::CustomBaseUrl;
                }
                KeyCode::Tab => {
                    self.next_step();
                }
                _ => {}
            },
        }
        WizardAction::None
    }

    fn handle_workspace_key(&mut self, event: KeyEvent) -> WizardAction {
        match self.focused_field {
            0 => {
                // Editing workspace path
                match event.code {
                    KeyCode::Char(c) => {
                        self.workspace_path.push(c);
                    }
                    KeyCode::Backspace => {
                        self.workspace_path.pop();
                    }
                    KeyCode::Tab => {
                        self.focused_field = 1;
                    }
                    KeyCode::Enter => {
                        self.next_step();
                    }
                    _ => {}
                }
            }
            1 => {
                // Seed templates toggle
                match event.code {
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        self.seed_templates = !self.seed_templates;
                    }
                    KeyCode::Tab => {
                        self.focused_field = 2;
                    }
                    KeyCode::BackTab => {
                        self.focused_field = 0;
                    }
                    _ => {}
                }
            }
            _ => {
                // "Next" button
                match event.code {
                    KeyCode::Enter => {
                        self.next_step();
                    }
                    KeyCode::BackTab => {
                        self.focused_field = 1;
                    }
                    _ => {}
                }
            }
        }
        WizardAction::None
    }

    fn handle_gateway_key(&mut self, event: KeyEvent) -> WizardAction {
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

    fn handle_channels_key(&mut self, event: KeyEvent) -> WizardAction {
        let count = self.channel_toggles.len();
        match event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.focused_field = self.focused_field.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.focused_field = (self.focused_field + 1).min(count.saturating_sub(1));
            }
            KeyCode::Char(' ') => {
                if self.focused_field < count {
                    self.channel_toggles[self.focused_field].1 =
                        !self.channel_toggles[self.focused_field].1;
                }
            }
            KeyCode::Enter => {
                self.next_step();
            }
            _ => {}
        }
        WizardAction::None
    }

    fn handle_daemon_key(&mut self, event: KeyEvent) -> WizardAction {
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

    fn handle_health_check_key(&mut self, event: KeyEvent) -> WizardAction {
        match event.code {
            KeyCode::Enter => {
                if self.health_complete {
                    self.next_step();
                    return WizardAction::Complete;
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

    fn handle_brain_setup_key(&mut self, event: KeyEvent) -> WizardAction {
        // Don't accept input while generating
        if self.brain_generating {
            return WizardAction::None;
        }

        // If already generated or errored, Enter advances
        if self.brain_generated || self.brain_error.is_some() {
            if event.code == KeyCode::Enter {
                self.next_step();
                return WizardAction::Complete;
            }
            return WizardAction::None;
        }

        match event.code {
            KeyCode::Tab => {
                self.brain_field = match self.brain_field {
                    BrainField::AboutMe => BrainField::AboutAgent,
                    BrainField::AboutAgent => BrainField::AboutMe,
                };
            }
            KeyCode::BackTab => {
                self.brain_field = match self.brain_field {
                    BrainField::AboutMe => BrainField::AboutAgent,
                    BrainField::AboutAgent => BrainField::AboutMe,
                };
            }
            KeyCode::Enter => {
                // Ctrl+Enter or Enter on AboutAgent triggers generation
                if self.brain_field == BrainField::AboutAgent {
                    if self.about_me.is_empty() && self.about_agent.is_empty() {
                        self.error_message = Some("Drop some info first — don't leave your agent hanging".to_string());
                        return WizardAction::None;
                    }
                    return WizardAction::GenerateBrain;
                }
                // Enter on AboutMe moves to AboutAgent
                self.brain_field = BrainField::AboutAgent;
            }
            KeyCode::Char(c) => {
                self.active_brain_field_mut().push(c);
            }
            KeyCode::Backspace => {
                self.active_brain_field_mut().pop();
            }
            _ => {}
        }
        WizardAction::None
    }

    /// Get mutable reference to the currently focused brain text area
    fn active_brain_field_mut(&mut self) -> &mut String {
        match self.brain_field {
            BrainField::AboutMe => &mut self.about_me,
            BrainField::AboutAgent => &mut self.about_agent,
        }
    }

    /// Build the prompt sent to the AI to generate personalized brain files
    pub fn build_brain_prompt(&self) -> String {
        let today = Local::now().format("%Y-%m-%d").to_string();

        let soul_template = include_str!("../docs/reference/templates/SOUL.md");
        let identity_template = include_str!("../docs/reference/templates/IDENTITY.md");
        let user_template = include_str!("../docs/reference/templates/USER.md");
        let agents_template = include_str!("../docs/reference/templates/AGENTS.md");
        let tools_template = include_str!("../docs/reference/templates/TOOLS.md");
        let memory_template = include_str!("../docs/reference/templates/MEMORY.md");

        format!(
            r#"You are setting up a personal AI agent's brain — its entire workspace of markdown files that define who it is, who its human is, and how it operates.

The user dumped two blocks of info. One about themselves (name, role, links, projects, whatever they shared). One about how they want their agent to be (personality, vibe, behavior). Use EVERYTHING they gave you to personalize ALL six template files below.

=== ABOUT THE USER ===
{about_me}

=== ABOUT THE AGENT ===
{about_agent}

=== TODAY'S DATE ===
{date}

Below are the 6 template files. Replace ALL <placeholder> tags and HTML comments with real values based on what the user provided. Keep the exact markdown structure. Fill what you can from the user's info, leave sensible defaults for anything not provided. Don't invent facts — if the user didn't mention something, use a reasonable placeholder like "TBD" or remove that line.

===TEMPLATE: SOUL.md===
{soul}

===TEMPLATE: IDENTITY.md===
{identity}

===TEMPLATE: USER.md===
{user}

===TEMPLATE: AGENTS.md===
{agents}

===TEMPLATE: TOOLS.md===
{tools}

===TEMPLATE: MEMORY.md===
{memory}

Respond with EXACTLY six sections using these delimiters. No extra text before the first delimiter or after the last section:
---SOUL---
(generated SOUL.md content)
---IDENTITY---
(generated IDENTITY.md content)
---USER---
(generated USER.md content)
---AGENTS---
(generated AGENTS.md content)
---TOOLS---
(generated TOOLS.md content)
---MEMORY---
(generated MEMORY.md content)"#,
            about_me = if self.about_me.is_empty() { "Not provided" } else { &self.about_me },
            about_agent = if self.about_agent.is_empty() { "Not provided" } else { &self.about_agent },
            date = today,
            soul = soul_template,
            identity = identity_template,
            user = user_template,
            agents = agents_template,
            tools = tools_template,
            memory = memory_template,
        )
    }

    /// Store the generated brain content from the AI response
    pub fn apply_generated_brain(&mut self, response: &str) {
        // Parse the response into six sections using delimiters
        let delimiters = [
            "---SOUL---",
            "---IDENTITY---",
            "---USER---",
            "---AGENTS---",
            "---TOOLS---",
            "---MEMORY---",
        ];

        // Find all delimiter positions
        let positions: Vec<Option<usize>> = delimiters.iter().map(|d| response.find(d)).collect();

        // Need at least SOUL, IDENTITY, USER to consider it a success
        if positions[0].is_none() || positions[1].is_none() || positions[2].is_none() {
            self.brain_error = Some("Couldn't parse AI response — using defaults".to_string());
            self.brain_generating = false;
            return;
        }

        // Extract content between delimiters
        // Build ordered list of (delimiter_index, position) sorted by position
        let mut ordered: Vec<(usize, usize)> = positions
            .iter()
            .enumerate()
            .filter_map(|(i, pos)| pos.map(|p| (i, p)))
            .collect();
        ordered.sort_by_key(|(_, pos)| *pos);

        for (idx, &(delim_idx, pos)) in ordered.iter().enumerate() {
            let start = pos + delimiters[delim_idx].len();
            let end = if idx + 1 < ordered.len() {
                ordered[idx + 1].1
            } else {
                response.len()
            };
            let content = response[start..end].trim();

            if !content.is_empty() {
                match delim_idx {
                    0 => self.generated_soul = Some(content.to_string()),
                    1 => self.generated_identity = Some(content.to_string()),
                    2 => self.generated_user = Some(content.to_string()),
                    3 => self.generated_agents = Some(content.to_string()),
                    4 => self.generated_tools = Some(content.to_string()),
                    5 => self.generated_memory = Some(content.to_string()),
                    _ => {}
                }
            }
        }

        self.brain_generated = true;
        self.brain_generating = false;
    }

    /// Apply wizard configuration — creates config.toml, stores API key, seeds workspace
    pub fn apply_config(&self) -> Result<(), String> {
        let mut config = Config::default();

        // Provider config (indices match PROVIDERS array: 0=Anthropic, 1=OpenAI, 2=Gemini, 3=Qwen, 4=Custom)
        match self.selected_provider {
            0 => {
                // Anthropic Claude (setup token or API key)
                let provider = &PROVIDERS[0];
                let model = provider
                    .models
                    .get(self.selected_model)
                    .unwrap_or(&"claude-opus-4-6");
                config.providers.anthropic = Some(ProviderConfig {
                    enabled: true,
                    api_key: None, // stored in keyring or env
                    base_url: None,
                    default_model: Some(model.to_string()),
                });
            }
            1 => {
                // OpenAI
                let model = PROVIDERS[1]
                    .models
                    .get(self.selected_model)
                    .unwrap_or(&"gpt-5.1-codex-mini");
                config.providers.openai = Some(ProviderConfig {
                    enabled: true,
                    api_key: None,
                    base_url: None,
                    default_model: Some(model.to_string()),
                });
            }
            2 => {
                // Gemini
                let model = PROVIDERS[2]
                    .models
                    .get(self.selected_model)
                    .unwrap_or(&"gemini-3-flash-preview");
                config.providers.gemini = Some(ProviderConfig {
                    enabled: true,
                    api_key: None,
                    base_url: None,
                    default_model: Some(model.to_string()),
                });
            }
            3 => {
                // Qwen/DashScope
                let model = PROVIDERS[3]
                    .models
                    .get(self.selected_model)
                    .unwrap_or(&"qwen3-coder-next");
                config.providers.qwen = Some(QwenProviderConfig {
                    enabled: true,
                    api_key: None,
                    base_url: None,
                    default_model: Some(model.to_string()),
                    tool_parser: None,
                    enable_thinking: false,
                    thinking_budget: None,
                    region: None,
                });
            }
            4 => {
                // Custom OpenAI-compatible
                config.providers.openai = Some(ProviderConfig {
                    enabled: true,
                    api_key: None,
                    base_url: Some(self.custom_base_url.clone()),
                    default_model: Some(self.custom_model.clone()),
                });
            }
            _ => {}
        }

        // Gateway config
        config.gateway = GatewayConfig {
            port: self.gateway_port.parse().unwrap_or(18789),
            bind: self.gateway_bind.clone(),
            auth_mode: if self.gateway_auth == 0 {
                "token".to_string()
            } else {
                "none".to_string()
            },
            enabled: false,
        };

        // Channels config
        config.channels = ChannelsConfig {
            telegram: ChannelConfig {
                enabled: self.channel_toggles.first().is_some_and(|t| t.1),
                token: None,
                allowed_users: Vec::new(),
            },
            discord: ChannelConfig {
                enabled: self.channel_toggles.get(1).is_some_and(|t| t.1),
                token: None,
                allowed_users: Vec::new(),
            },
            whatsapp: ChannelConfig {
                enabled: self.channel_toggles.get(2).is_some_and(|t| t.1),
                token: None,
                allowed_users: Vec::new(),
            },
            signal: ChannelConfig {
                enabled: self.channel_toggles.get(3).is_some_and(|t| t.1),
                token: None,
                allowed_users: Vec::new(),
            },
            google_chat: ChannelConfig {
                enabled: self.channel_toggles.get(4).is_some_and(|t| t.1),
                token: None,
                allowed_users: Vec::new(),
            },
            imessage: ChannelConfig {
                enabled: self.channel_toggles.get(5).is_some_and(|t| t.1),
                token: None,
                allowed_users: Vec::new(),
            },
        };

        // Write config.toml
        let config_path = dirs::config_dir()
            .ok_or_else(|| "Cannot determine config directory".to_string())?
            .join("opencrabs")
            .join("config.toml");

        config
            .save(&config_path)
            .map_err(|e| format!("Failed to write config: {}", e))?;

        // Store API key in keyring (skip if using existing key)
        if !self.api_key_input.is_empty() && !self.has_existing_key() && !self.is_custom_provider() {
            let provider = self.current_provider();
            if !provider.keyring_key.is_empty() {
                let secret = SecretString::from_str(&self.api_key_input);
                secret
                    .save_to_keyring(provider.keyring_key)
                    .map_err(|e| format!("Failed to save API key to keyring: {}", e))?;
            }
        }

        // Seed workspace templates (use AI-generated content when available)
        if self.seed_templates {
            let workspace = PathBuf::from(&self.workspace_path);
            std::fs::create_dir_all(&workspace)
                .map_err(|e| format!("Failed to create workspace: {}", e))?;

            for (filename, content) in TEMPLATE_FILES {
                let file_path = workspace.join(filename);
                if !file_path.exists() {
                    // Use AI-generated content when available, static template as fallback
                    let final_content = match *filename {
                        "SOUL.md" => self.generated_soul.as_deref().unwrap_or(content),
                        "IDENTITY.md" => self.generated_identity.as_deref().unwrap_or(content),
                        "USER.md" => self.generated_user.as_deref().unwrap_or(content),
                        "AGENTS.md" => self.generated_agents.as_deref().unwrap_or(content),
                        "TOOLS.md" => self.generated_tools.as_deref().unwrap_or(content),
                        "MEMORY.md" => self.generated_memory.as_deref().unwrap_or(content),
                        _ => content,
                    };
                    std::fs::write(&file_path, final_content)
                        .map_err(|e| format!("Failed to write {}: {}", filename, e))?;
                }
            }
        }

        // Install daemon if requested
        if self.install_daemon
            && let Err(e) = install_daemon_service() {
                tracing::warn!("Failed to install daemon: {}", e);
                // Non-fatal — don't block onboarding completion
            }

        Ok(())
    }
}

/// What the app should do after handling a wizard key event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardAction {
    /// Nothing special
    None,
    /// User cancelled the wizard (Esc from step 1)
    Cancel,
    /// Wizard completed successfully
    Complete,
    /// Trigger async AI generation of brain files
    GenerateBrain,
}

/// First-time detection: no config file AND no API keys in environment.
/// Once config.toml is written (by onboarding or manually), this returns false forever.
/// If any API key env var is set, the user has already configured auth — skip onboarding.
/// To re-run the wizard, use `opencrabs onboard`, `--onboard` flag, or `/onboard`.
pub fn is_first_time() -> bool {
    let has_config = dirs::config_dir()
        .map(|d| d.join("opencrabs").join("config.toml").exists())
        .unwrap_or(false);

    if has_config {
        return false;
    }

    let has_env_key = std::env::var("ANTHROPIC_MAX_SETUP_TOKEN").is_ok()
        || std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("GEMINI_API_KEY").is_ok()
        || std::env::var("DASHSCOPE_API_KEY").is_ok();

    !has_env_key
}

/// Install the appropriate daemon service for the current platform
fn install_daemon_service() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        install_systemd_service()
    }

    #[cfg(target_os = "macos")]
    {
        install_launchagent()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("Daemon installation not supported on this platform".to_string())
    }
}

#[cfg(target_os = "linux")]
fn install_systemd_service() -> Result<(), String> {
    let service_dir = dirs::config_dir()
        .ok_or("Cannot determine config dir")?
        .parent()
        .ok_or("Cannot determine parent of config dir")?
        .join(".config")
        .join("systemd")
        .join("user");

    // Try the standard XDG path first
    let service_dir = if service_dir.exists() {
        service_dir
    } else {
        dirs::home_dir()
            .ok_or("Cannot determine home dir")?
            .join(".config")
            .join("systemd")
            .join("user")
    };

    std::fs::create_dir_all(&service_dir)
        .map_err(|e| format!("Failed to create systemd dir: {}", e))?;

    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get exe path: {}", e))?;

    let service_content = format!(
        r#"[Unit]
Description=OpenCrabs AI Orchestration Agent
After=network.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#,
        exe_path.display()
    );

    let service_path = service_dir.join("opencrabs.service");
    std::fs::write(&service_path, service_content)
        .map_err(|e| format!("Failed to write service file: {}", e))?;

    // Enable the service
    std::process::Command::new("systemctl")
        .args(["--user", "enable", "opencrabs"])
        .output()
        .map_err(|e| format!("Failed to enable service: {}", e))?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn install_launchagent() -> Result<(), String> {
    let agents_dir = dirs::home_dir()
        .ok_or("Cannot determine home dir")?
        .join("Library")
        .join("LaunchAgents");

    std::fs::create_dir_all(&agents_dir)
        .map_err(|e| format!("Failed to create LaunchAgents dir: {}", e))?;

    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get exe path: {}", e))?;

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.opencrabs.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"#,
        exe_path.display()
    );

    let plist_path = agents_dir.join("com.opencrabs.agent.plist");
    std::fs::write(&plist_path, plist_content)
        .map_err(|e| format!("Failed to write plist: {}", e))?;

    std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("Failed to load launch agent: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_creation() {
        let wizard = OnboardingWizard::new();
        assert_eq!(wizard.step, OnboardingStep::ModeSelect);
        assert_eq!(wizard.mode, WizardMode::QuickStart);
        assert_eq!(wizard.channel_toggles.len(), CHANNEL_NAMES.len());
    }

    #[test]
    fn test_step_navigation() {
        let mut wizard = OnboardingWizard::new();
        wizard.api_key_input = "test-key".to_string();

        assert_eq!(wizard.step, OnboardingStep::ModeSelect);
        wizard.next_step();
        assert_eq!(wizard.step, OnboardingStep::ProviderAuth);
        wizard.next_step();
        assert_eq!(wizard.step, OnboardingStep::Workspace);

        // QuickStart skips to health check
        wizard.next_step();
        assert_eq!(wizard.step, OnboardingStep::HealthCheck);
    }

    #[test]
    fn test_advanced_mode_all_steps() {
        let mut wizard = OnboardingWizard::new();
        wizard.mode = WizardMode::Advanced;
        wizard.api_key_input = "test-key".to_string();

        wizard.next_step(); // -> ProviderAuth
        wizard.next_step(); // -> Workspace
        wizard.next_step(); // -> Gateway (not skipped in Advanced)
        assert_eq!(wizard.step, OnboardingStep::Gateway);
        wizard.next_step(); // -> Channels
        assert_eq!(wizard.step, OnboardingStep::Channels);
        wizard.next_step(); // -> Daemon
        assert_eq!(wizard.step, OnboardingStep::Daemon);
        wizard.next_step(); // -> HealthCheck
        assert_eq!(wizard.step, OnboardingStep::HealthCheck);
    }

    #[test]
    fn test_step_numbers() {
        assert_eq!(OnboardingStep::ModeSelect.number(), 1);
        assert_eq!(OnboardingStep::HealthCheck.number(), 7);
        assert_eq!(OnboardingStep::BrainSetup.number(), 8);
        assert_eq!(OnboardingStep::total(), 8);
    }

    #[test]
    fn test_prev_step_cancel() {
        let mut wizard = OnboardingWizard::new();
        // Going back from step 1 signals cancel
        assert!(wizard.prev_step());
    }

    #[test]
    fn test_provider_auth_defaults() {
        let wizard = OnboardingWizard::new();
        assert_eq!(wizard.selected_provider, 0);
        assert_eq!(wizard.auth_field, AuthField::Provider);
        assert!(wizard.api_key_input.is_empty());
        assert_eq!(wizard.selected_model, 0);
        // First provider is Anthropic Claude (recommended)
        assert_eq!(PROVIDERS[wizard.selected_provider].name, "Anthropic Claude (recommended)");
        assert!(!PROVIDERS[wizard.selected_provider].models.is_empty());
        assert!(!PROVIDERS[wizard.selected_provider].help_lines.is_empty());
    }

    #[test]
    fn test_channel_toggles_default_off() {
        let wizard = OnboardingWizard::new();
        assert_eq!(wizard.channel_toggles.len(), CHANNEL_NAMES.len());
        // All channels default to disabled
        for (name, enabled) in &wizard.channel_toggles {
            assert!(!enabled, "Channel {} should default to disabled", name);
        }
        // Verify all expected channels are present
        let toggle_names: Vec<&str> = wizard.channel_toggles.iter().map(|(n, _)| n.as_str()).collect();
        assert!(toggle_names.contains(&"Telegram"));
        assert!(toggle_names.contains(&"Discord"));
        assert!(toggle_names.contains(&"iMessage"));
    }
}
