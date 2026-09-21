//! Resolve an onboarding deep link (`/onboard`, `/onboard:<sub>`, `/doctor`,
//! `/models`) to the wizard step it opens.
//!
//! Pulled out of `handle_slash_command` so the subcommand set has one home.
//! Dispatch reads [`ONBOARD_SUBCOMMANDS`], and so do the witness tests that
//! hold the slash registry and the README table to the same list. A doc row or
//! a registry entry that outlives its arm (#1664) then fails the build instead
//! of the user.

use super::OnboardingStep;

/// Every `/onboard:<name>` subcommand dispatch accepts, with the step it opens.
///
/// `health` is deliberately absent: `/doctor` is the health checker and the
/// `/onboard:health` spelling was retired with it (#1665).
pub const ONBOARD_SUBCOMMANDS: &[(&str, OnboardingStep)] = &[
    ("provider", OnboardingStep::ProviderAuth),
    ("workspace", OnboardingStep::Workspace),
    ("channels", OnboardingStep::Channels),
    ("voice", OnboardingStep::VoiceSetup),
    ("image", OnboardingStep::ImageSetup),
    ("daemon", OnboardingStep::Daemon),
    ("brain", OnboardingStep::BrainSetup),
];

/// What a slash input resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    /// Bare `/onboard`: the full wizard, starting at the mode selector.
    FullWizard,
    /// A recognised deep link, locked to a single step.
    Step(OnboardingStep),
    /// `/onboard:<something>` with no arm behind it. Carries the suffix so the
    /// caller can name it back to the user instead of silently guessing.
    Unknown(String),
}

/// Resolve `command` (the first word) plus the full `input` line.
///
/// Returns the link and the trailing argument, if any. The argument is what
/// makes `/onboard:channels whatsapp` land on the WhatsApp dialog rather than
/// the channel menu (#271), so it is read off the full input, never off the
/// first word.
pub fn resolve<'a>(command: &str, input: &'a str) -> (DeepLink, &'a str) {
    // `/doctor` is its own command, not a suffix spelling. Routing it straight
    // to the step keeps `/onboard:health` from riding in on the same arm.
    if command == "/doctor" {
        return (DeepLink::Step(OnboardingStep::HealthCheck), "");
    }
    // `/models` IS `/onboard:provider` without progress dots: the exact same
    // shared provider/model picker. One implementation, so a change to the
    // picker affects both; no separate ModelSelector dialog.
    let suffix: &'a str = if command == "/models" {
        "provider"
    } else {
        input
            .strip_prefix("/onboard")
            .unwrap_or("")
            .trim_start_matches(':')
    };

    let mut parts = suffix.split_whitespace();
    let head = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("");
    if head.is_empty() {
        return (DeepLink::FullWizard, "");
    }
    match ONBOARD_SUBCOMMANDS.iter().find(|(name, _)| *name == head) {
        Some((_, step)) => (DeepLink::Step(*step), arg),
        None => (DeepLink::Unknown(head.to_string()), ""),
    }
}

/// The message shown for an unrecognised `/onboard:` suffix.
///
/// Naming the valid set beats opening the full wizard, which made a typo or
/// a stale documented name indistinguishable from bare `/onboard` (#1664).
pub fn unknown_suffix_message(suffix: &str) -> String {
    let valid = ONBOARD_SUBCOMMANDS
        .iter()
        .map(|(name, _)| format!("`/onboard:{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "⚠️ Unknown onboarding subcommand `{suffix}`. Valid: {valid}. \
         Bare `/onboard` runs the full wizard, `/doctor` runs the health check."
    )
}
