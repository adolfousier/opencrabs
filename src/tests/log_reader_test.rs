//! Parsing the log files `logging::logger` writes (#1528).
//!
//! Every fixture below is shaped like a real line from
//! `~/.opencrabs/logs/opencrabs.YYYY-MM-DD`, because the format is not
//! specified anywhere except by what the writer emits.

use crate::logging::reader::{LogLevel, parse};

/// One real header line, abbreviated only in the message.
const HEADER: &str = "2026-09-13T01:00:00.331731+01:00 DEBUG ThreadId(66) opencrabs::brain::provider::claude_cli: src/brain/provider/claude_cli.rs:884: CLI stdout raw";

#[test]
fn a_header_line_parses_into_an_entry() {
    let entries = parse(HEADER);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.level, LogLevel::Debug);
    assert_eq!(e.target, "opencrabs::brain::provider::claude_cli");
    assert_eq!(e.message, "CLI stdout raw");
    assert_eq!(e.timestamp, "2026-09-13T01:00:00.331731+01:00");
}

#[test]
fn the_file_and_line_prefix_is_dropped_from_the_message() {
    // `src/brain/provider/claude_cli.rs:884:` repeats the target with less
    // context and costs a third of the row's width.
    assert!(!parse(HEADER)[0].message.contains(".rs:"));
}

#[test]
fn every_level_is_recognised() {
    for (token, want) in [
        ("ERROR", LogLevel::Error),
        ("WARN", LogLevel::Warn),
        ("INFO", LogLevel::Info),
        ("DEBUG", LogLevel::Debug),
        ("TRACE", LogLevel::Trace),
    ] {
        let line = format!("2026-09-13T01:00:00.000000+01:00 {token} ThreadId(1) a::b: hello");
        assert_eq!(parse(&line)[0].level, want, "token {token}");
    }
}

#[test]
fn a_continuation_line_joins_the_entry_above_it() {
    // THE bug this parser exists to avoid. A logged message containing
    // newlines spills onto following lines with no timestamp and no level.
    // A line-based parser gives those a made-up level and a level filter
    // then drops them — silently eating the JSON payload or stack trace that
    // was the reason to open the log, while keeping its first line so the
    // filter still looks like it worked.
    let text = format!("{HEADER}\n{{\n  \"key\": \"value\"\n}}");
    let entries = parse(&text);

    assert_eq!(entries.len(), 1, "the body is not three more entries");
    assert_eq!(entries[0].continuation.len(), 3);
    assert_eq!(entries[0].lines(), 4);
}

#[test]
fn a_continuation_line_starting_with_a_level_word_is_still_a_continuation() {
    // Captured command output can begin with the word INFO. Without the
    // timestamp check that line would split the entry in two and orphan the
    // rest of the payload.
    let text = format!("{HEADER}\nINFO this came from a subprocess");
    let entries = parse(&text);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].continuation,
        vec!["INFO this came from a subprocess"]
    );
}

#[test]
fn continuation_lines_before_any_header_are_discarded() {
    // The tail window can open mid-entry. Rendering half a payload under no
    // header is a mystery rather than information.
    let entries = parse("  \"orphaned\": true\n}\n");
    assert!(entries.is_empty());
}

#[test]
fn a_blank_line_does_not_become_an_entry() {
    let text = format!("{HEADER}\n\n");
    let entries = parse(&text);
    assert_eq!(entries.len(), 1);
}

#[test]
fn at_least_keeps_the_level_asked_for_and_everything_worse() {
    // Error is most severe and sorts first, so the comparison reads backwards
    // from the intent. Pinned here so a refactor cannot quietly invert it.
    assert!(LogLevel::Error.at_least(LogLevel::Warn));
    assert!(LogLevel::Warn.at_least(LogLevel::Warn));
    assert!(!LogLevel::Info.at_least(LogLevel::Warn));
    assert!(!LogLevel::Debug.at_least(LogLevel::Warn));
}

#[test]
fn at_least_debug_keeps_almost_everything() {
    for level in [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
    ] {
        assert!(level.at_least(LogLevel::Debug), "{level:?}");
    }
    assert!(!LogLevel::Trace.at_least(LogLevel::Debug));
}

#[test]
fn a_search_needle_reaches_into_continuation_lines() {
    // The reason to search a log is usually a string inside a payload or a
    // stack trace, not one in the first line.
    let text = format!("{HEADER}\n  \"session_id\": \"abc-123\"");
    let entry = &parse(&text)[0];
    assert!(entry.matches("abc-123"), "body text must be searchable");
}

#[test]
fn a_search_needle_matches_the_target_and_the_message() {
    let entry = &parse(HEADER)[0];
    assert!(entry.matches("claude_cli"));
    assert!(entry.matches("stdout"));
}

#[test]
fn search_ignores_case() {
    let entry = &parse(HEADER)[0];
    assert!(entry.matches("CLAUDE_CLI"));
    assert!(entry.matches("StdOut"));
}

#[test]
fn an_empty_needle_matches_every_entry() {
    assert!(parse(HEADER)[0].matches(""));
}

#[test]
fn a_needle_matching_nothing_matches_nothing() {
    assert!(!parse(HEADER)[0].matches("zzzznope"));
}

#[test]
fn a_line_with_no_thread_id_still_parses() {
    // The thread id is skipped rather than required, so a format tweak that
    // drops it does not blank the target on every row.
    let line = "2026-09-13T01:00:00.000000+01:00 WARN opencrabs::channels: something happened";
    let e = &parse(line)[0];
    assert_eq!(e.target, "opencrabs::channels");
    assert_eq!(e.message, "something happened");
}
