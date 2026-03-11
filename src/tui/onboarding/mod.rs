//! Onboarding Wizard
//!
//! A 7-step TUI-based onboarding wizard for first-time OpenCrabs users.
//! Handles mode selection, provider/auth setup, workspace, gateway,
//! channels, daemon installation, and health check.

mod brain;
mod channels;
mod config;
mod fetch;
mod helpers;
mod input;
mod keys;
mod models;
mod navigation;
mod types;
pub mod voice;
mod wizard;

#[cfg(test)]
mod tests;

// Re-export all public types
pub use types::{
    AuthField, BrainField, CHANNEL_NAMES, ChannelTestStatus, DiscordField, EXISTING_KEY_SENTINEL,
    GitHubDeviceFlowStatus, HealthStatus, ImageField, OnboardingStep, PROVIDERS, ProviderInfo,
    SlackField, TelegramField, TrelloField, VoiceField, WhatsAppField, WizardAction, WizardMode,
};

pub use wizard::OnboardingWizard;

pub use fetch::{fetch_provider_models, is_first_time};
