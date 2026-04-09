/// Sentinel value stored in api_key_input when a key was loaded from config.
/// The actual key is never held in memory — this just signals "key exists".
pub use crate::tui::provider_selector::EXISTING_KEY_SENTINEL;

/// Provider definitions
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "anthropic",
        name: "Anthropic Claude",
        models: &[], // Fetched from API
        key_label: "Setup Token",
        help_lines: &[
            "Claude Max / Code: run 'claude setup-token'",
            "Or paste API key from console.anthropic.com",
        ],
    },
    ProviderInfo {
        id: "openai",
        name: "OpenAI",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from platform.openai.com"],
    },
    ProviderInfo {
        id: "github",
        name: "GitHub Copilot",
        models: &[],
        key_label: "OAuth",
        help_lines: &["Sign in with GitHub to use your Copilot subscription"],
    },
    ProviderInfo {
        id: "gemini",
        name: "Google Gemini",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from aistudio.google.com"],
    },
    ProviderInfo {
        id: "openrouter",
        name: "OpenRouter",
        models: &[],
        key_label: "API Key",
        help_lines: &["Get key from openrouter.ai/keys"],
    },
    ProviderInfo {
        id: "minimax",
        name: "Minimax",
        models: &[], // Loaded from config.toml at runtime
        key_label: "API Key",
        help_lines: &["Get key from platform.minimax.io"],
    },
    ProviderInfo {
        id: "zhipu",
        name: "z.ai GLM",
        models: &[], // Fetched from API
        key_label: "API Key",
        help_lines: &["Get key from open.bigmodel.cn"],
    },
    ProviderInfo {
        id: "claude-cli",
        name: "Claude CLI",
        models: &["sonnet", "opus", "haiku"],
        key_label: "",
        help_lines: &[
            "Uses local 'claude' CLI subprocess — no API key needed",
            "Requires: npm install -g @anthropic-ai/claude-code",
        ],
    },
    ProviderInfo {
        id: "opencode-cli",
        name: "OpenCode CLI",
        models: &[],
        key_label: "",
        help_lines: &[
            "Uses local 'opencode' CLI subprocess — free models, no API key needed",
            "Requires: curl -fsSL https://opencode.ai/install | bash",
        ],
    },
    ProviderInfo {
        id: "qwen-code-cli",
        name: "Qwen CLI",
        models: &["qwen3.6-plus", "qwen3.5-plus", "qwen3-coder-plus"],
        key_label: "",
        help_lines: &[
            "Uses local 'qwen' CLI subprocess — 1k free req/day via Qwen OAuth",
            "Requires: npm install -g @qwen-code/qwen-code  OR  brew install qwen-code",
        ],
    },
    ProviderInfo {
        id: "qwen",
        name: "Qwen",
        models: &["qwen3.6-plus"],
        key_label: "OAuth",
        help_lines: &[
            "Sign in with qwen.ai — OpenCrabs orchestrates tools and context natively",
            "Free tier: 60 req/min, 1000 req/day",
        ],
    },
    ProviderInfo {
        id: "", // dynamic — custom providers use runtime names
        name: "Custom OpenAI-Compatible",
        models: &[],
        key_label: "API Key",
        help_lines: &["Enter your own API endpoint"],
    },
];

pub struct ProviderInfo {
    /// Canonical provider id matching `KNOWN_PROVIDERS` (e.g. "anthropic", "claude-cli")
    /// Empty string for "Custom OpenAI-Compatible" (last entry) which is dynamic.
    pub id: &'static str,
    pub name: &'static str,
    pub models: &'static [&'static str],
    pub key_label: &'static str,
    pub help_lines: &'static [&'static str],
}

/// Channel definitions for the unified Channels step.
/// Index mapping: 0=Telegram, 1=Discord, 2=WhatsApp, 3=Slack, 4=Trello
pub const CHANNEL_NAMES: &[(&str, &str)] = &[
    ("Telegram", "Bot token (via @BotFather)"),
    ("Discord", "Bot token (via Developer Portal)"),
    ("WhatsApp", "QR code pairing"),
    ("Slack", "Socket Mode (bot + app tokens)"),
    ("Trello", "API Key + Token from trello.com/power-ups/admin"),
];

/// Template files to seed in the workspace
pub const TEMPLATE_FILES: &[(&str, &str)] = &[
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
    (
        "CODE.md",
        include_str!("../../docs/reference/templates/CODE.md"),
    ),
    (
        "SECURITY.md",
        include_str!("../../docs/reference/templates/SECURITY.md"),
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
    VoiceSetup,
    ImageSetup,
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
            Self::VoiceSetup => 5,
            Self::ImageSetup => 6,
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
            Self::VoiceSetup => "Voice Superpowers",
            Self::ImageSetup => "Image Handling",
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
            Self::VoiceSetup => "Talk to me, literally",
            Self::ImageSetup => "Vision and image generation via Google Gemini",
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
    CustomContextWindow,
    ZhipuEndpointType,
}

/// Which field is focused in DiscordSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordField {
    BotToken,
    ChannelID,
    AllowedList,
    RespondTo,
}

/// Which field is focused in SlackSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackField {
    BotToken,
    AppToken,
    ChannelID,
    AllowedList,
    RespondTo,
}

/// Which field is focused in TelegramSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramField {
    BotToken,
    UserID,
    RespondTo,
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
    SttModeSelect,
    GroqApiKey,
    LocalModelSelect,
    TtsModeSelect,
    TtsLocalVoiceSelect,
}

/// Which field is focused in ImageSetup step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageField {
    VisionToggle,
    GenerationToggle,
    ApiKey,
}

/// GitHub Copilot device flow status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubDeviceFlowStatus {
    /// Not started
    Idle,
    /// Waiting for user to enter code at github.com/login/device
    WaitingForUser,
    /// User authorized, token obtained
    Complete,
    /// Flow failed
    Failed(String),
}

/// Qwen native OAuth device flow status
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum QwenDeviceFlowStatus {
    /// Not started
    #[default]
    Idle,
    /// Device code obtained — show user_code + verification URL, polling token endpoint
    WaitingForUser { verification_uri: String },
    /// User authorized, credentials persisted
    Complete,
    /// Flow failed
    Failed(String),
    /// Rotation: authenticating account N of M
    RotationStep { current: usize, total: usize },
    /// Rotation: waiting for user to sign out before next account
    RotationSignout { current: usize, total: usize },
    /// Rotation: all accounts collected and persisted
    RotationComplete,
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
    /// Trigger async whisper model download
    DownloadWhisperModel,
    /// Trigger async Piper voice model download
    DownloadPiperVoice,
    /// Trigger GitHub Copilot OAuth device flow
    GitHubDeviceFlow,
    /// Trigger Qwen native OAuth device flow
    QwenDeviceFlow,
    /// Trigger Qwen rotation multi-account setup flow
    QwenRotationFlow,
    /// User confirmed signout — advance to next rotation account
    QwenRotationNext,
    /// Quick-jump step completed — save config and close wizard
    QuickJumpDone,
}
