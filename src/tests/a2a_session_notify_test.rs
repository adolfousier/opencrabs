//! `session/notify` JSON-RPC handler tests (#23) — the tooling-facing twin
//! of `session_notify_test` (which covers the agent-tool surface).
//!
//! What matters here: the zombie-wake existence gate (a dead uuid yields
//! `no_route` without touching the route table), live-route delivery
//! through the same `deliver_to_session` path the agent tool uses, the
//! sender-label override riding the `cli:` header, and that malformed
//! params are protocol errors while business outcomes are JSON-RPC
//! successes. (The archived-session auto-route case rides with the #19
//! channel-ownership harvest.)

use crate::a2a::handler::notify::{
    CLI_SENDER_PREFIX, DEFAULT_CLI_SENDER_LABEL, handle_session_notify,
};
use crate::a2a::test_helpers::helpers::placeholder_service_context;
use crate::a2a::types::{JsonRpcResponse, error_codes};
use crate::brain::agent::QueuedUserMessage;
use crate::brain::agent::service::restart_recovery::test_guard;
use crate::brain::agent::service::session_routes::register_session_route;
use crate::services::SessionService;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn params(session_id: &str, message: &str) -> serde_json::Value {
    serde_json::json!({ "session_id": session_id, "message": message })
}

fn outcome_of(resp: &JsonRpcResponse) -> String {
    resp.result
        .as_ref()
        .expect("success response")
        .get("outcome")
        .and_then(serde_json::Value::as_str)
        .expect("outcome field")
        .to_string()
}

#[tokio::test]
// test_guard serializes suites touching the process-global route table;
// holding it across the goal-dispatch `.await`s below is the entire point —
// the registered route must not interleave with another test's
// (session_notify_test precedent).
#[allow(clippy::await_holding_lock)]
async fn a2a_notify_dispatches_goal_to_target_session() {
    let ctx = placeholder_service_context().await;
    let svc = SessionService::new(ctx.clone());
    let session = svc
        .create_session(Some("goal-target".to_string()))
        .await
        .expect("session creates");

    let _g = test_guard();
    let sink = Arc::new(Mutex::new(Vec::new()));
    let sink_clone = sink.clone();
    register_session_route(
        session.id,
        Arc::new(move |_session_id: Uuid, msg: QueuedUserMessage| {
            sink_clone.lock().unwrap().push(msg);
        }),
    );

    let mut p = params(&session.id.to_string(), "wake with goal");
    p["goal"] = serde_json::json!("converge on clean audit");
    p["goal_max_turns"] = serde_json::json!(5);

    let resp = handle_session_notify(serde_json::json!(42), p, ctx.clone()).await;
    assert!(resp.error.is_none());
    assert_eq!(outcome_of(&resp), "delivered");

    let goal_mgr = crate::brain::goal::GoalManager::new(ctx);
    let state = goal_mgr
        .get_goal(session.id)
        .await
        .expect("goal query succeeds")
        .expect("goal exists");
    assert_eq!(state.goal_text, "converge on clean audit");
    assert_eq!(state.max_turns, 5);
}

#[tokio::test]
async fn dead_uuid_is_refused_without_touching_the_route_table() {
    // #23 acceptance: unknown uuid → no_route, nothing created. The DB
    // is empty, so the zombie-wake guard must fire BEFORE
    // deliver_to_session — no route is registered, and if the guard were
    // skipped the local-route fallback would still yield a non-no_route
    // outcome only when LOCAL_ROUTE is set; the DB gate makes the result
    // deterministic either way.
    let ctx = placeholder_service_context().await;
    let dead = uuid::Uuid::new_v4();
    let resp =
        handle_session_notify(serde_json::json!(1), params(&dead.to_string(), "ping"), ctx).await;
    assert!(
        resp.error.is_none(),
        "dead uuid is a business outcome, not a protocol error: {resp:?}"
    );
    assert_eq!(outcome_of(&resp), "no_route");
}

#[tokio::test]
// test_guard serializes suites touching the process-global route table;
// holding it across the delivery `.await`s below is the entire point —
// this suite's registered route must not interleave with another test's
// (session_notify_test precedent).
#[allow(clippy::await_holding_lock)]
async fn live_uuid_delivers_through_the_claimed_route() {
    // #23 acceptance: live uuid → delivered via the same
    // deliver_to_session path the agent tool uses.
    let _guard = test_guard();
    let ctx = placeholder_service_context().await;
    let session = SessionService::new(ctx.clone())
        .create_session(Some("#23 test session".to_string()))
        .await
        .expect("session row created");
    let sid = session.id;

    let captured: Arc<Mutex<Option<QueuedUserMessage>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    register_session_route(
        sid,
        Arc::new(move |_id, queued| {
            *sink.lock().unwrap() = Some(queued);
        }),
    );

    let resp =
        handle_session_notify(serde_json::json!(2), params(&sid.to_string(), "ping"), ctx).await;
    assert!(resp.error.is_none(), "{resp:?}");
    assert_eq!(outcome_of(&resp), "delivered");
    let queued = captured.lock().unwrap().take().expect("message enqueued");
    assert_eq!(
        queued.origin,
        crate::brain::agent::PushOrigin::SessionNotify
    );
    assert!(queued.context_text.contains(&format!(
        "[session-notify from={CLI_SENDER_PREFIX}{DEFAULT_CLI_SENDER_LABEL}]"
    )));
}

