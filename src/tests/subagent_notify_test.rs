//! session_notify tool: delivery-mode resolution, route confirmation, status verdicts, and the schema (#23, fork #50).

use crate::brain::agent::service::notify_policy::{
    CONFIRM_CAP, DeliveryMode, confirm_route, resolve_mode,
};
use crate::brain::tools::subagent::notify::*;
use crate::brain::tools::r#trait::Tool;
use std::time::Duration;

#[test]
fn verdict_carries_state_and_extra_metadata() {
    let result = verdict(
        true,
        "redirected",
        "detail".into(),
        &[
            ("notify_target", "t".into()),
            ("notify_occupant", "o".into()),
        ],
    );
    assert!(result.success);
    assert_eq!(result.metadata.get("notify_state").unwrap(), "redirected");
    assert_eq!(result.metadata.get("notify_target").unwrap(), "t");
    assert_eq!(result.metadata.get("notify_occupant").unwrap(), "o");
}

#[test]
fn verdict_refusal_is_error_with_state() {
    let result = verdict(
        false,
        "refused",
        "detail".into(),
        &[("notify_reason", "mid_turn".into())],
    );
    assert!(!result.success);
    assert_eq!(result.metadata.get("notify_state").unwrap(), "refused");
    assert_eq!(result.metadata.get("notify_reason").unwrap(), "mid_turn");
}

#[test]
fn verdict_states_mirror_delivery_enum() {
    // The v2 external vocabulary (fork #50): every internal Delivery
    // variant maps to exactly one of these states.
    const STATES: [&str; 4] = ["delivered", "queued", "redirected", "refused"];
    for state in STATES {
        let result = verdict(true, state, "d".into(), &[]);
        assert_eq!(result.metadata.get("notify_state").unwrap(), state);
    }
}

#[test]
fn resolve_mode_defaults_and_alias() {
    use DeliveryMode::*;
    // Default: refuse while streaming (the fork #13 failsafe).
    assert!(matches!(resolve_mode(None, None, None).unwrap(), Now));
    // The interrupt alias maps exactly onto the modes.
    assert!(matches!(
        resolve_mode(None, Some(true), None).unwrap(),
        TurnEnd
    ));
    assert!(matches!(
        resolve_mode(None, Some(false), None).unwrap(),
        Now
    ));
    assert!(matches!(
        resolve_mode(Some("now"), None, None).unwrap(),
        Now
    ));
    assert!(matches!(
        resolve_mode(Some("turn-end"), None, None).unwrap(),
        TurnEnd
    ));
}

#[test]
fn resolve_mode_agreeing_pair_passes_disagreement_rejected() {
    use DeliveryMode::*;
    assert!(matches!(
        resolve_mode(Some("turn-end"), Some(true), None).unwrap(),
        TurnEnd
    ));
    assert!(matches!(
        resolve_mode(Some("now"), Some(false), None).unwrap(),
        Now
    ));
    assert!(resolve_mode(Some("now"), Some(true), None).is_err());
    assert!(resolve_mode(Some("turn-end"), Some(false), None).is_err());
}

#[test]
fn resolve_mode_rejects_unknown_modes() {
    let err = resolve_mode(Some("warp"), None, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("not available yet"), "got: {err}");
}

#[test]
fn resolve_mode_quiet_defaults_and_custom_windows() {
    use DeliveryMode::*;
    let default = resolve_mode(Some("quiet"), None, None).unwrap();
    assert!(
        matches!(default, Quiet { quiet_for, max_delay }
            if quiet_for == Duration::from_secs(60) && max_delay == Duration::from_secs(1800)),
        "got: {default:?}"
    );
    let custom = serde_json::json!({ "quiet_for_secs": 300, "max_delay_secs": 60 });
    let got = resolve_mode(Some("quiet"), None, Some(&custom)).unwrap();
    assert!(
        matches!(got, Quiet { quiet_for, max_delay }
            if quiet_for == Duration::from_secs(300) && max_delay == Duration::from_secs(60)),
        "got: {got:?}"
    );
    // Non-integer window is rejected, not silently defaulted.
    let bad = serde_json::json!({ "quiet_for_secs": "soon" });
    assert!(resolve_mode(Some("quiet"), None, Some(&bad)).is_err());
}

