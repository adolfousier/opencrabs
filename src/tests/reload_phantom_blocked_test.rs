//! Regression for #1584: a phantom-blocked section never reloads as the
//! turn's answer.
//!
//! The tool loop persists an iteration the phantom detector refused under
//! `<!-- phantom_blocked=1 -->` markers (#1172). The reload split knew only
//! the reasoning markers, so the section came apart into its opener line,
//! its reasoning, and its narration plus closing marker as the last text
//! segment, which is the final answer by position. A 21K-character wall the
//! self-heal had drained from the live view rendered in full on the next
//! reload, closing marker as its last line.

use crate::tui::app::reasoning_split::{BLOCKED_LABEL, Segment, is_intermediate, split_segments};

const OPEN: &str = "<!-- phantom_blocked=1 -->";
const CLOSE: &str = "<!-- /phantom_blocked=1 -->";

/// The persisted layout, exactly as the tool loop appends it.
fn blocked_section(reasoning: &str, narration: &str) -> String {
    format!(
        "{OPEN}\n<!-- reasoning -->\n{reasoning}\n<!-- /reasoning -->\n\n{narration}\n\n\n{CLOSE}\n"
    )
}

fn no_markers(segments: &[Segment]) {
    for seg in segments {
        let text = match seg {
            Segment::Reasoning(t) | Segment::Text(t) | Segment::Blocked(t) => t,
        };
        assert!(
            !text.contains("<!--"),
            "a marker reached a segment: {text:?}"
        );
    }
}

#[test]
fn a_blocked_row_is_one_collapsed_segment_with_no_markers() {
    let row = blocked_section(
        "The user wants a read-only audit.",
        "I need to look into this further. Let me check the details.",
    );
    let segments = split_segments(&row);
    assert_eq!(segments.len(), 1, "{segments:?}");
    let Segment::Blocked(body) = &segments[0] else {
        panic!("expected a blocked segment, got {segments:?}");
    };
    assert!(body.starts_with("The user wants a read-only audit."));
    assert!(body.ends_with("Let me check the details."));
    no_markers(&segments);
}

#[test]
fn an_unclosed_opener_folds_to_the_end_of_the_row() {
    let row = format!(
        "{OPEN}\n<!-- reasoning -->\nthinking\n<!-- /reasoning -->\n\nhalf-written narration"
    );
    let segments = split_segments(&row);
    assert_eq!(
        segments,
        vec![Segment::Blocked(
            "thinking\n\nhalf-written narration".to_string()
        )]
    );
}

#[test]
fn text_around_a_blocked_section_keeps_its_place() {
    let row = format!(
        "First answer.\n\n{}\nSecond answer.",
        blocked_section("why", "narration")
    );
    let segments = split_segments(&row);
    assert_eq!(
        segments,
        vec![
            Segment::Text("First answer.".to_string()),
            Segment::Blocked("why\n\nnarration".to_string()),
            Segment::Text("Second answer.".to_string()),
        ]
    );
}

#[test]
fn the_answer_before_a_blocked_section_stays_the_answer() {
    let row = format!(
        "<!-- reasoning -->\nplan\n<!-- /reasoning -->\n\nHere is the result.\n\n{}",
        blocked_section("second thoughts", "Let me also check one more thing.")
    );
    let segments = split_segments(&row);
    assert_eq!(segments.len(), 3, "{segments:?}");
    assert!(matches!(segments[1], Segment::Text(ref t) if t == "Here is the result."));
    assert!(
        !is_intermediate(&segments, 1),
        "the reasoning folded into a blocked section must not demote the real answer"
    );
    assert!(matches!(segments[2], Segment::Blocked(_)));
}

#[test]
fn two_blocked_sections_stay_separate_rows() {
    let row = format!(
        "{}{}",
        blocked_section("a", "first attempt"),
        blocked_section("b", "second attempt")
    );
    let segments = split_segments(&row);
    assert_eq!(segments.len(), 2);
    assert!(segments.iter().all(|s| matches!(s, Segment::Blocked(_))));
}

#[test]
fn reload_renders_a_blocked_segment_collapsed_and_labelled() {
    const SRC: &str = include_str!("../tui/app/messaging.rs");
    let arm = SRC
        .find("Segment::Blocked(b) =>")
        .expect("push_segments handles blocked segments");
    let window = &SRC[arm..arm + 200];
    assert!(
        window.contains("String::new()") && window.contains("BLOCKED_LABEL"),
        "a blocked segment is details only, under the label: {window}"
    );
    assert!(!BLOCKED_LABEL.is_empty());
}
