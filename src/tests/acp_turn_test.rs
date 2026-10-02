//! Stop-reason mapping tests for the ACP turn bridge (#1540).

use crate::acp::turn::{round_text_is_duplicate, stop_reason};
use crate::brain::provider::StopReason;

#[test]
fn stop_reasons_map_to_acp() {
    assert_eq!(stop_reason(Some(StopReason::EndTurn)), "end_turn");
    assert_eq!(stop_reason(Some(StopReason::MaxTokens)), "max_tokens");
    // #1815 F2: this asserted "stop_sequence", which is not a v1 `StopReason`
    // variant. The test was guarding the deviation, so the assertion moved
    // with the fix rather than being deleted.
    assert_eq!(stop_reason(Some(StopReason::StopSequence)), "end_turn");
    assert_eq!(stop_reason(Some(StopReason::ToolUse)), "end_turn");
    assert_eq!(stop_reason(None), "end_turn");
}

/// The v1 `StopReason` enum, copied from `schema/v1/schema.json` (247168
/// bytes, fetched 2026-10-02). `PromptResponse.stopReason` is required, so
/// every string this function can return has to be one of these five.
const V1_STOP_REASONS: &[&str] = &[
    "end_turn",
    "max_tokens",
    "max_turn_requests",
    "refusal",
    "cancelled",
];

#[test]
fn every_mapped_stop_reason_is_a_v1_variant() {
    for reason in [
        Some(StopReason::EndTurn),
        Some(StopReason::MaxTokens),
        Some(StopReason::StopSequence),
        Some(StopReason::ToolUse),
        None,
    ] {
        let mapped = stop_reason(reason);
        assert!(
            V1_STOP_REASONS.contains(&mapped),
            "`{mapped}` is not in the v1 StopReason enum; a strict client may \
             reject the whole PromptResponse over one required-field value"
        );
    }
}

#[test]
fn stop_sequence_is_no_longer_emitted() {
    // Pinned as its own test so a future "helpful" re-specialization of the
    // provider's StopSequence has to pass a deliberate edit here, not a
    // silent one.
    assert!(!V1_STOP_REASONS.contains(&"stop_sequence"));
    for reason in [
        Some(StopReason::EndTurn),
        Some(StopReason::MaxTokens),
        Some(StopReason::StopSequence),
        Some(StopReason::ToolUse),
        None,
    ] {
        // `Option<StopReason>` is not `Copy` and `stop_reason` takes it by
        // value, so the label has to be built before the move.
        let label = format!("{reason:?}");
        assert_ne!(
            stop_reason(reason),
            "stop_sequence",
            "StopReason({label}) invented a variant again"
        );
    }
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
