//! Tests for onboarding navigation — `next_step()` and `prev_step()` transitions.
//!
//! Covers the wizard step flow for both QuickStart and Advanced modes,
//! plus edge cases around validation and back-navigation.

use crate::tui::onboarding::{OnboardingStep, OnboardingWizard, WizardAction, WizardMode};

fn wizard() -> OnboardingWizard {
    OnboardingWizard::default()
}

// ── next_step: Advanced mode flow ───────────────────────────────

#[test]
fn mode_select_to_workspace() {
    let mut w = wizard();
    w.step = OnboardingStep::ModeSelect;
    w.next_step();
    assert_eq!(w.step, OnboardingStep::Workspace);
}

#[test]
fn provider_auth_to_channels_in_advanced() {
    let mut w = wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.mode = WizardMode::Advanced;
    w.api_key_input = "sk-test-key".to_string();
    w.next_step();
    assert_eq!(w.step, OnboardingStep::Channels);
}

#[test]
fn provider_auth_requires_api_key() {
    let mut w = wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.api_key_input.clear();
    w.next_step();
    // Should not advance — error set
    assert_eq!(w.step, OnboardingStep::ProviderAuth);
    assert!(w.error_message.is_some());
}

#[test]
fn channels_to_voice_setup() {
    let mut w = wizard();
    w.step = OnboardingStep::Channels;
    w.next_step();
    assert_eq!(w.step, OnboardingStep::VoiceSetup);
}

#[test]
fn voice_setup_to_image_setup() {
    let mut w = wizard();
    w.step = OnboardingStep::VoiceSetup;
    w.next_step();
    assert_eq!(w.step, OnboardingStep::ImageSetup);
}

#[test]
fn image_setup_to_daemon() {
    let mut w = wizard();
    w.step = OnboardingStep::ImageSetup;
    w.next_step();
    assert_eq!(w.step, OnboardingStep::Daemon);
}

#[test]
fn brain_setup_to_complete_when_generated() {
    let mut w = wizard();
    w.step = OnboardingStep::BrainSetup;
    w.brain_generated = true;
    w.next_step();
    assert_eq!(w.step, OnboardingStep::Complete);
}

#[test]
fn brain_setup_to_complete_when_error() {
    let mut w = wizard();
    w.step = OnboardingStep::BrainSetup;
    w.brain_error = Some("error".to_string());
    w.next_step();
    assert_eq!(w.step, OnboardingStep::Complete);
}

#[test]
fn brain_setup_stays_when_not_generated() {
    let mut w = wizard();
    w.step = OnboardingStep::BrainSetup;
    w.next_step();
    assert_eq!(w.step, OnboardingStep::BrainSetup);
}

#[test]
fn channel_setup_returns_to_channels() {
    for step in [
        OnboardingStep::TelegramSetup,
        OnboardingStep::DiscordSetup,
        OnboardingStep::WhatsAppSetup,
        OnboardingStep::SlackSetup,
        OnboardingStep::TrelloSetup,
    ] {
        let mut w = wizard();
        w.step = step;
        w.next_step();
        assert_eq!(w.step, OnboardingStep::Channels, "failed for {:?}", step);
    }
}

// ── next_step: QuickStart mode ──────────────────────────────────

#[test]
fn quickstart_provider_auth_skips_channels_to_daemon() {
    let mut w = wizard();
    w.mode = WizardMode::QuickStart;
    w.step = OnboardingStep::ProviderAuth;
    w.api_key_input = "key".to_string();
    w.next_step();
    assert_eq!(w.step, OnboardingStep::Daemon);
}

// ── next_step: clears error and focused_field ───────────────────

#[test]
fn next_step_clears_error_and_focus() {
    let mut w = wizard();
    w.step = OnboardingStep::ModeSelect;
    w.error_message = Some("old error".to_string());
    w.focused_field = 3;
    w.next_step();
    assert!(w.error_message.is_none());
    assert_eq!(w.focused_field, 0);
}

// ── next_step: quick_jump mode ──────────────────────────────────

#[test]
fn quick_jump_sets_done_flag() {
    let mut w = wizard();
    w.quick_jump = true;
    w.step = OnboardingStep::VoiceSetup;
    w.next_step();
    assert!(w.quick_jump_done);
    // Step should NOT change
    assert_eq!(w.step, OnboardingStep::VoiceSetup);
}

// ── prev_step ───────────────────────────────────────────────────

#[test]
fn prev_from_mode_select_signals_cancel() {
    let mut w = wizard();
    w.step = OnboardingStep::ModeSelect;
    let cancel = w.prev_step();
    assert!(cancel);
}

#[test]
fn prev_from_workspace_to_mode_select() {
    let mut w = wizard();
    w.step = OnboardingStep::Workspace;
    let cancel = w.prev_step();
    assert!(!cancel);
    assert_eq!(w.step, OnboardingStep::ModeSelect);
}

#[test]
fn prev_from_provider_auth_to_workspace() {
    let mut w = wizard();
    w.step = OnboardingStep::ProviderAuth;
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::Workspace);
}

#[test]
fn prev_from_channels_to_provider_auth() {
    let mut w = wizard();
    w.step = OnboardingStep::Channels;
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::ProviderAuth);
}

#[test]
fn prev_from_voice_to_channels() {
    let mut w = wizard();
    w.step = OnboardingStep::VoiceSetup;
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::Channels);
}

#[test]
fn prev_from_image_to_voice() {
    let mut w = wizard();
    w.step = OnboardingStep::ImageSetup;
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::VoiceSetup);
}

#[test]
fn prev_from_daemon_advanced_to_image() {
    let mut w = wizard();
    w.mode = WizardMode::Advanced;
    w.step = OnboardingStep::Daemon;
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::ImageSetup);
}

#[test]
fn prev_from_daemon_quickstart_to_provider_auth() {
    let mut w = wizard();
    w.mode = WizardMode::QuickStart;
    w.step = OnboardingStep::Daemon;
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::ProviderAuth);
}

#[test]
fn prev_from_health_check_to_daemon() {
    let mut w = wizard();
    w.step = OnboardingStep::HealthCheck;
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::Daemon);
}

#[test]
fn prev_from_brain_to_health_check() {
    let mut w = wizard();
    w.step = OnboardingStep::BrainSetup;
    w.brain_generating = true;
    w.brain_error = Some("err".to_string());
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::HealthCheck);
    assert!(!w.brain_generating);
    assert!(w.brain_error.is_none());
}

#[test]
fn prev_from_complete_to_brain() {
    let mut w = wizard();
    w.step = OnboardingStep::Complete;
    w.prev_step();
    assert_eq!(w.step, OnboardingStep::BrainSetup);
}

#[test]
fn prev_from_channel_setup_to_channels() {
    for step in [
        OnboardingStep::TelegramSetup,
        OnboardingStep::DiscordSetup,
        OnboardingStep::WhatsAppSetup,
        OnboardingStep::SlackSetup,
        OnboardingStep::TrelloSetup,
    ] {
        let mut w = wizard();
        w.step = step;
        w.prev_step();
        assert_eq!(w.step, OnboardingStep::Channels, "failed for {:?}", step);
    }
}

#[test]
fn prev_step_clears_error_and_focus() {
    let mut w = wizard();
    w.step = OnboardingStep::Channels;
    w.error_message = Some("old".to_string());
    w.focused_field = 5;
    w.prev_step();
    assert!(w.error_message.is_none());
    assert_eq!(w.focused_field, 0);
}
