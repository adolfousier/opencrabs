//! Tests for onboarding types — step numbering, titles, subtitles, provider info, channels.

use crate::tui::onboarding::{
    BrainField, CHANNEL_NAMES, HealthStatus, ImageField, OnboardingStep, PROVIDERS, VoiceField,
    WizardAction,
};

// ── OnboardingStep ──────────────────────────────────────────────

#[test]
fn step_numbers_are_sequential() {
    let steps = [
        OnboardingStep::ModeSelect,
        OnboardingStep::Workspace,
        OnboardingStep::ProviderAuth,
        OnboardingStep::Channels,
        OnboardingStep::VoiceSetup,
        OnboardingStep::ImageSetup,
        OnboardingStep::Daemon,
        OnboardingStep::HealthCheck,
        OnboardingStep::BrainSetup,
        OnboardingStep::Complete,
    ];
    for (i, step) in steps.iter().enumerate() {
        assert_eq!(step.number(), i + 1, "step {:?} has wrong number", step);
    }
}

#[test]
fn channel_sub_steps_share_number_4() {
    assert_eq!(OnboardingStep::TelegramSetup.number(), 4);
    assert_eq!(OnboardingStep::DiscordSetup.number(), 4);
    assert_eq!(OnboardingStep::WhatsAppSetup.number(), 4);
    assert_eq!(OnboardingStep::SlackSetup.number(), 4);
    assert_eq!(OnboardingStep::TrelloSetup.number(), 4);
}

#[test]
fn total_steps_is_9() {
    assert_eq!(OnboardingStep::total(), 9);
}

#[test]
fn all_steps_have_titles() {
    let steps = [
        OnboardingStep::ModeSelect,
        OnboardingStep::Workspace,
        OnboardingStep::ProviderAuth,
        OnboardingStep::Channels,
        OnboardingStep::TelegramSetup,
        OnboardingStep::DiscordSetup,
        OnboardingStep::WhatsAppSetup,
        OnboardingStep::SlackSetup,
        OnboardingStep::TrelloSetup,
        OnboardingStep::VoiceSetup,
        OnboardingStep::ImageSetup,
        OnboardingStep::Daemon,
        OnboardingStep::HealthCheck,
        OnboardingStep::BrainSetup,
        OnboardingStep::Complete,
    ];
    for step in &steps {
        assert!(!step.title().is_empty(), "{:?} has empty title", step);
        assert!(!step.subtitle().is_empty(), "{:?} has empty subtitle", step);
    }
}

// ── PROVIDERS ───────────────────────────────────────────────────

#[test]
fn provider_count_matches_expected() {
    assert_eq!(PROVIDERS.len(), 7);
}

#[test]
fn anthropic_is_first_provider() {
    assert_eq!(PROVIDERS[0].name, "Anthropic Claude");
}

#[test]
fn custom_provider_is_last() {
    assert_eq!(PROVIDERS[6].name, "Custom OpenAI-Compatible");
}

#[test]
fn all_providers_have_key_label_and_help() {
    for (i, p) in PROVIDERS.iter().enumerate() {
        assert!(!p.key_label.is_empty(), "provider {} missing key_label", i);
        assert!(
            !p.help_lines.is_empty(),
            "provider {} missing help_lines",
            i
        );
    }
}

// ── CHANNEL_NAMES ───────────────────────────────────────────────

#[test]
fn channel_count() {
    assert_eq!(CHANNEL_NAMES.len(), 5);
}

#[test]
fn first_three_channels() {
    assert_eq!(CHANNEL_NAMES[0].0, "Telegram");
    assert_eq!(CHANNEL_NAMES[1].0, "Discord");
    assert_eq!(CHANNEL_NAMES[2].0, "WhatsApp");
}

#[test]
fn all_channels_have_descriptions() {
    for (name, desc) in CHANNEL_NAMES {
        assert!(!desc.is_empty(), "Channel {} must have a description", name);
    }
}

// TEMPLATE_FILES and EXISTING_KEY_SENTINEL are pub(super) — tested in onboarding::tests

// ── WizardAction enum ───────────────────────────────────────────

#[test]
fn wizard_action_eq() {
    assert_eq!(WizardAction::None, WizardAction::None);
    assert_ne!(WizardAction::None, WizardAction::Complete);
    assert_eq!(WizardAction::GenerateBrain, WizardAction::GenerateBrain);
}

// ── HealthStatus enum ───────────────────────────────────────────

#[test]
fn health_status_eq() {
    assert_eq!(HealthStatus::Pending, HealthStatus::Pending);
    assert_eq!(HealthStatus::Pass, HealthStatus::Pass);
    assert_ne!(HealthStatus::Pending, HealthStatus::Pass);
    assert_eq!(
        HealthStatus::Fail("err".to_string()),
        HealthStatus::Fail("err".to_string())
    );
}

// ── Enum field variants ─────────────────────────────────────────

#[test]
fn voice_field_variants() {
    let fields = [
        VoiceField::SttModeSelect,
        VoiceField::GroqApiKey,
        VoiceField::LocalModelSelect,
        VoiceField::TtsModeSelect,
        VoiceField::TtsLocalVoiceSelect,
    ];
    // Just ensure they're all distinct
    for (i, a) in fields.iter().enumerate() {
        for (j, b) in fields.iter().enumerate() {
            if i != j {
                assert_ne!(a, b);
            }
        }
    }
}

#[test]
fn image_field_variants() {
    assert_ne!(ImageField::VisionToggle, ImageField::GenerationToggle);
    assert_ne!(ImageField::GenerationToggle, ImageField::ApiKey);
}

#[test]
fn brain_field_variants() {
    assert_ne!(BrainField::AboutMe, BrainField::AboutAgent);
}
