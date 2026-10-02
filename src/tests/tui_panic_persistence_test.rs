// Witness for #1791: the TUI panic hook computed a source location, a
// force-captured backtrace and the first OpenCrabs frame, stashed one of them
// for the render `catch_unwind` path, and threw the rest away. A panic
// anywhere else unwound out of the event loop and the process died with its
// own post-mortem: an instance that exited between 22:12:59 and 22:18:41 left
// zero trace in the daily log, so "check the logs" could not be answered.
//
// These tests cover the record that hook now persists: the shape of the line,
// the payload reader that must never panic inside a panic, the cap on the
// backtrace, the append to `panic.log`, the re-entrancy latch, and one
// structural pin on the hook itself so the persistence cannot be
// refactored back out silently.

use std::any::Any;
use std::path::{Path, PathBuf};

use crate::logging::panic_record::{
    BACKTRACE_CAP, PANIC_LOG_NAME, PanicRecord, TRUNCATION_MARK, panic_log_path, payload_message,
    persist, persist_into,
};

fn record(file: Option<&str>, line: Option<u32>, column: Option<u32>) -> PanicRecord {
    PanicRecord {
        file: file.map(str::to_string),
        line,
        column,
        message: "index out of bounds: the len is 3 but the index is 3".to_string(),
        opencrabs_frame: None,
    }
}

fn header_with_frame() -> PanicRecord {
    PanicRecord {
        opencrabs_frame: Some(
            "opencrabs::tui::render::chat::draw at src/tui/render/chat.rs:410:9".to_string(),
        ),
        ..record(Some("src/tui/render/chat.rs"), Some(412), Some(18))
    }
}

#[test]
fn a_panic_site_with_file_line_and_column_is_named_exactly() {
    let r = record(Some("src/tui/render/chat.rs"), Some(412), Some(18));
    assert_eq!(r.location(), "src/tui/render/chat.rs:412:18");
    assert_eq!(
        r.header(),
        "PANIC src/tui/render/chat.rs:412:18 :: index out of bounds: the len is 3 but the index is 3"
    );
}

#[test]
fn a_missing_column_still_yields_a_usable_site() {
    let r = record(Some("src/tui/app.rs"), Some(77), None);
    assert_eq!(r.location(), "src/tui/app.rs:77");
    assert!(r.header().starts_with("PANIC src/tui/app.rs:77 :: "));
}

#[test]
fn a_record_with_no_location_says_unknown_rather_than_inventing_one() {
    let r = record(None, None, None);
    assert_eq!(r.location(), "unknown location");
    assert!(
        r.header().starts_with("PANIC unknown location :: "),
        "the prefix is the grep handle: {}",
        r.header()
    );
    assert!(!r.header().contains("::: "));
}

#[test]
fn the_first_opencrabs_frame_precedes_the_message() {
    let h = header_with_frame().header();
    assert!(
        h.contains("opencrabs::tui::render::chat::draw at src/tui/render/chat.rs:410:9"),
        "the caller widget is the whole point of capturing a backtrace: {h}"
    );
    assert!(
        h.find("opencrabs::tui") < h.find("index out of bounds"),
        "location and caller before the payload: {h}"
    );
}

#[test]
fn the_message_survives_without_a_frame() {
    let h = record(Some("src/tui/app.rs"), Some(9), Some(1)).header();
    assert!(h.ends_with(":: index out of bounds: the len is 3 but the index is 3"));
    assert_eq!(
        h.matches("::").count(),
        1,
        "no empty caller slot left behind: {h}"
    );
}

#[test]
fn a_str_payload_and_a_string_payload_read_identically() {
    let literal: &str = "terminal left in raw mode";
    let owned = String::from("terminal left in raw mode");
    let a: &(dyn Any + Send) = &literal;
    let b: &(dyn Any + Send) = &owned;
    assert_eq!(payload_message(a), payload_message(b));
    assert_eq!(payload_message(a), "terminal left in raw mode");
}

#[test]
fn a_non_string_payload_is_described_not_dropped() {
    let n: &(dyn Any + Send) = &404u32;
    let msg = payload_message(n);
    assert!(
        !msg.is_empty(),
        "a payload we cannot read still needs a word: {msg}"
    );
    assert_eq!(msg, "panic payload was not a string");
    assert_eq!(payload_message(&'x'), "x");
}

#[test]
fn the_backtrace_survives_verbatim() {
    let bt = "   0: core::panicking::panic\n             at library/core/src/panicking.rs:52\n";
    let body = record(Some("src/tui/app.rs"), Some(3), Some(1)).report(bt);
    assert!(body.starts_with("PANIC src/tui/app.rs:3:1 :: "));
    assert!(body.contains(bt), "the raw stack is the record: {body}");
    assert!(!body.contains(TRUNCATION_MARK));
}

#[test]
fn an_uncaptured_backtrace_says_so() {
    let body = record(Some("src/tui/app.rs"), Some(3), Some(1)).report("");
    assert!(body.contains("backtrace: none captured"));
}

#[test]
fn a_backtrace_at_exactly_the_cap_is_kept_whole() {
    let bt = "f".repeat(BACKTRACE_CAP);
    let body = record(Some("src/tui/app.rs"), Some(3), Some(1)).report(&bt);
    assert!(body.contains(&bt));
    assert!(!body.contains(TRUNCATION_MARK));
}

