//! `/onboard:<sub>`, `/doctor` and `/models` resolution (#1665).
//!
//! `/doctor` used to rewrite itself to the suffix `health` and ride the same
//! dispatch arm a typed `/onboard:health` rode, keeping alive a spelling that
//! was retired when `/doctor` became the standalone health checker. Two
//! entry points to one step, one of them undocumented and unautocompleted,
//! is the one that rots.

use crate::tui::onboarding::OnboardingStep;
use crate::tui::onboarding::deep_link::{DeepLink, ONBOARD_SUBCOMMANDS, resolve};

#[test]
fn doctor_opens_the_health_check() {
    let (link, arg) = resolve("/doctor", "/doctor");
    assert_eq!(link, DeepLink::Step(OnboardingStep::HealthCheck));
    assert_eq!(arg, "");
}

#[test]
fn onboard_health_no_longer_resolves_to_a_step() {
    let (link, _) = resolve("/onboard:health", "/onboard:health");
    assert_eq!(
        link,
        DeepLink::Unknown("health".to_string()),
        "/doctor is the health checker; the /onboard:health spelling was retired with it"
    );
}

#[test]
fn health_is_absent_from_the_subcommand_table() {
    assert!(
        !ONBOARD_SUBCOMMANDS
            .iter()
            .any(|(name, _)| *name == "health"),
        "a health subcommand would reintroduce the second path into HealthCheck"
    );
}

#[test]
fn bare_onboard_runs_the_full_wizard() {
    let (link, arg) = resolve("/onboard", "/onboard");
    assert_eq!(link, DeepLink::FullWizard);
    assert_eq!(arg, "");
}

#[test]
fn models_is_the_provider_step() {
    let (link, _) = resolve("/models", "/models");
    assert_eq!(link, DeepLink::Step(OnboardingStep::ProviderAuth));
}

#[test]
fn every_subcommand_in_the_table_resolves_to_its_step() {
    for (name, step) in ONBOARD_SUBCOMMANDS {
        let input = format!("/onboard:{name}");
        let (link, _) = resolve(&input, &input);
        assert_eq!(
            link,
            DeepLink::Step(*step),
            "/onboard:{name} must open {step:?}"
        );
    }
}

#[test]
fn channel_argument_survives_resolution() {
    // #271: the argument is read off the full input, never off the first word.
    let (link, arg) = resolve("/onboard:channels", "/onboard:channels whatsapp");
    assert_eq!(link, DeepLink::Step(OnboardingStep::Channels));
    assert_eq!(arg, "whatsapp");
}

#[test]
fn unknown_suffix_is_reported_as_unknown_not_silently_accepted() {
    let (link, _) = resolve("/onboard:gateway", "/onboard:gateway");
    assert_eq!(link, DeepLink::Unknown("gateway".to_string()));
}
