/// Sentinel value stored in api_key_input when a key was loaded from config.
/// The actual key is never held in memory — this just signals "key exists".
pub(super) const EXISTING_KEY_SENTINEL: &str = "__EXISTING_KEY__";

/// Provider definitions
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        name: "Anthropic Claude",
        models: &[], // Fetched from API
        key_label: "Setup Token",
        help_lines: &[
            "Claude Max / Code: run 'claude setup-token'",
            "Or paste API key from console.anthropic.com",
        ],
    },
    ProviderInfo {
        name: "OpenAI",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from platform.openai.com"],
    },
    ProviderInfo {
        name: "Google Gemini",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from aistudio.google.com"],
    },
    ProviderInfo {
        name: "OpenRouter",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from openrouter.ai/keys"],
    },
    ProviderInfo {
        name: "Minimax",
        models: &[], // Loaded from config.toml at runtime
        key_label: "API Key",
        help_lines: &["Get key from platform.minimax.io"],
    },
    ProviderInfo {
        name: "Custom OpenAI-Compatible",
        models: &[],
        key_label: "API Key",
        help_lines: &["Enter your own API endpoint"],
    },
];

pub struct ProviderInfo {
    pub name: &'static str,
    pub models: &'static [&'static str],
    pub key_label: &'static str,
    pub help_lines: &'static [&'static str],
}

/// Channel definitions for the unified Channels step.
/// Index mapping: 0=Telegram, 1=Discord, 2=WhatsApp, 3=Slack, 4=Trello, 5=Signal, 6=Google Chat, 7=iMessage
pub const CHANNEL_NAMES: &[(&str, &str)] = &[
    ("Telegram", "Bot token (via @BotFather)"),
    ("Discord", "Bot token (via Developer Portal)"),
    ("WhatsApp", "QR code pairing"),
    ("Slack", "Socket Mode (bot + app tokens)"),
    ("Trello", "API Key + Token from trello.com/power-ups/admin"),
    ("Signal", "Coming soon"),
    ("Google Chat", "Coming soon"),
    ("iMessage", "Coming soon"),
];

/// Template files to seed in the workspace
pub(super) const TEMPLATE_FILES: &[(&str, &str)] = &[
    (
        "SOUL.md",
        include_str!("../../docs/reference/templates/SOUL.md"),
    ),
    (
        "IDENTITY.md",
        include_str!("../../docs/reference/templates/IDENTITY.md"),
    ),
    (
        "USER.md",
        include_str!("../../docs/reference/templates/USER.md"),
    ),
    (
        "AGENTS.md",
        include_str!("../../docs/reference/templates/AGENTS.md"),
    ),
    (
        "TOOLS.md",
        include_str!("../../docs/reference/templates/TOOLS.md"),
    ),
    (
        "MEMORY.md",
        include_str!("../../docs/reference/templates/MEMORY.md"),
    ),
];

/// Current step in the onboarding wizard
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    ModeSelect,
    Workspace,
    ProviderAuth,
    Channels,
    TelegramSetup,
    DiscordSetup,
    WhatsAppSetup,
    SlackSetup,
    TrelloSetup,
    Gateway,
    VoiceSetup,
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
            Self::Workspace => 2,
            Self::ProviderAuth => 3,
            Self::Channels => 4,
            Self::TelegramSetup => 4, // sub-step of Channels
            Self::DiscordSetup => 4,  // sub-step of Channels
            Self::WhatsAppSetup => 4, // sub-step of Channels
            Self::SlackSetup => 4,    // sub-step of Channels
            Self::TrelloSetup => 4,   // sub-step of Channels
            Self::Gateway => 5,
            Self::VoiceSetup => 6,
            Self::Daemon => 7,
            Self::HealthCheck => 8,
            Self::BrainSetup => 9,
            Self::Complete => 10,
        }
    }

    /// Total number of steps (excluding Complete)
    pub fn total() -> usize {
        9
    }

    /// Step title
    pub fn title(&self) -> &'static str {
        match self {
            Self::ModeSelect => "Pick Your Vibe",
            Self::Workspace => "Home Base",
            Self::ProviderAuth => "Brain Fuel",
            Self::Channels => "Chat Me Anywhere",
            Self::TelegramSetup => "Telegram Bot",
            Self::DiscordSetup => "Discord Bot",
            Self::WhatsAppSetup => "WhatsApp",
            Self::SlackSetup => "Slack Bot",
            Self::TrelloSetup => "Trello",
            Self::Gateway => "API Gateway",
            Self::VoiceSetup => "Voice Superpowers",
            Self::Daemon => "Always On",
            Self::HealthCheck => "Vibe Check",
            Self::BrainSetup => "Make It Yours",
            Self::Complete => "Let's Go!",
        }
    }

    /// Step subtitle
    pub fn subtitle(&self) -> &'static str {
        match self {
            Self::ModeSelect => "Quick and easy or full control — your call",
            Self::Workspace => "Where my brain lives on disk",
            Self::ProviderAuth => "Pick your AI model and drop your key",
            Self::Channels => "Chat with me from your phone — Telegram, WhatsApp, whatever",
            Self::TelegramSetup => "Hook up your Telegram bot token",
            Self::DiscordSetup => "Hook up your Discord bot token",
            Self::WhatsAppSetup => "Scan the QR code with your phone",
            Self::SlackSetup => "Hook up your Slack bot and app tokens",
            Self::TrelloSetup => "Hook up your Trello API Key and Token",
            Self::Gateway => "Open up an HTTP API if you want one",
            Self::VoiceSetup => "Talk to me, literally",
            Self::Daemon => "Keep me running in the background",
            Self::HealthCheck => "Making sure everything's wired up right",
            Self::BrainSetup => "Make me yours, drop some context so I actually get you",
            Self::Complete => "You're all set — let's build something cool",
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
    CustomName,
    CustomBaseUrl,
    CustomApiKey,
    CustomModel,
}

/// Which field is focused in DiscordSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordField {
    BotToken,
    ChannelID,
    AllowedList,
}

/// Which field is focused in SlackSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackField {
    BotToken,
    AppToken,
    ChannelID,
    AllowedList,
}

/// Which field is focused in TelegramSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramField {
    BotToken,
    UserID,
}

/// Which field is focused in WhatsAppSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhatsAppField {
    Connection,
    PhoneAllowlist,
}

/// Which field is focused in TrelloSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrelloField {
    ApiKey,
    ApiToken,
    BoardId,
    AllowedUsers,
}

/// Channel test connection status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelTestStatus {
    Idle,
    Testing,
    Success,
    Failed(String),
}

/// Which field is focused in VoiceSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceField {
    GroqApiKey,
    TtsToggle,
}

/// Which text area is focused in BrainSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrainField {
    AboutMe,
    AboutAgent,
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
    /// Trigger async model list fetch from provider API
    FetchModels,
    /// Trigger async WhatsApp QR code pairing
    WhatsAppConnect,
    /// Trigger async Telegram test message
    TestTelegram,
    /// Trigger async Discord test message
    TestDiscord,
    /// Trigger async Slack test message
    TestSlack,
    /// Trigger async WhatsApp test message
    TestWhatsApp,
    /// Trigger async Trello connection test
    TestTrello,
}
