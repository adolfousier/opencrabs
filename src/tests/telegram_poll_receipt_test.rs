//! #1778 receipt-path pins: the poll-cycle classifier that decides which
//! log line each getUpdates response produces. The inbound-stall diagnosis
//! depends on every response shape mapping to a DISTINCT receipt (no
//! receipt lines = dispatcher starvation; Empty = healthy idle poll;
//! MalformedResult = API answered with a poisoned 200; NotOk = failure
//! detail via classify_poll_failure). These tests pin that mapping so a
//! future edit cannot silently merge two shapes back into one log line.

use crate::channels::telegram::raw_updates::PollOutcome;
use crate::channels::telegram::raw_updates::classify_poll_outcome;

#[test]
fn nonempty_result_classifies_as_updates() {
    let body = serde_json::json!({
        "ok": true,
        "result": [
            {"update_id": 1, "message": {"message_id": 10, "date": 0,
             "chat": {"id": 1, "type": "private", "first_name": "A"}, "text": "hi"}},
            {"update_id": 2}
        ]
    });
    assert!(matches!(classify_poll_outcome(&body), PollOutcome::Updates));
}

#[test]
fn empty_result_is_healthy_idle_not_malformed() {
    // The long-poll timeout returns an empty array on a quiet bot: this is
    // the HEALTHY shape and must keep its own receipt (received=0), never
    // merge with the malformed case.
    let body = serde_json::json!({"ok": true, "result": []});
    assert!(matches!(classify_poll_outcome(&body), PollOutcome::Empty));
}

#[test]
fn missing_result_on_ok_body_is_malformed() {
    // The previously-silent path (#1778): a 200 with ok=true and no result
    // array used to return without any log line.
    let body = serde_json::json!({"ok": true});
    assert!(matches!(
        classify_poll_outcome(&body),
        PollOutcome::MalformedResult
    ));
}

#[test]
fn non_array_result_is_malformed() {
    let body = serde_json::json!({"ok": true, "result": {"unexpected": "shape"}});
    assert!(matches!(
        classify_poll_outcome(&body),
        PollOutcome::MalformedResult
    ));
}

#[test]
fn not_ok_body_classifies_as_not_ok() {
    // 409-conflict and other API failures carry their own detail log via
    // classify_poll_failure; the classifier only marks the branch.
    let body = serde_json::json!({
        "ok": false,
        "error_code": 409,
        "description": "Conflict: terminated by other getUpdates request"
    });
    assert!(matches!(classify_poll_outcome(&body), PollOutcome::NotOk));
}

#[test]
fn missing_ok_field_is_not_ok() {
    // A proxy or captive portal answering 200 with HTML garbage has no
    // ok field: classified NotOk, failure detail logged, never a phantom
    // "received=0 healthy" receipt.
    let body = serde_json::json!({"error": "bad gateway"});
    assert!(matches!(classify_poll_outcome(&body), PollOutcome::NotOk));
}
