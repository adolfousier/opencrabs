//! Log viewer filtering, scrolling and file navigation (#1528).
//!
//! `LogViewerState::open` and `load` touch the disk; everything else is pure,
//! and it is the pure part a reader actually depends on. These build a state
//! from fixture text rather than from a real log, so the rules are pinned
//! without needing a populated `~/.opencrabs/logs`.

use crate::logging::reader::{LogLevel, parse};
use crate::tui::app::mission_control::log_viewer::{LogViewerState, level_for_key};

fn line(level: &str, target: &str, msg: &str) -> String {
    format!("2026-09-13T01:00:00.000000+01:00 {level} ThreadId(1) {target}: {msg}")
}

/// A viewer over fixture text, with a viewport tall enough not to clamp.
fn viewer(text: &str) -> LogViewerState {
    LogViewerState {
        entries: parse(text),
        viewport_rows: 100,
        ..Default::default()
    }
}

fn sample() -> LogViewerState {
    let text = [
        line("INFO", "opencrabs::boot", "started"),
        line("DEBUG", "opencrabs::brain", "thinking"),
        line("ERROR", "opencrabs::channels::whatsapp", "send failed"),
        line("WARN", "opencrabs::db", "slow query"),
    ]
    .join("\n");
    viewer(&text)
}

#[test]
fn the_default_level_shows_everything_the_app_actually_logs() {
    // Trace is off in every shipped config, so defaulting to it would promise
    // a level the file never contains.
    let v = sample();
    assert_eq!(v.min_level, LogLevel::Debug);
    assert_eq!(v.visible().len(), 4);
}

#[test]
fn a_level_filter_keeps_that_level_and_worse() {
    let mut v = sample();
    v.set_level(LogLevel::Warn);
    let levels: Vec<LogLevel> = v.visible().iter().map(|e| e.level).collect();
    assert_eq!(levels, vec![LogLevel::Error, LogLevel::Warn]);
}

#[test]
fn filtering_to_error_keeps_only_errors() {
    let mut v = sample();
    v.set_level(LogLevel::Error);
    assert_eq!(v.visible().len(), 1);
    assert_eq!(v.visible()[0].message, "send failed");
}

#[test]
fn a_level_filter_never_orphans_a_continuation_line() {
    // The failure this design exists to prevent: a naive parser would give
    // the payload lines their own (wrong) level, the filter would drop them,
    // and the entry's first line would survive — so the reader sees the
    // error announced and its body silently gone.
    let text = format!(
        "{}\n{{\n  \"reason\": \"timeout\"\n}}",
        line("ERROR", "opencrabs::net", "request failed")
    );
    let mut v = viewer(&text);
    v.set_level(LogLevel::Error);

    let visible = v.visible();
    assert_eq!(visible.len(), 1);
    assert_eq!(
        visible[0].continuation.len(),
        3,
        "the body must survive its own entry's filter"
    );
    assert_eq!(v.content_rows(), 4, "header plus three body rows");
}

#[test]
fn a_text_filter_and_a_level_filter_compose() {
    let mut v = sample();
    v.set_level(LogLevel::Warn);
    for c in "whatsapp".chars() {
        v.push_search(c);
    }
    assert_eq!(v.visible().len(), 1);
    assert_eq!(v.visible()[0].level, LogLevel::Error);
}

#[test]
fn a_text_filter_matching_nothing_hides_everything() {
    let mut v = sample();
    for c in "zzzznope".chars() {
        v.push_search(c);
    }
    assert!(v.visible().is_empty());
    // The entries are still loaded, so the renderer can say how many are
    // hidden rather than implying the file is empty.
    assert_eq!(v.entries.len(), 4);
}

#[test]
fn changing_a_filter_resets_scroll() {
    // The row the offset pointed at may no longer be on screen, or may no
    // longer exist at all.
    let mut v = sample();
    v.scroll = 3;
    v.set_level(LogLevel::Error);
    assert_eq!(v.scroll, 0);

    v.scroll = 2;
    v.push_search('x');
    assert_eq!(v.scroll, 0);
}

