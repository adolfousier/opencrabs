//! The help-screen command catalogue (#1530).
//!
//! `load()` is not tested: it walks the skills directories and reads
//! `commands.toml`, so its answer depends on the machine. What is pinned is
//! the grouping the renderer depends on.

use crate::tui::app::help_catalog::{HelpRow, HelpSection, max_scroll, section_rows};

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
fn rows_are_grouped_by_section() {
    let rows = catalog();
    assert_eq!(section_rows(&rows, HelpSection::BuiltIn).len(), 2);
    assert_eq!(section_rows(&rows, HelpSection::Channel).len(), 1);
    assert_eq!(section_rows(&rows, HelpSection::Skill).len(), 1);
    assert_eq!(section_rows(&rows, HelpSection::Custom).len(), 1);
}

#[test]
fn a_section_never_yields_another_section_row() {
    // Each section renders under its own header, so a row must only ever
    // appear beneath the header it belongs to.
    let rows = catalog();
    let skills = section_rows(&rows, HelpSection::Skill);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "/drop-release");
}

#[test]
fn rows_keep_catalogue_order_within_a_section() {
    let rows = catalog();
    let names: Vec<&str> = section_rows(&rows, HelpSection::BuiltIn)
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, vec!["/new", "/models"]);
}

#[test]
fn section_order_is_fixed() {
    // Derived from the data, the screen could reorder itself between visits.
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
