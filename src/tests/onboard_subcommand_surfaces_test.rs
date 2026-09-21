//! The `/onboard:<sub>` set must read the same on every surface (#1664).
//!
//! `README.md` documented `/onboard:gateway` for a step that does not exist
//! and an arm that was never written. Typing it opened the full wizard at the
//! mode selector, silently, because the dispatch match ended in a catch-all
//! that swallowed every unrecognised suffix. A stale doc row was therefore
//! indistinguishable from bare `/onboard`, and nothing failed until a user
//! followed the README.
//!
//! Dispatch is the source of truth. These tests hold the README table and the
//! slash registry to it in both directions, so the next row that outlives its
//! arm, or arm that never reaches a surface, fails the build instead.

use crate::tui::app::state::SLASH_COMMANDS;
use crate::tui::onboarding::deep_link::{ONBOARD_SUBCOMMANDS, unknown_suffix_message};
use std::fs;
use std::path::Path;

fn readme() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("README.md must be readable")
}

/// Every `| `/onboard:x` |` row in the README, from any of its command tables.
fn readme_rows(readme: &str) -> Vec<String> {
    readme
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("| `/onboard:")?;
            let name = rest.split('`').next()?;
            Some(name.to_string())
        })
        .collect()
}

fn registry_names() -> Vec<String> {
    SLASH_COMMANDS
        .iter()
        .filter_map(|c| c.name.strip_prefix("/onboard:").map(str::to_string))
        .collect()
}

fn is_dispatched(name: &str) -> bool {
    ONBOARD_SUBCOMMANDS.iter().any(|(sub, _)| *sub == name)
}

#[test]
fn every_documented_subcommand_has_a_dispatch_arm() {
    let readme = readme();
    for name in readme_rows(&readme) {
        assert!(
            is_dispatched(&name),
            "README documents /onboard:{name} but dispatch has no arm for it, \
             so typing it opens the wizard at the mode selector instead"
        );
    }
}

#[test]
fn every_dispatch_arm_is_documented_in_the_readme() {
    let readme = readme();
    let rows = readme_rows(&readme);
    for (name, _) in ONBOARD_SUBCOMMANDS {
        assert!(
            rows.iter().any(|row| row == name),
            "/onboard:{name} dispatches to a step but no README row mentions it"
        );
    }
}

#[test]
fn every_registered_subcommand_has_a_dispatch_arm() {
    for name in registry_names() {
        assert!(
            is_dispatched(&name),
            "autocomplete offers /onboard:{name} but dispatch has no arm for it"
        );
    }
}

#[test]
fn every_dispatch_arm_is_offered_by_autocomplete() {
    let registered = registry_names();
    for (name, _) in ONBOARD_SUBCOMMANDS {
        assert!(
            registered.iter().any(|r| r == name),
            "/onboard:{name} dispatches to a step but is absent from SLASH_COMMANDS, \
             so neither autocomplete nor the help dialog can offer it"
        );
    }
}

#[test]
fn gateway_is_gone_from_every_surface() {
    assert!(
        !readme().contains("/onboard:gateway"),
        "there is no gateway wizard step and no gateway dispatch arm"
    );
    assert!(!is_dispatched("gateway"));
    assert!(!registry_names().iter().any(|n| n == "gateway"));
}

#[test]
fn unknown_suffix_message_names_the_offender_and_the_valid_set() {
    let msg = unknown_suffix_message("gateway");
    assert!(msg.contains("gateway"), "{msg}");
    for (name, _) in ONBOARD_SUBCOMMANDS {
        assert!(
            msg.contains(&format!("/onboard:{name}")),
            "valid set must list /onboard:{name}: {msg}"
        );
    }
}
