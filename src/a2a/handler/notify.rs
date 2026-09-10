//! `session/notify` — mechanical session notifications for tooling (#23).
//!
//! Thin JSON-RPC wrapper over `session_routes::deliver_to_session`, the SAME
//! route the agent's `session_notify` tool uses, so external tooling (the
//! `opencrabs session notify` CLI verb and, on top of it, the oc-deploy
//! post-CI fan-out — #24) can post into a live session's queue.
//!
//! Zombie-wake guard (#17 class): the target must exist as a row in the
//! sessions table before the route table is touched at all. An unknown or
//! dead uuid (NO row — never one that merely changed state) yields
//! `no_route` without ever reaching `deliver_to_session`, whose local-route
//! fallback would otherwise inject the message into this process's own boot
//! channel — traffic resurrected for a session that is gone.
//!
//! ARCHIVED sessions are NOT dead (owner directive 2026-08-28): they pass
//! the gate like any live session and auto-route exactly as everywhere else
//! — an archived session whose channel a successor now occupies is
//! REDIRECTED to the occupant by the #19 machinery, with provenance
//! framing. Existence-gate only, never an activity-gate.
//!
//! SENDER FRAMING (owner amendment 2026-08-28, "Overridable"): the CLI lane
//! has no sender session (it is a separate process), so instead of the
//! agent tool's `from=<uuid>` the header stamps `from=cli:<label>` —
//! default [`DEFAULT_CLI_SENDER_LABEL`], overridable via the `sender`
//! param (CLI: `--sender`). The telegram echo surface renders the label
//! verbatim; the recipient's model still reads the mechanical frame.

use crate::a2a::types::*;
use crate::brain::agent::service::notify_policy::{
    CONFIRM_CAP, DeliveryMode, confirm_route, resolve_mode, validate_sender_label,
};
use crate::brain::agent::service::notify_receipts;
use crate::brain::agent::service::quiet_delivery;
use crate::brain::agent::service::session_routes::Delivery;
use crate::brain::agent::service::session_routes::deliver_to_session;
use crate::brain::agent::{PushOrigin, QueuedUserMessage};
use crate::services::{ServiceContext, SessionService};

/// The CLI lane's prefix inside the mechanical `[session-notify from=…]`
/// header (#23). The CLI verb runs as a separate process with no sender
/// session, so it stamps `cli:<label>` instead of a uuid; the telegram echo
/// surface (`channels::telegram::resume::split_notify_header`) recognizes
/// the prefix and renders the carried label verbatim. Agent-to-agent pushes
/// keep the bare-uuid shape (#1203/#1225).
pub(crate) const CLI_SENDER_PREFIX: &str = "cli:";

/// Default sender label for CLI notifications (#23) — overridable via the
/// `sender` JSON-RPC param / the `--sender` CLI flag (owner amendment
/// 2026-08-28).
pub(crate) const DEFAULT_CLI_SENDER_LABEL: &str = "CLI tooling";

