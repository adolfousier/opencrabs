//! `session_notify` surface and delivery reporting (PR #1207, issue #1203).
//!
//! The tool is a thin wrapper over `deliver_to_session`, so what matters here
//! is its contract: the schema it advertises, and that it reports a PARKED
//! delivery as queued rather than as a missing route. Parking arrived with
//! #1206, after this tool was written against a two-state bool.

use uuid::Uuid;

use crate::brain::agent::QueuedUserMessage;
use crate::brain::agent::service::restart_recovery::{expect_channel_route, test_guard};
use crate::brain::agent::service::session_routes::{
    ChannelOwnership, Delivery, deliver_to_session, register_channel_owner_probe,
    register_session_route, register_turn_probe,
};
use crate::brain::tools::subagent::SessionNotifyTool;
use crate::brain::tools::r#trait::{Tool, ToolExecutionContext};
use crate::db::{Database, NotifyQueueRepository, SessionRepository};
use crate::db::models::Session;
use crate::services::ServiceContext;

fn msg() -> QueuedUserMessage {
    QueuedUserMessage {
        context_text: "[session-notify from=x]\n\nbody".to_string(),
        display_text: "notify".to_string(),
        origin: crate::brain::agent::PushOrigin::Other,
        bg_meta: None,
    }
}

#[tokio::test]
#[expect(clippy::await_holding_lock)]
async fn absent_session_fails_loudly_without_queue_residue() {
    let _guard = test_guard();
    let db = Database::connect_in_memory().await.expect("in-memory DB");
    db.run_migrations().await.expect("migrations");
    let absent = Uuid::new_v4();
    let mut context = ToolExecutionContext::new(Uuid::new_v4());
    context.service_context = Some(ServiceContext::new(db.pool().clone()));

    let result = SessionNotifyTool
        .execute(
            serde_json::json!({"target_session": absent.to_string(), "message": "probe"}),
            &context,
        )
        .await
        .expect("tool returns a verdict");

    assert!(!result.success, "absent session must fail loudly: {result:?}");
    assert_eq!(
        result.metadata.get("notify_state").map(String::as_str),
        Some("undeliverable")
    );
    assert_eq!(
        result.metadata.get("notify_reason").map(String::as_str),
        Some("no_such_session")
    );
    // A failing verdict carries its text in `error`, not `output`:
    // `ToolResult::error` deliberately leaves `output` empty, and
    // `build_tool_result_content` renders `error` to the caller
    // (tool_loop.rs). Asserting on `output` here would pass on a verdict
    // that told the model nothing — which is the defect this test exists to
    // catch, so read the field the caller actually reads.
    let detail = result.error.as_deref().unwrap_or_default();
    assert!(detail.contains("a2a_send"), "got: {detail:?}");
    assert!(
        NotifyQueueRepository::new(db.pool().clone())
            .all()
            .await
            .expect("queue query")
            .is_empty(),
        "a rejected target must never leave durable queue residue"
    );
}

#[tokio::test]
#[expect(clippy::await_holding_lock)]
async fn existing_unbound_session_reports_unclaimed_no_binding() {
    let _guard = test_guard();
    let db = Database::connect_in_memory().await.expect("in-memory DB");
    db.run_migrations().await.expect("migrations");
    let target = Session::new(Some("headless target".into()), None, None);
    SessionRepository::new(db.pool().clone())
        .create(&target)
        .await
        .expect("seed session");
    crate::brain::agent::service::restart_recovery::expect_channel_route(target.id);

    let mut context = ToolExecutionContext::new(Uuid::new_v4());
    context.service_context = Some(ServiceContext::new(db.pool().clone()));
    let result = SessionNotifyTool
        .execute(
            serde_json::json!({"target_session": target.id.to_string(), "message": "probe"}),
            &context,
        )
        .await
        .expect("tool returns a verdict");

    assert!(result.success, "an unbound real session still parks: {result:?}");
    assert_eq!(
        result.metadata.get("notify_reason").map(String::as_str),
        Some("unclaimed_no_binding")
    );
    assert!(result.output.contains("No surface has ever claimed"));
}

