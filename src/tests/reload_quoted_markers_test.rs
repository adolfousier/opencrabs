//! Regression for #1587: a marker the model quoted inside its own text is
//! not a real marker on reload.
//!
//! The model read its own history through `session_search` and quoted, in
//! its reasoning, an old row's reasoning markers and a ledger opener cut off
//! by an ellipsis. The reload splitter took the quoted ledger opener as
//! real, failed to parse it, and dumped everything after it as the answer;
//! the reasoning splitter took the quoted close as real and made the prose
//! after it the answer. Live the turn had rendered collapsed, because the
//! live view never parses the row.
//!
//! Real markers start a line, always: the persist path writes them that
//! way (109,369 of 109,755 reasoning openers and 122,582 of 122,715 ledger
//! openers on a live database, every mid-line one a quote). So a marker
//! counts only at a line start, and a candidate that does not parse is
//! skipped rather than ending the scan.

use crate::tui::app::ledger_scan::{find_at_line_start, next_ledger};
use crate::tui::app::reasoning_split::{Segment, is_intermediate, split_segments};

/// Tonight's row, in shape: a real reasoning block that quotes markers and a
/// truncated ledger opener mid-sentence, then the real ledger on its own line.
fn tonight_row() -> String {
    concat!(
        "<!-- reasoning -->\n",
        "Interesting. Let me re-read the assistant message from the tail:\n\n",
        "\"[assistant: <!-- reasoning --> The wrapper's rc=0, but that just means the outer ",
        "command executed. <!-- /reasoning --> The wrapper…\"\n\n",
        "And the search result displayed: \"Only 2 files to commit: <!-- tools-v2: ",
        "[{\"d\":\"bash: git status --short; echo ===VERIFY-MOUNTS; echo ===ORG…\"\n\n",
        "===VERIFY-MOUNTS greps the file in App.tsx, then \"===ORG\" grabs the org picker.\n\n",
        "No, actually, maybe I should fetch the row from the DB. Let me check the schema.\n",
        "<!-- /reasoning -->\n\n\n",
        "<!-- tools-v2: [{\"d\":\"bash: sqlite3 db \\\"PRAGMA table_info(messages);\\\"\",",
        "\"o\":\"STDOUT:\\n0|id|TEXT\",\"s\":true}] -->\n",
    )
    .to_string()
}

#[test]
fn the_quoted_ledger_opener_is_skipped_and_the_real_one_found() {
    let row = tonight_row();
    let ledger = next_ledger(&row).expect("the real ledger parses");
    let real = row.rfind("<!-- tools-v2:").unwrap();
    assert_eq!(
        ledger.start, real,
        "the mid-sentence quote is not the ledger"
    );
    assert!(ledger.is_v2);
    let calls: Vec<serde_json::Value> = serde_json::from_str(ledger.body).expect("json body");
    assert_eq!(
        calls[0]["d"],
        "bash: sqlite3 db \"PRAGMA table_info(messages);\""
    );
    assert_eq!(&row[ledger.end - 3..ledger.end], "-->");
    assert!(next_ledger(&row[ledger.end..]).is_none());
}

#[test]
fn the_text_before_the_real_ledger_is_one_collapsed_reasoning_block() {
    let row = tonight_row();
    let ledger = next_ledger(&row).unwrap();
    let segments = split_segments(row[..ledger.start].trim());
    assert_eq!(segments.len(), 1, "{segments:?}");
    let Segment::Reasoning(inner) = &segments[0] else {
        panic!("expected reasoning, got {segments:?}");
    };
    assert!(
        inner.contains("VERIFY-MOUNTS greps"),
        "the quoted prose stays inside the block"
    );
    assert!(
        inner.contains("<!-- /reasoning --> The wrapper"),
        "the quote is kept verbatim"
    );
    assert!(
        !segments.iter().any(|s| matches!(s, Segment::Text(_))),
        "nothing from the quote becomes visible answer text"
    );
}

#[test]
fn a_quoted_close_mid_line_does_not_end_the_real_block() {
    let row = "<!-- reasoning -->\nthinking, quoting <!-- /reasoning --> here\nstill thinking\n<!-- /reasoning -->\n\nThe answer.";
    let segments = split_segments(row);
    assert_eq!(
        segments,
        vec![
            Segment::Reasoning(
                "thinking, quoting <!-- /reasoning --> here\nstill thinking".to_string()
            ),
            Segment::Text("The answer.".to_string()),
        ]
    );
    assert!(!is_intermediate(&segments, 1));
}

#[test]
fn an_orphaned_marker_line_is_dropped_from_visible_text() {
    let segments = split_segments("Answer line one\n<!-- /reasoning -->\nAnswer line two");
    assert_eq!(
        segments,
        vec![Segment::Text(
            "Answer line one\nAnswer line two".to_string()
        )]
    );
    let segments = split_segments("<!-- /phantom_blocked=1 -->\nJust the answer.");
    assert_eq!(
        segments,
        vec![Segment::Text("Just the answer.".to_string())]
    );
}

#[test]
fn a_line_start_opener_with_broken_json_is_skipped_not_fatal() {
    let s = "\n<!-- tools-v2: [{\"d\":\"broken\n\nplain text after it\n<!-- tools-v2: [{\"d\":\"ok\",\"s\":true}] -->\n";
    let ledger = next_ledger(s).expect("the second opener parses");
    assert_eq!(ledger.start, s.rfind("<!-- tools-v2:").unwrap());
    assert!(ledger.body.contains("\"ok\""));
}

#[test]
fn a_v1_ledger_still_parses_at_line_start() {
    let s = "text\n<!-- tools: read a | grep b -->\nmore";
    let ledger = next_ledger(s).expect("v1 parses");
    assert!(!ledger.is_v2);
    assert_eq!(ledger.body, "read a | grep b");
    assert_eq!(&s[ledger.end..], "\nmore");
}

#[test]
fn only_quoted_openers_means_no_ledger() {
    assert!(next_ledger("prose <!-- tools-v2: [{\"d\":\"x\"}] --> more prose").is_none());
    assert!(next_ledger("see `<!-- tools:` in the docs").is_none());
}

#[test]
fn an_inner_arrow_inside_a_string_still_closes_correctly() {
    let s = "<!-- tools-v2: [{\"d\":\"cargo build\",\"o\":\"error --> src/main.rs:42\",\"s\":false}] -->\ntail";
    let ledger = next_ledger(s).unwrap();
    assert_eq!(&s[ledger.end..], "\ntail");
}

#[test]
fn line_start_search_accepts_offset_zero_and_after_newlines_only() {
    assert_eq!(find_at_line_start("<!-- x -->", "<!-- x -->", 0), Some(0));
    assert_eq!(
        find_at_line_start("a <!-- x -->\n<!-- x -->", "<!-- x -->", 0),
        Some(13)
    );
    assert_eq!(find_at_line_start("a <!-- x --> b", "<!-- x -->", 0), None);
}
