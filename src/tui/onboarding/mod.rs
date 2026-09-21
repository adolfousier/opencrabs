//! Onboarding Wizard
//!
//! A 7-step TUI-based onboarding wizard for first-time OpenCrabs users.
//! Handles mode selection, provider/auth setup, workspace, gateway,
//! channels, daemon installation, and health check.

mod brain;
mod channels;
mod config;
pub mod deep_link;
mod fetch;
pub(crate) mod helpers;
mod input;
pub(crate) mod key_field;
mod keys;
mod models;
mod navigation;
pub mod state;
mod types;
pub mod voice;
pub(crate) mod voice_chain;
mod wizard;

// Re-export all public types
pub use types::{
    AuthField, BrainField, CHANNEL_NAMES, ChannelTestStatus, CodexDeviceFlowStatus, DiscordField,
    EXISTING_KEY_SENTINEL, GitHubDeviceFlowStatus, HealthStatus, ImageField, OnboardingStep,
    PROVIDERS, ProviderInfo, SlackField, SttProvider, TEMPLATE_FILES, TelegramField, TrelloField,
    TtsProvider, VoiceField, WhatsAppField, WizardAction, WizardMode,
};

pub use wizard::OnboardingWizard;

pub(crate) use brain::parse_brain_sections;
pub use fetch::{fetch_provider_models, is_first_time};
// merge_minimax_baseline is used by tests via `crate::tui::onboarding::fetch::...`;
// the module is private but the function is `pub(crate)`, accessible only when
// the `fetch` module is reachable. Test access goes through a cfg(test) bridge.
#[cfg(test)]
pub(crate) use fetch::{merge_minimax_baseline, xiaomi_baseline_models};

/// System prompt sent once after the user completes first-time onboarding.
/// Starts with `[SYSTEM:` so it's hidden from the user display but processed
/// by the agent. The agent uses its personality (SOUL.md) and user knowledge
/// (USER.md) to craft a personalized surprise instead of a hardcoded greeting.
pub const WELCOME_MESSAGE: &str = "[SYSTEM: Onboarding complete. Check your environment — use load_config to see what's configured and tool_search to discover what you have. You know who your user is (USER.md) and you have your personality (SOUL.md). Now do something that'll make them go 'holy shit'. Not a boring greeting. Not 'hey how can I help'. Something only YOU would say, like your best friend just walked in. Be creative, be unexpected. Then ask if they want daily check-ins, task reminders, or heartbeat — you can set those up with cron_manage right now.]";