#[tokio::test]
// The guard serializes suites touching the process-global parked-queue state
// (#1206); holding it across the tool `.await` below is the entire point —
// the awaited region must not interleave with another test's park.
#[allow(clippy::await_holding_lock)]
async fn test_notify_pushes_carry_sessionnotify_origin_for_topic_echo() {
    // #1221 notify lane: the Telegram resume callback echoes only origins it
    // knows about, so the tool must tag its pushes SessionNotify — the silent
    // Other default keeps every session_notify push invisible in topics.
    let _guard = test_guard();
    let session = Uuid::new_v4();
    let captured: std::sync::Arc<std::sync::Mutex<Option<QueuedUserMessage>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let sink = captured.clone();
    crate::brain::agent::service::session_routes::register_session_route(
        session,
        std::sync::Arc::new(move |_id, queued| {
            *sink.lock().unwrap() = Some(queued);
        }),
    );

    let context = crate::brain::tools::r#trait::ToolExecutionContext::new(Uuid::new_v4());
    let outcome = SessionNotifyTool
        .execute(
            serde_json::json!({"target_session": session.to_string(), "message": "ping"}),
            &context,
        )
        .await;
    assert!(outcome.is_ok(), "delivery should succeed: {outcome:?}");
    let queued = captured.lock().unwrap().take().expect("message enqueued");
    assert_eq!(
        queued.origin,
        crate::brain::agent::PushOrigin::SessionNotify,
        "#1221: Other-tagged notify pushes never earn an echo bubble"
    );
}

#[test]
fn test_schema_requires_target_and_message() {
    let tool = SessionNotifyTool;
    assert_eq!(tool.name(), "session_notify");
    assert!(!tool.requires_approval());
    assert!(!tool.description().is_empty());

    let schema = tool.input_schema();
    let required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required list")
        .iter()
        .map(|v| v.as_str().expect("required entry is a string"))
        .collect();
    // v2 (#1273): only target_session is required — `message` became optional
    // with the action verb family (action=status polls a receipt, sends no text).
    // Mirrors the in-file schema test in subagent/notify.rs.
    assert_eq!(required, vec!["target_session"]);
    assert!(schema["properties"]["target_session"].is_object());
    assert!(schema["properties"]["message"].is_object());
}

#[test]
fn test_a_parked_delivery_is_not_a_missing_route() {
    // The distinction the tool reports on: a session whose channel has not
    // claimed it since a restart holds the message rather than losing it.
    let _guard = test_guard();
    let session = Uuid::new_v4();

    expect_channel_route(session);

    let outcome = deliver_to_session(session, msg(), false);
    assert_eq!(
        outcome,
        Delivery::Parked,
        "#1206: a park is queued, not lost — reporting it as a missing route \
         tells the caller the opposite of what happened"
    );
}

#[test]
fn test_an_unroutable_session_is_reported_as_such() {
    let _guard = test_guard();
    // No local route is registered in tests, so nothing can take it.
    assert_eq!(
        deliver_to_session(Uuid::new_v4(), msg(), false),
        Delivery::NoRoute
    );
}

// ── In-flight gate (fork #13) ────────────────────────────────────────────

#[test]
fn test_inflight_target_refuses_without_interrupt() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    expect_channel_route(session);
    register_turn_probe(session, std::sync::Arc::new(|| true));
    assert_eq!(
        deliver_to_session(session, msg(), false),
        Delivery::RefusedInFlight {
            redirected_to: None
        },
        "#13: default-false must refuse a mid-turn target, not derail it"
    );
}

#[test]
fn test_interrupt_true_delivers_to_inflight_target() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    expect_channel_route(session);
    register_turn_probe(session, std::sync::Arc::new(|| true));
    // Parked, not delivered: the channel route is expected but unclaimed, so
    // the point here is only that the gate let the message THROUGH.
    assert_eq!(
        deliver_to_session(session, msg(), true),
        Delivery::Parked,
        "#13: interrupt=true rides today's queue-for-boundary semantics"
    );
}

#[test]
fn test_idle_target_delivers_without_interrupt() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    expect_channel_route(session);
    register_turn_probe(session, std::sync::Arc::new(|| false));
    assert_eq!(
        deliver_to_session(session, msg(), false),
        Delivery::Parked,
        "#13: idle target — the gate never engages"
    );
}

#[test]
fn test_no_probe_fails_open() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    expect_channel_route(session);
    // No probe registered: a surface without turn state must stay notifyable.
    assert_eq!(
        deliver_to_session(session, msg(), false),
        Delivery::Parked,
        "#13: unknown turn state fails open — never refuse on missing semantics"
    );
}

#[tokio::test]
#[expect(clippy::await_holding_lock)]
async fn test_tool_reports_refusal_with_remedy() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    expect_channel_route(session);
    register_turn_probe(session, std::sync::Arc::new(|| true));

    let context = crate::brain::tools::r#trait::ToolExecutionContext::new(Uuid::new_v4());
    let outcome = SessionNotifyTool
        .execute(
            serde_json::json!({"target_session": session.to_string(), "message": "ping"}),
            &context,
        )
        .await;
    let result = outcome.expect("tool executes");
    assert!(
        !result.success,
        "#13: refusal must read as failure to the sender"
    );
    let error = result.error.expect("#13: refusal carries an explanation");
    assert!(
        error.contains("interrupt=true"),
        "#13: the error must name the remedy so the sender learns the knob: {error}"
    );
}