#[test]
fn scroll_cannot_pass_the_last_row() {
    let mut v = sample();
    v.viewport_rows = 2;
    assert_eq!(v.max_scroll(), 2);
    v.scroll_down(999);
    assert_eq!(v.scroll, 2);
}

#[test]
fn scroll_cannot_go_above_the_first_row() {
    let mut v = sample();
    v.scroll_up(999);
    assert_eq!(v.scroll, 0);
}

#[test]
fn content_shorter_than_the_viewport_cannot_scroll() {
    let mut v = sample();
    v.viewport_rows = 100;
    assert_eq!(v.max_scroll(), 0);
    v.scroll_down(10);
    assert_eq!(v.scroll, 0);
}

#[test]
fn end_lands_on_the_newest_entries() {
    // Where a just-reproduced problem lives.
    let mut v = sample();
    v.viewport_rows = 2;
    v.scroll_to_end();
    assert_eq!(v.scroll, v.max_scroll());
}

#[test]
fn content_rows_counts_continuation_lines() {
    let text = format!("{}\nbody one\nbody two", line("INFO", "a::b", "head"));
    assert_eq!(viewer(&text).content_rows(), 3);
}

#[test]
fn backspacing_an_empty_filter_closes_the_search_box() {
    // Otherwise the user is stranded in a prompt they have to Esc out of
    // anyway.
    let mut v = sample();
    v.search_active = true;
    v.pop_search();
    assert!(!v.search_active);
}

#[test]
fn backspacing_a_non_empty_filter_only_removes_a_character() {
    let mut v = sample();
    v.search_active = true;
    v.push_search('d');
    v.push_search('b');
    v.pop_search();
    assert_eq!(v.search, "d");
    assert!(v.search_active, "still typing");
}

#[test]
fn clearing_the_filter_leaves_the_search_box() {
    let mut v = sample();
    v.search_active = true;
    v.push_search('x');
    v.clear_search();
    assert!(v.search.is_empty());
    assert!(!v.search_active);
    assert_eq!(v.visible().len(), 4);
}

#[test]
fn stepping_past_either_end_of_the_file_list_stays_put() {
    // Wrapping from the newest file to the oldest would read as a jump
    // backwards in time with nothing to say it happened.
    let mut v = sample();
    v.files = vec!["a".into(), "b".into(), "c".into()];
    v.file_index = 2;
    v.step_file(1);
    assert_eq!(v.file_index, 2, "cannot go past the newest");

    v.file_index = 0;
    v.step_file(-1);
    assert_eq!(v.file_index, 0, "cannot go before the oldest");
}

#[test]
fn stepping_with_no_files_is_a_no_op() {
    let mut v = sample();
    assert!(v.files.is_empty());
    v.step_file(1);
    v.step_file(-1);
    assert_eq!(v.file_index, 0);
}

#[test]
fn level_keys_accept_digits_and_initials() {
    // Neither is obviously right: digits match the analytics window keys next
    // door, initials are what someone who knows log levels tries first.
    assert_eq!(level_for_key('1'), Some(LogLevel::Error));
    assert_eq!(level_for_key('e'), Some(LogLevel::Error));
    assert_eq!(level_for_key('2'), Some(LogLevel::Warn));
    assert_eq!(level_for_key('w'), Some(LogLevel::Warn));
    assert_eq!(level_for_key('3'), Some(LogLevel::Info));
    assert_eq!(level_for_key('i'), Some(LogLevel::Info));
    assert_eq!(level_for_key('4'), Some(LogLevel::Debug));
    assert_eq!(level_for_key('d'), Some(LogLevel::Debug));
}

#[test]
fn an_unbound_key_is_not_a_level() {
    for c in ['/', 'q', 'z', '9', '['] {
        assert_eq!(level_for_key(c), None, "{c}");
    }
}

#[test]
fn the_file_name_is_readable_with_no_file_open() {
    // The status bar renders before the first load, so this must not panic.
    assert_eq!(LogViewerState::default().file_name(), "(no file)");
}
