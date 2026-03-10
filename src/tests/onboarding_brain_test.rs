//! Tests for onboarding brain setup — prompt building and response parsing.
//!
//! Covers `truncate_preview`, `build_brain_prompt`, `apply_generated_brain`,
//! and brain key handling via the public `handle_key` API.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::onboarding::{BrainField, OnboardingStep, OnboardingWizard, WizardAction};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn make_wizard_at_brain() -> OnboardingWizard {
    let mut w = OnboardingWizard::new();
    w.step = OnboardingStep::BrainSetup;
    w.brain_field = BrainField::AboutMe;
    w
}

// ── brain key handling via handle_key ───────────────────────────

#[test]
fn brain_ignores_input_while_generating() {
    let mut w = make_wizard_at_brain();
    w.brain_generating = true;
    let before = w.about_me.clone();
    let action = w.handle_key(key(KeyCode::Char('a')));
    assert_eq!(action, WizardAction::None);
    assert_eq!(w.about_me, before); // unchanged
}

#[test]
fn brain_enter_advances_when_generated() {
    let mut w = make_wizard_at_brain();
    w.brain_generated = true;
    let action = w.handle_key(key(KeyCode::Enter));
    assert_eq!(action, WizardAction::Complete);
}

#[test]
fn brain_enter_advances_when_error() {
    let mut w = make_wizard_at_brain();
    w.brain_error = Some("parse error".to_string());
    let action = w.handle_key(key(KeyCode::Enter));
    assert_eq!(action, WizardAction::Complete);
}

#[test]
fn brain_esc_goes_to_health_check() {
    let mut w = make_wizard_at_brain();
    // Esc triggers prev_step() which goes BrainSetup → HealthCheck
    let action = w.handle_key(key(KeyCode::Esc));
    assert_eq!(w.step, OnboardingStep::HealthCheck);
    assert_eq!(action, WizardAction::None);
}

#[test]
fn brain_tab_toggles_fields() {
    let mut w = make_wizard_at_brain();
    assert_eq!(w.brain_field, BrainField::AboutMe);
    w.handle_key(key(KeyCode::Tab));
    assert_eq!(w.brain_field, BrainField::AboutAgent);
    w.handle_key(key(KeyCode::Tab));
    assert_eq!(w.brain_field, BrainField::AboutMe);
}

#[test]
fn brain_backtab_toggles_fields() {
    let mut w = make_wizard_at_brain();
    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.brain_field, BrainField::AboutAgent);
    w.handle_key(key(KeyCode::BackTab));
    assert_eq!(w.brain_field, BrainField::AboutMe);
}

#[test]
fn brain_char_appends_to_about_me() {
    let mut w = make_wizard_at_brain();
    w.about_me.clear(); // clear any pre-loaded content
    w.handle_key(key(KeyCode::Char('H')));
    w.handle_key(key(KeyCode::Char('i')));
    assert_eq!(w.about_me, "Hi");
}

#[test]
fn brain_char_appends_to_about_agent_when_focused() {
    let mut w = make_wizard_at_brain();
    w.brain_field = BrainField::AboutAgent;
    w.about_opencrabs.clear(); // clear any pre-loaded content
    w.handle_key(key(KeyCode::Char('A')));
    w.handle_key(key(KeyCode::Char('I')));
    assert_eq!(w.about_opencrabs, "AI");
}

#[test]
fn brain_backspace_removes_char() {
    let mut w = make_wizard_at_brain();
    w.about_me = "Hello".to_string();
    w.handle_key(key(KeyCode::Backspace));
    assert_eq!(w.about_me, "Hell");
}

#[test]
fn brain_enter_on_about_me_moves_to_about_agent() {
    let mut w = make_wizard_at_brain();
    w.about_me = "test".to_string();
    w.handle_key(key(KeyCode::Enter));
    assert_eq!(w.brain_field, BrainField::AboutAgent);
}

#[test]
fn brain_enter_on_about_agent_with_empty_both_completes() {
    let mut w = make_wizard_at_brain();
    w.brain_field = BrainField::AboutAgent;
    let action = w.handle_key(key(KeyCode::Enter));
    assert_eq!(action, WizardAction::Complete);
    assert_eq!(w.step, OnboardingStep::Complete);
}

