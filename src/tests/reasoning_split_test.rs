//! Reload splits a persisted turn back into its iterations (#764).
//!
//! The tool loop appends one `<!-- reasoning -->` block plus that iteration's
//! text per iteration. Reload used to split only on TOOL markers, so a turn
//! that ran no tools fused every iteration into one row. That row is the
//! turn's final answer, which the per-turn fold must always show, so a
//! reasoning-heavy turn became a wall after a restart.

use crate::tui::app::reasoning_split::{Segment, split_segments};

fn text(s: &str) -> Segment {
    Segment::Text(s.to_string())
}
fn reasoning(s: &str) -> Segment {
    Segment::Reasoning(s.to_string())
}

#[test]
fn plain_content_is_one_text_segment() {
    assert_eq!(
        split_segments("just an answer"),
        vec![text("just an answer")]
    );
}

#[test]
fn empty_content_yields_nothing() {
    assert!(split_segments("").is_empty());
    assert!(split_segments("   \n  ").is_empty());
}

#[test]
fn iterations_keep_their_chronological_order() {
    // This is the shape the tool loop writes: think, text, think, text.
    let persisted = "<!-- reasoning -->\nfirst thought\n<!-- /reasoning -->\n\n\
                     first answer\n\n\
                     <!-- reasoning -->\nsecond thought\n<!-- /reasoning -->\n\n\
                     second answer\n\n";
    assert_eq!(
        split_segments(persisted),
        vec![
            reasoning("first thought"),
            text("first answer"),
            reasoning("second thought"),
            text("second answer"),
        ]
    );
}

#[test]
fn only_the_last_text_segment_can_be_the_final_answer() {
    // The point of splitting: earlier text becomes its own row, so the fold
    // hides it instead of it fusing into the final answer.
    let persisted = "<!-- reasoning -->\nthinking\n<!-- /reasoning -->\n\n\
                     a long intermediate narration\n\n\
                     <!-- reasoning -->\nmore thinking\n<!-- /reasoning -->\n\n\
                     the actual answer";
    let segments = split_segments(persisted);
    let texts: Vec<&Segment> = segments
        .iter()
        .filter(|s| matches!(s, Segment::Text(_)))
        .collect();
    assert_eq!(texts.len(), 2, "intermediate text is its own row");
    assert_eq!(*texts[1], text("the actual answer"));
}

#[test]
fn reasoning_only_turn_produces_no_visible_text() {
    let persisted = "<!-- reasoning -->\nall thought, no answer\n<!-- /reasoning -->\n\n";
    assert_eq!(
        split_segments(persisted),
        vec![reasoning("all thought, no answer")]
    );
}

#[test]
fn an_unclosed_block_stays_reasoning() {
    // A half-written chain must not spill into the visible answer, which is
    // the exact failure this module exists to prevent.
    let persisted = "answer so far\n\n<!-- reasoning -->\ncut off mid thought";
    assert_eq!(
        split_segments(persisted),
        vec![text("answer so far"), reasoning("cut off mid thought")]
    );
}

#[test]
fn text_before_the_first_block_is_kept() {
    let persisted = "leading text\n\n<!-- reasoning -->\nthought\n<!-- /reasoning -->\n\n";
    assert_eq!(
        split_segments(persisted),
        vec![text("leading text"), reasoning("thought")]
    );
}

#[test]
fn empty_blocks_do_not_create_blank_rows() {
    let persisted = "<!-- reasoning -->\n\n<!-- /reasoning -->\n\nreal answer";
    assert_eq!(split_segments(persisted), vec![text("real answer")]);
}

#[test]
fn three_blocks_split_into_three_reasoning_rows() {
    // Matches the real row that prompted this: 3 wrapped blocks with large
    // unwrapped stretches between them, which used to fuse into one message.
    let persisted = "<!-- reasoning -->\nA\n<!-- /reasoning -->\n\nwall one\n\n\
                     <!-- reasoning -->\nB\n<!-- /reasoning -->\n\nwall two\n\n\
                     <!-- reasoning -->\nC\n<!-- /reasoning -->\n\nfinal";
    let segments = split_segments(persisted);
    assert_eq!(
        segments
            .iter()
            .filter(|s| matches!(s, Segment::Reasoning(_)))
            .count(),
        3
    );
    assert_eq!(
        segments
            .iter()
            .filter(|s| matches!(s, Segment::Text(_)))
            .count(),
        3,
        "each unwrapped stretch is its own row, so only the last is the answer"
    );
}

// ── Which text is the answer (#760 rendering consequence) ───────────────────
// The model sometimes restates its reasoning through the content channel,
// where it is persisted unwrapped and reads exactly like an answer. Position
// is the one exact signal: text the model kept thinking past is not the answer.

use crate::tui::app::reasoning_split::is_intermediate;

#[test]
fn text_followed_by_more_reasoning_is_not_the_answer() {
    let segs = vec![
        reasoning("thought one"),
        text("restated reasoning that looks like an answer"),
        reasoning("thought two"),
        text("the real answer"),
    ];
    assert!(is_intermediate(&segs, 1), "more reasoning follows it");
    assert!(
        !is_intermediate(&segs, 3),
        "nothing thinks after it, so it is the answer"
    );
}

#[test]
fn the_only_text_is_the_answer() {
    let segs = vec![reasoning("thinking"), text("answer")];
    assert!(!is_intermediate(&segs, 1));
}

#[test]
fn without_any_reasoning_every_text_stays_visible() {
    // A plain turn must not have its content collapsed.
    let segs = vec![text("first"), text("second")];
    assert!(!is_intermediate(&segs, 0));
    assert!(!is_intermediate(&segs, 1));
}

#[test]
fn terminal_reasoning_after_final_text_keeps_it_visible() {
    // CLI providers append the whole turn's reasoning as ONE block AFTER the
    // streamed text (live persistence, #269), so a finished turn's row ends
    // [answer, reasoning] even though the answer was fully delivered (#1728).
    // Terminal reasoning is an append artifact, not proof the model was still
    // thinking: the text before it stays visible.
    let segs = vec![text("the delivered report"), reasoning("turn artifact")];
    assert!(
        !is_intermediate(&segs, 0),
        "nothing but reasoning follows it, so it is the answer"
    );
}

#[test]
fn cli_reasoning_artifact_does_not_promote_narration_before_the_answer() {
    // #760 still holds when a CLI artifact trails the region: narration the
    // model kept thinking past stays collapsed, and only the real answer,
    // followed by nothing but the artifact, stays visible.
    let segs = vec![
        text("restated reasoning that looks like an answer"),
        reasoning("thought two"),
        text("the real answer"),
        reasoning("whole-turn artifact"),
    ];
    assert!(is_intermediate(&segs, 0));
    assert!(!is_intermediate(&segs, 2));
}

#[test]
fn cli_final_region_split_keeps_the_report_visible() {
    // End-to-end shape of a CLI row's final region: streamed report text,
    // then the whole-turn reasoning block appended at EOF. Splitting must
    // classify the report as the answer, not demote it into hidden details.
    let persisted = "step 9 done, receipts posted\n\n\
                     <!-- reasoning -->\nwhole turn thought\n<!-- /reasoning -->";
    let segs = split_segments(persisted);
    assert_eq!(
        segs,
        vec![
            text("step 9 done, receipts posted"),
            reasoning("whole turn thought")
        ]
    );
    assert!(!is_intermediate(&segs, 0));
}
