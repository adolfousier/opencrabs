//! Help-screen catalogue and its search filter (#1527).
//!
//! The matcher and the scroll clamp are pure, so both are pinned here without
//! a terminal. What is NOT tested is `load()`, which walks the skills
//! directories and reads `commands.toml`: its answer depends on the machine.

use crate::tui::app::help_catalog::{
    HelpRow, HelpSection, match_count, max_scroll, section_matches,
};

fn row(name: &str, description: &str, section: HelpSection) -> HelpRow {
    HelpRow {
        name: name.to_string(),
        description: description.to_string(),
        section,
    }
}

fn catalog() -> Vec<HelpRow> {
    vec![
        row("/new", "Start a new session", HelpSection::BuiltIn),
        row("/models", "Switch AI model", HelpSection::BuiltIn),
        row("/respond_to", "Reply to a message", HelpSection::Channel),
        row("/drop-release", "Publish a release", HelpSection::Skill),
        row("/check", "Run clippy and tests", HelpSection::Custom),
    ]
}

#[test]
fn an_empty_needle_matches_everything() {
    // So an unfiltered screen and a cleared filter take the same code path
    // rather than needing a special case at every call site.
    let rows = catalog();
    assert_eq!(match_count(&rows, ""), rows.len());
    for r in &rows {
        assert!(r.matches(""));
    }
}

#[test]
fn a_name_substring_matches() {
    assert!(row("/mission-control", "Open MC", HelpSection::BuiltIn).matches("mis"));
}

#[test]
fn a_description_substring_matches_too() {
    // The whole point of the screen is helping someone who does NOT know the
    // command's name. Searching "release" has to find a command called
    // something else whose description mentions releasing.
    let r = row(
        "/drop-release",
        "Research, draft and publish",
        HelpSection::Skill,
    );
    assert!(r.matches("publish"), "description must be searchable");
}

#[test]
fn matching_ignores_case_both_ways() {
    let r = row("/Doctor", "Diagnose Setup", HelpSection::BuiltIn);
    assert!(r.matches("doctor"));
    assert!(r.matches("DIAGNOSE"));
    assert!(r.matches("SeTuP"));
}

#[test]
fn a_needle_matching_nothing_matches_nothing() {
    assert_eq!(match_count(&catalog(), "zzzznope"), 0);
}

#[test]
fn a_filter_narrows_within_a_section() {
    let rows = catalog();
    let hits = section_matches(&rows, HelpSection::BuiltIn, "model");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "/models");
}

#[test]
fn a_filter_does_not_leak_across_sections() {
    // Each section renders under its own header, so a row must only ever
    // appear beneath the header it belongs to.
    let rows = catalog();
    let skills = section_matches(&rows, HelpSection::Skill, "");
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "/drop-release");
}

#[test]
fn section_order_is_fixed() {
    // Derived from the data, a filter could reorder the screen under the
    // reader between keystrokes.
    assert_eq!(
        HelpSection::all(),
        [
            HelpSection::BuiltIn,
            HelpSection::Channel,
            HelpSection::Skill,
            HelpSection::Custom,
        ]
    );
}

#[test]
fn every_section_has_a_header() {
    for s in HelpSection::all() {
        assert!(!s.title().is_empty(), "{s:?} renders a header");
    }
}

#[test]
fn scroll_stops_with_content_still_on_screen() {
    // The bug this replaces: `saturating_add(1)` with no ceiling wound the
    // offset past the end and left the reader on blank rows.
    assert_eq!(max_scroll(100, 30), 70);
}

#[test]
fn content_shorter_than_the_viewport_cannot_scroll() {
    assert_eq!(max_scroll(10, 30), 0);
    assert_eq!(max_scroll(0, 30), 0);
}

#[test]
fn content_exactly_filling_the_viewport_cannot_scroll() {
    assert_eq!(max_scroll(30, 30), 0);
}

#[test]
fn a_zero_height_viewport_does_not_underflow() {
    // Degenerate, but a panic here would take the whole TUI down on a
    // terminal resized to nothing.
    assert_eq!(max_scroll(50, 0), 50);
}