#[tokio::test]
#[expect(clippy::await_holding_lock)]
async fn test_tool_interrupt_param_reaches_delivery() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    let captured: std::sync::Arc<std::sync::Mutex<Option<QueuedUserMessage>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let sink = captured.clone();
    crate::brain::agent::service::session_routes::register_session_route(
        session,
        std::sync::Arc::new(move |_id, queued| {
            *sink.lock().unwrap() = Some(queued);
        }),
    );
    register_turn_probe(session, std::sync::Arc::new(|| true));

    let context = crate::brain::tools::r#trait::ToolExecutionContext::new(Uuid::new_v4());
    let outcome = SessionNotifyTool
        .execute(
            serde_json::json!({
                "target_session": session.to_string(),
                "message": "ping",
                "interrupt": true
            }),
            &context,
        )
        .await;
    assert!(
        outcome.expect("tool executes").success,
        "interrupt=true must deliver"
    );
    let queued = captured.lock().unwrap().take().expect("message enqueued");
    assert!(
        !queued
            .context_text
            .contains("queued while you were working"),
        "framing is added by the channel queue branch, not by the tool"
    );
}

// ── Channel-ownership gate (fork #17) + redirect (fork #19) ─────────────
//
// A session REPLACED on its chat/topic (idle-timeout reset creates a
// successor) keeps its delivery route — routes are UUID-keyed and never
// evicted. Without the gate any push wakes the replaced session into the
// successor's conversation. The gate is the OUTERMOST check: it guards who
// owns the channel, not the target's turn state, so `interrupt` does not
// override it. Unknown ownership fails open, same posture as the turn gate.
//
// Occupied no longer REFUSES (#19): the message is REDIRECTED to the
// occupant — the session that owns the channel now — with a provenance
// frame on its context text, up to a 3-hop cap (cycle insurance), then
// parked. The original `interrupt` flag is honored against the FINAL
// target after redirecting.

#[test]
fn test_occupied_channel_redirects_delivery_to_occupant() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    let occupant = Uuid::new_v4();
    let captured: std::sync::Arc<std::sync::Mutex<Option<QueuedUserMessage>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let sink = captured.clone();
    register_channel_owner_probe(
        session,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant }),
    );
    register_session_route(
        occupant,
        std::sync::Arc::new(move |_id, queued| {
            *sink.lock().unwrap() = Some(queued);
        }),
    );
    let outcome = deliver_to_session(session, msg(), false);
    assert_eq!(
        outcome,
        Delivery::Redirected { to: occupant },
        "#19: an occupied channel redirects to the occupant — delivered, never refused"
    );
    let queued = captured
        .lock()
        .unwrap()
        .take()
        .expect("occupant got the message");
    let frame = format!(
        "[redirected — originally for session {session}, which no longer owns this channel]"
    );
    assert!(
        queued.context_text.starts_with(&frame),
        "#19: the successor must see the provenance frame: {}",
        queued.context_text
    );
}

#[test]
fn test_occupied_channel_redirects_even_with_interrupt() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    let occupant = Uuid::new_v4();
    register_channel_owner_probe(
        session,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant }),
    );
    register_session_route(occupant, std::sync::Arc::new(|_id, _queued| {}));
    assert_eq!(
        deliver_to_session(session, msg(), true),
        Delivery::Redirected { to: occupant },
        "#19: interrupt overrides the TURN gate only — never channel ownership; \
         the redirect still happens"
    );
}

#[test]
fn test_ownership_gate_outranks_turn_gate() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    let occupant = Uuid::new_v4();
    register_turn_probe(session, std::sync::Arc::new(|| true));
    register_channel_owner_probe(
        session,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant }),
    );
    register_session_route(occupant, std::sync::Arc::new(|_id, _queued| {}));
    // Mid-turn AND replaced: the redirect wins — the message goes to the
    // occupant, whose own turn state (not the dead session's) gates it.
    assert_eq!(
        deliver_to_session(session, msg(), false),
        Delivery::Redirected { to: occupant },
        "#19: the ownership gate runs outside the turn gate"
    );
}

#[test]
fn test_redirect_honors_interrupt_against_occupant() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    let occupant = Uuid::new_v4();
    register_channel_owner_probe(
        session,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant }),
    );
    // The FINAL target (the occupant) is mid-turn: the sender's original
    // `interrupt` flag gates IT, not the replaced session.
    register_turn_probe(occupant, std::sync::Arc::new(|| true));
    register_session_route(occupant, std::sync::Arc::new(|_id, _queued| {}));
    assert_eq!(
        deliver_to_session(session, msg(), false),
        Delivery::RefusedInFlight {
            redirected_to: Some(occupant)
        },
        "#19: a redirect landing on a mid-turn occupant is refused with the \
         redirect context — the sender hears where the message WOULD have gone"
    );
    assert_eq!(
        deliver_to_session(session, msg(), true),
        Delivery::Redirected { to: occupant },
        "#19: interrupt=true still delivers through the redirect"
    );
}

