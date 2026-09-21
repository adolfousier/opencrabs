//! Stop-reason mapping tests for the ACP turn bridge (#1540).

use crate::acp::turn::{round_text_is_duplicate, stop_reason};
use crate::brain::provider::StopReason;

#[test]
fn stop_reasons_map_to_acp() {
    assert_eq!(stop_reason(Some(StopReason::EndTurn)), "end_turn");
    assert_eq!(stop_reason(Some(StopReason::MaxTokens)), "max_tokens");
    assert_eq!(stop_reason(Some(StopReason::StopSequence)), "stop_sequence");
    assert_eq!(stop_reason(Some(StopReason::ToolUse)), "end_turn");
    assert_eq!(stop_reason(None), "end_turn");
}

#[test]
fn round_aggregate_repeating_streamed_text_is_a_duplicate() {
    // The wire-proven doubling: the loop streams the answer and then fires
    // the round aggregate with the same text (smoke phase B: two identical
    // `agent_message_chunk` frames, "ACP-OK" twice).
    assert!(round_text_is_duplicate("ACP-OK", "ACP-OK"));
}

#[test]
fn whitespace_disagreement_is_still_a_duplicate() {
    assert!(round_text_is_duplicate("ACP-OK", "ACP-OK\n"));
    assert!(round_text_is_duplicate(" ACP-OK ", "ACP-OK"));
}

#[test]
fn unstreamed_or_differing_text_is_not_a_duplicate() {
    // CLI providers stream nothing — the aggregate is the only delivery.
    assert!(!round_text_is_duplicate("", "ACP-OK"));
    // Round 2 after a tool call: fresh stream, different aggregate.
    assert!(!round_text_is_duplicate("round one", "round two"));
    // An empty aggregate would only add noise; never treat it as a dup.
    assert!(!round_text_is_duplicate("ACP-OK", ""));
    assert!(!round_text_is_duplicate("", "\n"));
}