/// Handle a `session/notify` JSON-RPC call (#23).
///
/// Business outcomes are returned as JSON-RPC SUCCESSES carrying
/// `{outcome, detail}` — the caller maps them to exit codes. The only
/// JSON-RPC errors this method emits are protocol-level (malformed params,
/// lookup failure), never delivery results.
pub async fn handle_session_notify(
    req_id: serde_json::Value,
    params: serde_json::Value,
    service_context: ServiceContext,
) -> JsonRpcResponse {
    let session_id = match params.get("session_id").and_then(serde_json::Value::as_str) {
        Some(raw) => match raw.parse::<uuid::Uuid>() {
            Ok(id) => id,
            Err(_) => {
                return JsonRpcResponse::error(
                    req_id,
                    error_codes::INVALID_PARAMS,
                    format!("'session_id' is not a valid UUID: {raw}"),
                );
            }
        },
        None => {
            return JsonRpcResponse::error(
                req_id,
                error_codes::INVALID_PARAMS,
                "'session_id' is required",
            );
        }
    };
    let message = match params.get("message").and_then(serde_json::Value::as_str) {
        Some(m) if !m.trim().is_empty() => m.to_string(),
        _ => {
            return JsonRpcResponse::error(
                req_id,
                error_codes::INVALID_PARAMS,
                "'message' is required and must be non-empty",
            );
        }
    };
    let title = params
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    // Sender label (#23): no sender session exists for the CLI lane, so the
    // label is carried verbatim — default DEFAULT_CLI_SENDER_LABEL,
    // overridable by the caller. Validation lives in the SHARED policy
    // module (fork #146) — the same rules the tool and the CLI verb run;
    // only the empty-label default differs (CLI lane defaults, not errors).
    let sender = match params.get("sender").and_then(serde_json::Value::as_str) {
        Some(raw) => {
            let label = raw.trim();
            if label.is_empty() {
                DEFAULT_CLI_SENDER_LABEL.to_string()
            } else if let Err(e) = validate_sender_label(label) {
                return JsonRpcResponse::error(req_id, error_codes::INVALID_PARAMS, e);
            } else {
                label.to_string()
            }
        }
        None => DEFAULT_CLI_SENDER_LABEL.to_string(),
    };

    // Delivery policy (fork #146): the A2A surface carries the SAME policy
    // ontology as the agent tool — `delivery {mode, quiet_for_secs,
    // max_delay_secs}` with the deprecated `interrupt` alias resolving
    // through the shared `resolve_mode`. Quiet banks the notice and returns
    // its id; every success path records a receipt so `session/notify-status`
    // can poll the injection stamp.
    let mode = match resolve_mode(
        params
            .get("delivery")
            .and_then(|d| d.get("mode"))
            .and_then(serde_json::Value::as_str),
        params.get("interrupt").and_then(serde_json::Value::as_bool),
        params.get("delivery"),
    ) {
        Ok(m) => m,
        Err(e) => {
            return JsonRpcResponse::error(req_id, error_codes::INVALID_PARAMS, e);
        }
    };

    // Zombie-wake guard (#23, #17 class): only a session with a DB row may
    // be notified. `deliver_to_session` is never touched for a uuid with NO
    // row — its local-route fallback would hand the message to this
    // process's own boot channel, resurrecting traffic for a session that no
    // longer exists. An ARCHIVED row passes: archived sessions auto-route
    // like anywhere else (#19 redirect to the successor occupying the
    // channel), so the gate checks existence only, never activity.
    let session_svc = SessionService::new(service_context);
    match session_svc.get_session(session_id).await {
        Ok(Some(_session)) => {}
        Ok(None) => {
            return JsonRpcResponse::success(
                req_id,
                serde_json::json!({
                    "outcome": "no_route",
                    "detail": format!(
                        "session {session_id} does not exist — nothing sent, nothing created"
                    ),
                }),
            );
        }
        Err(e) => {
            return JsonRpcResponse::error(
                req_id,
                error_codes::INTERNAL_ERROR,
                format!("session lookup failed: {e}"),
            );
        }
    }

    // Same message shape as the agent's session_notify tool
    // (tools/subagent/notify.rs): SessionNotify origin so the topic-echo
    // surface renders the push (#1221), and a mechanical sender frame. The
    // CLI lane stamps `cli:<label>` instead of a uuid — there is no sender
    // session; the echo renders the label verbatim.
    let header = match &title {
        Some(t) => format!("📨 {t} (from {sender}):"),
        None => format!("📨 notify from {sender}:"),
    };
    let msg = QueuedUserMessage {
        context_text: format!("[session-notify from={CLI_SENDER_PREFIX}{sender}]\n\n{message}"),
        display_text: format!("{header}\n{message}"),
        origin: PushOrigin::SessionNotify,
        bg_meta: None,
    };

    // Quiet mode (fork #43/#50): bank the notice, return the id — accepted,
    // not yet delivered; the id is the status handle from birth.
    if let DeliveryMode::Quiet {
        quiet_for,
        max_delay,
    } = mode
    {
        let id = quiet_delivery::defer_quiet(session_id, msg, quiet_for, max_delay);
        notify_receipts::record_queued(id, session_id);
        return JsonRpcResponse::success(
            req_id,
            serde_json::json!({
                "outcome": "deferred",
                "detail": format!(
                    "deferred for session {session_id}: delivers once the session has been \
                     quiet for {}s (hard cap {}s) — notification id {id}",
                    quiet_for.as_secs(),
                    max_delay.as_secs()
                ),
                "notify_id": id.to_string(),
                "notify_state": "deferred",
            }),
        );
    }

    let interrupt = matches!(mode, DeliveryMode::TurnEnd);
    let confirm = params
        .get("confirm")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let notify_id = uuid::Uuid::new_v4();

    let (outcome, detail, extra) = match deliver_to_session(session_id, msg, interrupt) {
        Delivery::Delivered => {
            notify_receipts::record_queued(notify_id, session_id);
            if confirm {
                let (state, cdetail, reason) = confirm_route(session_id, CONFIRM_CAP).await;
                (
                    "delivered",
                    cdetail,
                    serde_json::json!({
                        "notify_id": notify_id.to_string(),
                        "notify_state": state,
                        "notify_reason": reason,
                    }),
                )
            } else {
                (
                    "delivered",
                    format!("delivered to session {session_id}"),
                    serde_json::json!({ "notify_id": notify_id.to_string() }),
                )
            }
        }
        Delivery::Redirected { to } => {
            notify_receipts::record_queued(notify_id, to);
            if confirm {
                let (state, cdetail, reason) = confirm_route(to, CONFIRM_CAP).await;
                (
                    "delivered",
                    format!("{cdetail} (redirected to session {to})"),
                    serde_json::json!({
                        "notify_id": notify_id.to_string(),
                        "notify_state": state,
                        "notify_reason": reason,
                        "notify_occupant": to.to_string(),
                    }),
                )
            } else {
                (
                    "delivered",
                    format!(
                        "redirected to session {to}: session {session_id} no longer owns its \
                         channel (#19)"
                    ),
                    serde_json::json!({
                        "notify_id": notify_id.to_string(),
                        "notify_occupant": to.to_string(),
                    }),
                )
            }
        }
        // Queued, not lost: the session's channel has not claimed it since
        // the last restart (#1206). Reporting this as a failure would be the
        // opposite of what happened — same reading as the agent tool.
        Delivery::Parked => {
            notify_receipts::record_queued(notify_id, session_id);
            (
                "parked",
                format!(
                    "queued for session {session_id}: its channel has not claimed it since \
                     the last restart (#1206) — it delivers on the next claim"
                ),
                serde_json::json!({ "notify_id": notify_id.to_string() }),
            )
        }
        Delivery::RefusedInFlight { redirected_to } => {
            let who = redirected_to.map_or_else(
                || session_id.to_string(),
                |to| format!("{to} (redirected from {session_id})"),
            );
            (
                "refused_in_flight",
                format!(
                    "session {who} is mid-turn and interrupt was not set — retry when \
                     idle or resend with interrupt=true (#13 failsafe)"
                ),
                serde_json::json!({}),
            )
        }
        Delivery::NoRoute => (
            "no_route",
            format!("no live route for session {session_id} and nothing is holding it"),
            serde_json::json!({}),
        ),
    };

    JsonRpcResponse::success(req_id, {
        let mut body = serde_json::json!({ "outcome": outcome, "detail": detail });
        if let (Some(obj), Some(extra_obj)) = (body.as_object_mut(), extra.as_object()) {
            for (k, v) in extra_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
        body
    })
}

/// Handle a `session/notify-status` JSON-RPC call (fork #146): the A2A twin
/// of the agent tool's `action: "status"` — poll a notify receipt by id.
/// Same in-memory honesty: receipts die with the process, an unknown id
/// after a restart reports `unknown_id` instead of guessing.
pub fn handle_notify_status(
    req_id: serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    use crate::brain::agent::service::notify_receipts::{self, ReceiptState};

    let raw = match params.get("notify_id").and_then(serde_json::Value::as_str) {
        Some(raw) => raw,
        None => {
            return JsonRpcResponse::error(
                req_id,
                error_codes::INVALID_PARAMS,
                "'notify_id' is required",
            );
        }
    };
    let id: uuid::Uuid = match raw.parse() {
        Ok(id) => id,
        Err(_) => {
            return JsonRpcResponse::error(
                req_id,
                error_codes::INVALID_PARAMS,
                format!("'notify_id' is not a valid UUID: {raw}"),
            );
        }
    };
    match notify_receipts::status(id) {
        None => JsonRpcResponse::success(
            req_id,
            serde_json::json!({
                "outcome": "unknown_id",
                "detail": format!(
                    "no notification {id} is tracked in this process — receipts are \
                     in-memory and do not survive restarts"
                ),
                "notify_id": id.to_string(),
                "notify_state": "unknown_id",
            }),
        ),
        Some(receipt) => {
            let (outcome, detail) = match receipt.state {
                ReceiptState::Injected => {
                    let at = receipt
                        .injected_at
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_default();
                    (
                        "injected",
                        format!(
                            "notification {id} was INJECTED into session {}'s model \
                             context at {at} — the receiving machinery consumed it",
                            receipt.target
                        ),
                    )
                }
                ReceiptState::Queued => (
                    "queued",
                    format!(
                        "notification {id} is routed to session {} but NOT yet observed \
                         at a tool-loop drain point — delivery != queue acceptance",
                        receipt.target
                    ),
                ),
            };
            let mut body = serde_json::json!({
                "outcome": outcome,
                "detail": detail,
                "notify_id": id.to_string(),
                "notify_state": receipt.state.as_str(),
                "notify_target": receipt.target.to_string(),
                "queued_at": receipt.queued_at.to_rfc3339(),
            });
            if let Some(at) = receipt.injected_at {
                body["injected_at"] = serde_json::json!(at.to_rfc3339());
            }
            JsonRpcResponse::success(req_id, body)
        }
    }
}