#[test]
fn test_redirect_hop_cap_parks_beyond_three() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    let occ1 = Uuid::new_v4();
    let occ2 = Uuid::new_v4();
    let occ3 = Uuid::new_v4();
    let occ4 = Uuid::new_v4();
    // A replacing chain: each session's channel is occupied by the next.
    register_channel_owner_probe(
        session,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant: occ1 }),
    );
    register_channel_owner_probe(
        occ1,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant: occ2 }),
    );
    register_channel_owner_probe(
        occ2,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant: occ3 }),
    );
    register_channel_owner_probe(
        occ3,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant: occ4 }),
    );
    assert_eq!(
        deliver_to_session(session, msg(), false),
        Delivery::Parked,
        "#19: hop cap 3 is cycle insurance — past it the message is parked, \
         never looped and never dropped"
    );
}

#[test]
fn test_owned_channel_passes_ownership_gate() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    expect_channel_route(session);
    register_channel_owner_probe(session, std::sync::Arc::new(|| ChannelOwnership::Owned));
    assert_eq!(
        deliver_to_session(session, msg(), false),
        Delivery::Parked,
        "#17: an owning session passes the gate — parked here only because \
         the expected route is unclaimed"
    );
}

#[test]
fn test_unknown_ownership_fails_open() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    expect_channel_route(session);
    register_channel_owner_probe(session, std::sync::Arc::new(|| ChannelOwnership::Unknown));
    assert_eq!(
        deliver_to_session(session, msg(), false),
        Delivery::Parked,
        "#17: unknown ownership fails open — never refuse on missing semantics"
    );
}

#[tokio::test]
#[expect(clippy::await_holding_lock)]
async fn test_tool_reports_redirect_to_occupant() {
    let _guard = test_guard();
    let session = Uuid::new_v4();
    let occupant = Uuid::new_v4();
    register_channel_owner_probe(
        session,
        std::sync::Arc::new(move || ChannelOwnership::Occupied { occupant }),
    );
    register_session_route(occupant, std::sync::Arc::new(|_id, _queued| {}));

    let context = crate::brain::tools::r#trait::ToolExecutionContext::new(Uuid::new_v4());
    let outcome = SessionNotifyTool
        .execute(
            serde_json::json!({"target_session": session.to_string(), "message": "ping"}),
            &context,
        )
        .await;
    let result = outcome.expect("tool executes");
    assert!(result.success, "#19: a redirect is delivery, not failure");
    let message = result.output;
    assert!(!message.is_empty(), "#19: the outcome names where it went");
    assert!(
        message.contains(&occupant.to_string()),
        "#19: must name the session that owns the channel now: {message}"
    );
    assert!(
        message.contains("Redirected"),
        "#19: must say it was redirected: {message}"
    );
}

#[tokio::test]
async fn test_ownership_mirror_tracks_channel_replacement() {
    // The sync mirror behind the telegram probe (state.rs): binding a
    // successor to the same (chat, topic) must flip the old session to
    // Occupied naming the successor, while the successor reads Owned and a
    // never-bound session reads Unknown.
    use crate::channels::telegram::TelegramState;
    let state = TelegramState::new();
    let old = Uuid::new_v4();
    let successor = Uuid::new_v4();
    let (chat, topic) = (-100_123_456_i64, Some(42_i32));

    assert_eq!(
        state.channel_ownership_of(old),
        ChannelOwnership::Unknown,
        "#17: never-bound session — no binding recorded"
    );

    state.register_session_chat(old, chat, topic).await;
    assert_eq!(state.channel_ownership_of(old), ChannelOwnership::Owned);

    // Idle-timeout replacement: same channel, new session.
    state.register_session_chat(successor, chat, topic).await;
    assert_eq!(
        state.channel_ownership_of(old),
        ChannelOwnership::Occupied {
            occupant: successor
        },
        "#17: the replaced session must see its successor by name"
    );
    assert_eq!(
        state.channel_ownership_of(successor),
        ChannelOwnership::Owned,
        "#17: the successor owns the channel it just bound"
    );
}

#[tokio::test]
async fn test_ownership_mirror_keys_dm_and_general_buckets_separately() {
    // (chat, None) buckets — DMs / non-forum groups — are channels too: a
    // replacement there must occupy exactly that bucket and no other.
    use crate::channels::telegram::TelegramState;
    let state = TelegramState::new();
    let old = Uuid::new_v4();
    let successor = Uuid::new_v4();
    let chat = 777_i64;

    state.register_session_chat(old, chat, None).await;
    state.register_session_chat(successor, chat, None).await;
    assert_eq!(
        state.channel_ownership_of(old),
        ChannelOwnership::Occupied {
            occupant: successor
        }
    );
}