#[test]
fn resolve_mode_quiet_contradicts_interrupt_true() {
    // quiet WAITS; interrupt=true DERAILS — passing both is an error,
    // never a silent precedence.
    assert!(resolve_mode(Some("quiet"), Some(true), None).is_err());
    // interrupt=false is the natural form and passes.
    assert!(resolve_mode(Some("quiet"), Some(false), None).is_ok());
}

// confirm_route (owner-approved state-diag, 2026-09-01): each test uses
// its own session uuid — the probe registry is a process-wide static.

#[tokio::test]
async fn confirm_reports_pending_drain_when_target_already_mid_turn() {
    use crate::brain::agent::service::session_routes::register_turn_probe;
    let session = uuid::Uuid::new_v4();
    register_turn_probe(session, std::sync::Arc::new(|| true));
    let (state, _, reason) = confirm_route(session, Duration::from_millis(100)).await;
    assert_eq!(state, "queued_pending_drain");
    assert_eq!(reason, "mid_turn");
}

#[tokio::test]
async fn confirm_reports_woke_when_probe_flips_true() {
    use crate::brain::agent::service::session_routes::register_turn_probe;
    let session = uuid::Uuid::new_v4();
    register_turn_probe(session, std::sync::Arc::new(|| false));
    // The idle target "starts a turn" 300ms after the send.
    let wake = session;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        register_turn_probe(wake, std::sync::Arc::new(|| true));
    });
    let (state, detail, reason) = confirm_route(session, Duration::from_secs(3)).await;
    assert_eq!(state, "woke");
    assert_eq!(reason, "wake_confirmed");
    assert!(detail.contains("started a turn"), "got: {detail}");
}

#[tokio::test]
async fn confirm_times_out_to_honest_unconfirmed_delivered() {
    use crate::brain::agent::service::session_routes::register_turn_probe;
    let session = uuid::Uuid::new_v4();
    // An idle target that never wakes: probe reads false throughout.
    register_turn_probe(session, std::sync::Arc::new(|| false));
    let (state, detail, reason) = confirm_route(session, Duration::from_millis(150)).await;
    assert_eq!(state, "delivered");
    assert_eq!(reason, "unconfirmed");
    assert!(detail.contains("no wake was observed"), "got: {detail}");
}

#[test]
fn status_verdict_unknown_id_is_honest_failure() {
    let input = serde_json::json!({ "notify_id": uuid::Uuid::new_v4().to_string() });
    let result = status_verdict(&input).expect("verdict builds");
    assert!(!result.success);
    assert_eq!(result.metadata.get("notify_state").unwrap(), "unknown_id");
}

#[test]
fn status_verdict_requires_notify_id() {
    let input = serde_json::json!({});
    let err = status_verdict(&input).unwrap_err().to_string();
    assert!(err.contains("'notify_id' is required"), "got: {err}");
}

#[test]
fn status_verdict_tracks_queued_then_injected_lifecycle() {
    use crate::brain::agent::service::notify_receipts;
    let (id, target) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    notify_receipts::record_queued(id, target);

    let input = serde_json::json!({ "notify_id": id.to_string() });
    let queued = status_verdict(&input).expect("verdict builds");
    assert!(queued.success);
    assert_eq!(queued.metadata.get("notify_state").unwrap(), "queued");
    assert_eq!(
        queued.metadata.get("notify_target").unwrap(),
        target.to_string().as_str()
    );
    assert!(!queued.metadata.contains_key("injected_at"));
    assert!(
        queued.output.contains("NOT yet observed"),
        "got: {}",
        queued.output
    );

    assert_eq!(notify_receipts::mark_injected_for_target(target), 1);
    let injected = status_verdict(&input).expect("verdict builds");
    assert_eq!(injected.metadata.get("notify_state").unwrap(), "injected");
    assert!(injected.metadata.contains_key("injected_at"));
    assert!(
        injected.output.contains("INJECTED"),
        "got: {}",
        injected.output
    );
}

#[test]
fn schema_ships_status_verb_and_notify_id_param() {
    let tool = SessionNotifyTool;
    let schema = tool.input_schema();
    assert_eq!(
        schema["properties"]["action"]["enum"],
        serde_json::json!(["send", "status"])
    );
    assert!(schema["properties"]["notify_id"].is_object());
    // 'message' moved off required so status calls validate; the send
    // path still refuses a missing/empty message at execute time.
    assert_eq!(schema["required"], serde_json::json!(["target_session"]));
}
