//! Stop-reason mapping tests for the ACP turn bridge (#1540).

use crate::acp::turn::stop_reason;
use crate::brain::provider::StopReason;

#[test]
fn stop_reasons_map_to_acp() {
    assert_eq!(stop_reason(Some(StopReason::EndTurn)), "end_turn");
    assert_eq!(stop_reason(Some(StopReason::MaxTokens)), "max_tokens");
    assert_eq!(stop_reason(Some(StopReason::StopSequence)), "stop_sequence");
    assert_eq!(stop_reason(Some(StopReason::ToolUse)), "end_turn");
    assert_eq!(stop_reason(None), "end_turn");
}