#[tokio::test]
// test_guard: same serialization rationale as the suite above — the
// registered route must survive the delivery `.await`s untouched.
#[allow(clippy::await_holding_lock)]
async fn sender_override_rides_the_header() {
    // #23 owner amendment ("Overridable"): the sender label is overridable
    // via the `sender` param (CLI: `--sender`), and the echo surface reads
    // it off the cli:-prefixed header verbatim.
    let _guard = test_guard();
    let ctx = placeholder_service_context().await;
    let session = SessionService::new(ctx.clone())
        .create_session(Some("#23 sender override".to_string()))
        .await
        .expect("session row created");
    let sid = session.id;

    let captured: Arc<Mutex<Option<QueuedUserMessage>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    register_session_route(
        sid,
        Arc::new(move |_id, queued| {
            *sink.lock().unwrap() = Some(queued);
        }),
    );

    let mut p = params(&sid.to_string(), "ping");
    p["sender"] = serde_json::json!("oc-deploy");
    let resp = handle_session_notify(serde_json::json!(7), p, ctx).await;
    assert!(resp.error.is_none(), "{resp:?}");
    assert_eq!(outcome_of(&resp), "delivered");
    let queued = captured.lock().unwrap().take().expect("message enqueued");
    assert!(
        queued
            .context_text
            .contains("[session-notify from=cli:oc-deploy]"),
        "override must ride the cli:-prefixed header: {}",
        queued.context_text
    );
    assert!(
        queued.display_text.contains("from oc-deploy"),
        "display frame names the overridden sender: {}",
        queued.display_text
    );
}

#[tokio::test]
async fn malformed_params_are_protocol_errors() {
    let ctx = placeholder_service_context().await;
    let bad_uuid = handle_session_notify(
        serde_json::json!(3),
        params("not-a-uuid", "ping"),
        ctx.clone(),
    )
    .await;
    assert_eq!(
        bad_uuid.error.expect("error response").code,
        error_codes::INVALID_PARAMS
    );

    let empty_msg = handle_session_notify(
        serde_json::json!(4),
        params(&uuid::Uuid::new_v4().to_string(), "   "),
        ctx.clone(),
    )
    .await;
    assert_eq!(
        empty_msg.error.expect("error response").code,
        error_codes::INVALID_PARAMS
    );

    // A sender label that would break the `[session-notify from=cli:<label>]`
    // framing is a protocol error, not a delivery result.
    let mut bad_sender = params(&uuid::Uuid::new_v4().to_string(), "ping");
    bad_sender["sender"] = serde_json::json!("bad]label");
    let bad_sender_resp = handle_session_notify(serde_json::json!(5), bad_sender, ctx).await;
    assert_eq!(
        bad_sender_resp.error.expect("error response").code,
        error_codes::INVALID_PARAMS
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn delivery_mode_turn_end_without_interrupt_resolves_cleanly() {
    // Fork #158 regression: CLI with `--mode turn-end` sends delivery.mode="turn-end"
    // without an interrupt key. This must resolve cleanly (DeliveryMode::TurnEnd)
    // rather than failing with INVALID_PARAMS (-32602) from disagreement.
    let _guard = test_guard();
    let ctx = placeholder_service_context().await;
    let session = SessionService::new(ctx.clone())
        .create_session(Some("#158 turn-end test session".to_string()))
        .await
        .expect("session row created");
    let sid = session.id;

    let captured: Arc<Mutex<Option<QueuedUserMessage>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    register_session_route(
        sid,
        Arc::new(move |_id, queued| {
            *sink.lock().unwrap() = Some(queued);
        }),
    );

    // (1) With `interrupt: false` alongside `delivery.mode: "turn-end"`: fails with -32602 (the defect)
    let mut bad_p = params(&sid.to_string(), "turn-end conflicting ping");
    bad_p["delivery"] = serde_json::json!({ "mode": "turn-end" });
    bad_p["interrupt"] = serde_json::json!(false);
    let bad_resp = handle_session_notify(serde_json::json!(6), bad_p, ctx.clone()).await;
    assert_eq!(
        bad_resp.error.expect("error response").code,
        error_codes::INVALID_PARAMS,
        "explicit interrupt:false alongside mode:turn-end must be rejected by policy"
    );

    // (2) With `interrupt` omitted (the fix): succeeds cleanly
    let mut good_p = params(&sid.to_string(), "turn-end ping");
    good_p["delivery"] = serde_json::json!({ "mode": "turn-end" });
    let resp = handle_session_notify(serde_json::json!(7), good_p, ctx).await;
    assert!(
        resp.error.is_none(),
        "delivery.mode turn-end without interrupt must succeed: {resp:?}"
    );
    assert_eq!(outcome_of(&resp), "delivered");
    let queued = captured.lock().unwrap().take().expect("message enqueued");
    assert_eq!(
        queued.origin,
        crate::brain::agent::PushOrigin::SessionNotify
    );
}