#[test]
fn an_over_cap_backtrace_is_cut_on_a_char_boundary_and_marked() {
    // Head ends one byte before the cap, so byte BACKTRACE_CAP lands inside the
    // first three-byte follow-up character. A naive slice would panic in the
    // middle of handling a panic; the renderer has to walk back instead.
    let head = "x".repeat(BACKTRACE_CAP - 2);
    let follow = "a\u{3042}".repeat(64);
    let tail = "TAIL-PAST-THE-CAP";
    let bt = format!("{head}{follow}{tail}");
    assert!(bt.len() > BACKTRACE_CAP, "the fixture must exceed the cap");
    assert!(
        !bt.is_char_boundary(BACKTRACE_CAP),
        "if the naive cut were already safe, this test would prove nothing"
    );

    let body = record(Some("src/tui/app.rs"), Some(3), Some(1)).report(&bt);
    assert!(
        body.contains(TRUNCATION_MARK),
        "truncation must be visible: {body}"
    );
    assert!(!body.contains(tail));
    let kept = body
        .strip_suffix(&format!("\n{TRUNCATION_MARK}\n"))
        .expect("shape");
    assert!(
        kept.contains(&head),
        "the frames nearest the panic are the useful ones"
    );
    assert!(
        kept.ends_with('a'),
        "cut walked back to a boundary, mid-char is banned"
    );
    assert!(
        !kept.contains('\u{3042}'),
        "no partial character was written"
    );
    assert!(kept.is_char_boundary(kept.len()));
}

#[test]
fn the_panic_log_sits_in_the_log_dir() {
    assert_eq!(
        panic_log_path(Path::new("/tmp/x/logs")),
        PathBuf::from("/tmp/x/logs").join(PANIC_LOG_NAME)
    );
    assert_eq!(PANIC_LOG_NAME, "panic.log");
}

#[test]
fn persist_into_creates_a_missing_directory_and_appends_in_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A subdirectory nothing created yet: a dying process cannot assume the log
    // dir exists, since the panic may precede any logging at all.
    let nested = dir.path().join("runs").join("logs");
    let path = panic_log_path(&nested);
    assert!(!nested.exists());

    let first = record(Some("src/a.rs"), Some(1), Some(1));
    let second = record(Some("src/b.rs"), Some(2), Some(2));
    assert!(persist_into(&first, "stack one\n", &path));
    assert!(persist_into(&second, "stack two\n", &path));

    let body = std::fs::read_to_string(&path).expect("record readable");
    let at_first = body
        .find("PANIC src/a.rs:1:1")
        .expect("first record present");
    let at_second = body
        .find("PANIC src/b.rs:2:2")
        .expect("second record present");
    assert!(at_first < at_second, "oldest first: {body}");
    assert!(body.contains("stack one"));
    assert!(body.contains("stack two"));
}

#[test]
fn a_directory_that_cannot_be_created_fails_quietly_instead_of_panicking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("notadir");
    std::fs::write(&blocker, b"i am a file").expect("blocker written");
    let path = panic_log_path(&blocker.join("logs"));

    let wrote = persist_into(&record(Some("src/a.rs"), Some(1), Some(1)), "bt\n", &path);
    assert!(
        !wrote,
        "an unwritable sink reports failure rather than exploding"
    );
    assert!(!path.exists());
}

#[test]
fn the_latch_refuses_a_second_record_and_releases_on_drop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = record(Some("src/a.rs"), Some(1), Some(1));

    let held = crate::logging::panic_record::PersistGuard::try_enter();
    assert!(held.is_some(), "the latch starts free");
    assert!(
        persist(&r, "bt\n", dir.path()).is_none(),
        "a panic while a record is in flight must not recurse"
    );
    assert!(
        !panic_log_path(dir.path()).exists(),
        "the refused call wrote nothing"
    );

    drop(held);
    let path = persist(&r, "bt\n", dir.path());
    assert!(path.is_some(), "the latch released with its holder");
    assert!(panic_log_path(dir.path()).exists());
}

#[test]
fn the_tui_hook_persists_before_restoring_the_terminal() {
    let src = include_str!("../tui/runner.rs");
    let start = src
        .find("let default_hook = std::panic::take_hook();")
        .expect("the hook installs itself in run()");
    let end = src[start..]
        .find("}));")
        .map(|i| start + i)
        .expect("the hook closure closes");
    let hook = &src[start..end];

    assert!(
        hook.contains("persist("),
        "#1791: the hook has to hand the record to logging, not just to stderr"
    );
    assert!(
        hook.contains("payload_message("),
        "the message has to be read without downcast-unwrap"
    );
    let at_persist = hook.find("persist(").unwrap();
    let at_restore = hook
        .find("force_restore_terminal()")
        .expect("the terminal is still restored");
    assert!(
        at_persist < at_restore,
        "restoring can block on a write; the record goes first"
    );
    assert!(
        hook.contains("default_hook(info)"),
        "the default stderr report is kept, not replaced"
    );
    assert!(
        hook.contains("LAST_PANIC_LOCATION.lock()"),
        "the render catch_unwind correlation still reads the stash"
    );
}
