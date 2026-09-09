//! session/notify handler (#23): delivery through registered session routes, channel ownership, and the CLI sender prefix.

use crate::a2a::handler::notify::*;
use crate::a2a::test_helpers::helpers::placeholder_service_context;
use crate::a2a::types::*;
use crate::brain::agent::service::restart_recovery::test_guard;
use crate::brain::agent::service::session_routes::{ChannelOwnership, register_session_route};
use crate::brain::agent::{PushOrigin, QueuedUserMessage};
use crate::services::SessionService;
use std::sync::{Arc, Mutex};

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
// (#22 shape, session_notify_test precedent).
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
    assert_eq!(queued.origin, PushOrigin::SessionNotify);
    assert!(queued.context_text.contains(&format!(
        "[session-notify from={CLI_SENDER_PREFIX}{DEFAULT_CLI_SENDER_LABEL}]"
    )));
}

#[tokio::test]
// test_guard: same serialization rationale as the suite above — the
// registered route must survive the delivery `.await`s untouched.
#[allow(clippy::await_holding_lock)]
async fn sender_override_rides_the_header() {
    // #23 owner amendment ("Overridable"): the sender label is
    // overridable via the `sender` param (CLI: `--sender`), and the
    // echo surface reads it off the cli:-prefixed header verbatim.
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
// test_guard: this suite touches the route table AND the channel-owner
// registry across `.await`s (create → archive → occupy → notify); the
// guard keeps the whole sequence atomic against other suites.
#[allow(clippy::await_holding_lock)]
async fn archived_session_auto_routes_to_its_successor() {
    // Owner directive 2026-08-28: archived ≠ dead. An archived session
    // whose channel a successor occupies must auto-route exactly like
    // any session_notify — the #19 redirect carries the notification to
    // the occupant with provenance framing, never a no_route refusal.
    let _guard = test_guard();
    let ctx = placeholder_service_context().await;
    let svc = SessionService::new(ctx.clone());
    let old = svc
        .create_session(Some("#23 old session".to_string()))
        .await
        .expect("old session row");
    svc.archive_session(old.id).await.expect("archived");
    let successor = svc
        .create_session(Some("#23 successor session".to_string()))
        .await
        .expect("successor session row");

    // The old session's channel is now occupied by the successor, and the
    // successor has a live route — the exact replaced-session shape.
    let occupant = successor.id;
    crate::brain::agent::service::session_routes::register_channel_owner_probe(
        old.id,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant }),
    );
    let captured: Arc<Mutex<Option<QueuedUserMessage>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    register_session_route(
        successor.id,
        Arc::new(move |_id, queued| {
            *sink.lock().unwrap() = Some(queued);
        }),
    );

    let resp = handle_session_notify(
        serde_json::json!(5),
        params(&old.id.to_string(), "ping"),
        ctx,
    )
    .await;
    assert!(resp.error.is_none(), "{resp:?}");
    assert_eq!(outcome_of(&resp), "delivered");
    let detail = resp
        .result
        .unwrap()
        .get("detail")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        detail.contains("redirected"),
        "detail should name the redirect: {detail}"
    );
    let queued = captured
        .lock()
        .unwrap()
        .take()
        .expect("successor received the redirect");
    assert!(
        queued
            .context_text
            .contains(&format!("originally for session {}", old.id)),
        "provenance framing must name the archived session: {}",
        queued.context_text
    );
}

#[tokio::test]
// test_guard: route-table suite — same serialization rationale as above.
#[allow(clippy::await_holding_lock)]
async fn quiet_mode_banks_the_notice_and_returns_the_id() {
    // fork #146 acceptance: the A2A surface carries the full v2 policy —
    // `delivery.mode=quiet` banks via the quiet engine and returns a
    // notification id, the same contract the agent tool honors.
    let _guard = test_guard();
    let ctx = placeholder_service_context().await;
    let session = SessionService::new(ctx.clone())
        .create_session(Some("#146 quiet test".to_string()))
        .await
        .expect("session row created");
    let sid = session.id;

    let mut p = params(&sid.to_string(), "ping");
    p["delivery"] = serde_json::json!({ "mode": "quiet", "quiet_for_secs": 3600 });
    let resp = handle_session_notify(serde_json::json!(11), p, ctx).await;
    assert!(resp.error.is_none(), "{resp:?}");
    let result = resp.result.expect("success");
    assert_eq!(
        result.get("outcome").and_then(|v| v.as_str()),
        Some("deferred")
    );
    let notify_id = result
        .get("notify_id")
        .and_then(|v| v.as_str())
        .expect("quiet verdict carries the notification id");
    // The deferred id is status-checkable from birth: queued until the
    // quiet release drains it.
    let status = crate::a2a::handler::notify::handle_notify_status(
        serde_json::json!(12),
        serde_json::json!({ "notify_id": notify_id }),
    );
    let status_body = status.result.expect("status success");
    assert_eq!(
        status_body.get("notify_state").and_then(|v| v.as_str()),
        Some("queued")
    );
}

#[tokio::test]
async fn turn_end_mode_queues_instead_of_refusing() {
    // fork #146: `delivery.mode=turn-end` (the deprecated interrupt=true)
    // reaches the same policy through A2A — a mid-turn session QUEUES the
    // message at its next boundary instead of refusing it.
    let ctx = placeholder_service_context().await;
    let session = SessionService::new(ctx.clone())
        .create_session(Some("#146 turn-end test".to_string()))
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

    let _guard = test_guard();

    let mut p = params(&sid.to_string(), "ping");
    p["delivery"] = serde_json::json!({ "mode": "turn-end" });
    let resp = handle_session_notify(serde_json::json!(13), p, ctx).await;
    assert!(resp.error.is_none(), "{resp:?}");
    assert_eq!(outcome_of(&resp), "delivered");
    assert!(captured.lock().unwrap().take().is_some());
}

#[tokio::test]
async fn quiet_contradicting_interrupt_is_invalid_params() {
    // The shared resolve_mode agreement rule fires through A2A too.
    let ctx = placeholder_service_context().await;
    let mut p = params(&uuid::Uuid::new_v4().to_string(), "ping");
    p["delivery"] = serde_json::json!({ "mode": "quiet" });
    p["interrupt"] = serde_json::json!(true);
    let resp = handle_session_notify(serde_json::json!(14), p, ctx).await;
    assert_eq!(
        resp.error.expect("error response").code,
        error_codes::INVALID_PARAMS
    );
}

#[tokio::test]
async fn notify_status_reports_unknown_id_honestly() {
    // fork #146: the A2A status verb exists; an untracked id reports
    // unknown_id (in-memory receipts die with the process).
    let resp = crate::a2a::handler::notify::handle_notify_status(
        serde_json::json!(15),
        serde_json::json!({ "notify_id": uuid::Uuid::new_v4().to_string() }),
    );
    let body = resp.result.expect("business outcome is a success");
    assert_eq!(
        body.get("notify_state").and_then(|v| v.as_str()),
        Some("unknown_id")
    );
}

#[tokio::test]
async fn notify_status_requires_the_id() {
    let resp = crate::a2a::handler::notify::handle_notify_status(
        serde_json::json!(16),
        serde_json::json!({}),
    );
    assert_eq!(
        resp.error.expect("error response").code,
        error_codes::INVALID_PARAMS
    );
}
