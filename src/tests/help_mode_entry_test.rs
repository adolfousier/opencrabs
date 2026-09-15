//! Entering the help screen has to load its catalogue (#1586).
//!
//! #1530 moved the catalogue out of the renderer and into `switch_mode`, so
//! the screen is only populated on the paths that go through it. The `/help`
//! slash handler set `self.mode` directly and skipped that load, which left
//! every section empty and the screen reading "No command matches" with no
//! filter typed. These pin the wiring rather than the rendering: the bug was
//! which function ran, not what it drew.

use crate::tui::app::help_catalog::{HelpSection, section_matches};
use crate::tui::app::state::SLASH_COMMANDS;

fn app_source() -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui/app");
    let mut source = String::new();
    for entry in std::fs::read_dir(dir).expect("tui app dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            source.push_str(&std::fs::read_to_string(path).expect("source"));
        }
    }
    source
}

#[test]
fn no_handler_enters_help_by_assigning_the_mode_directly() {
    // `switch_mode` is the only entry that loads the catalogue, so a bare
    // assignment anywhere is the same empty screen coming back.
    assert!(
        !app_source().contains("self.mode = AppMode::Help"),
        "enter help through switch_mode(AppMode::Help), not a bare mode assignment"
    );
}

#[test]
fn the_help_slash_handler_calls_switch_mode() {
    let messaging = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui/app/messaging.rs"),
    )
    .expect("messaging source");
    let arm = messaging.find("\"/help\" =>").expect("/help arm");
    let body = &messaging[arm..arm + 400];
    assert!(
        body.contains("switch_mode(AppMode::Help)"),
        "the /help arm must route through switch_mode"
    );
}

#[test]
fn switch_mode_loads_the_catalog_on_entering_help() {
    let state = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui/app/state.rs"),
    )
    .expect("state source");
    let branch = state
        .find("if mode == AppMode::Help {")
        .expect("help branch in switch_mode");
    let body = &state[branch..branch + 600];
    assert!(
        body.contains("help_catalog::load()"),
        "entering help must reload the catalogue"
    );
}

#[test]
fn the_built_in_section_carries_the_whole_slash_table() {
    // What the empty screen was missing. Built from the static table, so this
    // holds on any machine regardless of installed skills or commands.toml.
    let rows: Vec<_> = SLASH_COMMANDS
        .iter()
        .map(|cmd| crate::tui::app::help_catalog::HelpRow {
            name: cmd.name.to_string(),
            description: cmd.description.to_string(),
            section: HelpSection::BuiltIn,
        })
        .collect();

    let built_in = section_matches(&rows, HelpSection::BuiltIn, "");
    assert_eq!(built_in.len(), SLASH_COMMANDS.len());
    for expected in ["/theme", "/clear", "/help"] {
        assert!(
            built_in.iter().any(|r| r.name == expected),
            "built-in section is missing {expected}"
        );
    }
}