#[test]
fn brain_enter_on_about_agent_with_input_triggers_generate() {
    let mut w = make_wizard_at_brain();
    w.brain_field = BrainField::AboutAgent;
    w.about_me = "John".to_string();
    let action = w.handle_key(key(KeyCode::Enter));
    assert_eq!(action, WizardAction::GenerateBrain);
}

#[test]
fn brain_enter_skips_when_inputs_unchanged_from_loaded() {
    let mut w = make_wizard_at_brain();
    w.brain_field = BrainField::AboutAgent;
    w.about_me = "existing".to_string();
    w.original_about_me = "existing".to_string();
    w.about_opencrabs = "same".to_string();
    w.original_about_opencrabs = "same".to_string();
    let action = w.handle_key(key(KeyCode::Enter));
    assert_eq!(action, WizardAction::Complete);
}

// truncate_preview is pub(super) — tested in onboarding::tests

// ── build_brain_prompt ──────────────────────────────────────────

#[test]
fn brain_prompt_contains_user_input() {
    let mut w = OnboardingWizard::new();
    w.about_me = "I am a developer".to_string();
    w.about_opencrabs = "Be helpful".to_string();
    let prompt = w.build_brain_prompt();
    assert!(prompt.contains("I am a developer"));
    assert!(prompt.contains("Be helpful"));
}

#[test]
fn brain_prompt_uses_not_provided_when_empty() {
    let mut w = OnboardingWizard::new();
    w.about_me.clear();
    w.about_opencrabs.clear();
    let prompt = w.build_brain_prompt();
    assert!(prompt.contains("Not provided"));
}

#[test]
fn brain_prompt_includes_template_markers() {
    let w = OnboardingWizard::new();
    let prompt = w.build_brain_prompt();
    assert!(prompt.contains("===TEMPLATE: SOUL.md==="));
    assert!(prompt.contains("===TEMPLATE: IDENTITY.md==="));
    assert!(prompt.contains("===TEMPLATE: USER.md==="));
    assert!(prompt.contains("===TEMPLATE: AGENTS.md==="));
    assert!(prompt.contains("===TEMPLATE: TOOLS.md==="));
    assert!(prompt.contains("===TEMPLATE: MEMORY.md==="));
}

#[test]
fn brain_prompt_includes_response_delimiters() {
    let w = OnboardingWizard::new();
    let prompt = w.build_brain_prompt();
    assert!(prompt.contains("---SOUL---"));
    assert!(prompt.contains("---MEMORY---"));
}

// ── apply_generated_brain ───────────────────────────────────────

#[test]
fn apply_valid_response_sets_all_fields() {
    let mut w = OnboardingWizard::new();
    let response = "---SOUL---\nSoul content\n---IDENTITY---\nIdentity content\n---USER---\nUser content\n---AGENTS---\nAgents content\n---TOOLS---\nTools content\n---MEMORY---\nMemory content";
    w.apply_generated_brain(response);
    assert!(w.brain_generated);
    assert!(!w.brain_generating);
    assert!(w.brain_error.is_none());
    assert_eq!(w.generated_soul.as_deref(), Some("Soul content"));
    assert_eq!(w.generated_identity.as_deref(), Some("Identity content"));
    assert_eq!(w.generated_user.as_deref(), Some("User content"));
}

#[test]
fn apply_partial_response_missing_required_sets_error() {
    let mut w = OnboardingWizard::new();
    let response = "---AGENTS---\nAgents content";
    w.apply_generated_brain(response);
    assert!(w.brain_error.is_some());
    assert!(!w.brain_generated);
}

#[test]
fn apply_response_with_only_three_required() {
    let mut w = OnboardingWizard::new();
    let response = "---SOUL---\nS\n---IDENTITY---\nI\n---USER---\nU";
    w.apply_generated_brain(response);
    assert!(w.brain_generated);
    assert!(w.brain_error.is_none());
}

#[test]
fn apply_empty_response_sets_error() {
    let mut w = OnboardingWizard::new();
    w.apply_generated_brain("");
    assert!(w.brain_error.is_some());
}
