//! The prompt bridge: one `session/prompt` request drives one full tool-loop
//! turn, translating in both directions —
//!
//! - loop `ProgressEvent`s -> outbound `session/update` notifications
//!   (message chunks, thought chunks, tool-call lifecycle, usage);
//! - approval-gated tools -> outbound `session/request_permission` calls,
//!   whose client answer becomes the loop's `(approved, always)` pair;
//! - `session/cancel` -> the turn's `CancellationToken`, surfacing as
//!   `stopReason: "cancelled"`.
//!
//! Constraint that shapes the code: `ProgressCallback` is a synchronous
//! `Fn`, so the tool-call-id pairing map is a plain `std::sync::Mutex` —
//! held for microseconds, never across an await. `ApprovalCallback` returns
//! a future, so the permission round-trip is free to await the client.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::brain::agent::{ApprovalCallback, ProgressCallback, ProgressEvent, ToolApprovalInfo};
use crate::brain::provider::StopReason;

use super::protocol::{self, permission_options, permission_outcome, session_update, text_chunk};
use super::server::{ServerState, SessionState};
use super::transport::TransportHandle;

/// Run one turn to completion and answer the `session/prompt` request.
pub async fn run_turn(
    state: Arc<ServerState>,
    session: Arc<SessionState>,
    request_id: Value,
    text: String,
) {
    let cancel = CancellationToken::new();
    *session.active_cancel.lock().await = Some(cancel.clone());
    let acp_session_id = session.id.to_string();

    let progress = progress_callback(state.handle.clone(), acp_session_id.clone());
    let approval = approval_callback(state.handle.clone(), acp_session_id);

    let model = session.model.lock().await.clone();
    let result = state
        .agent
        .send_message_with_tools_and_callback(
            session.id,
            text,
            model,
            Some(cancel.clone()),
            Some(approval),
            Some(progress),
            "acp",
            None,
            None,
        )
        .await;

    *session.active_cancel.lock().await = None;

    match result {
        Ok(response) => {
            let stop = if cancel.is_cancelled() {
                "cancelled"
            } else {
                stop_reason(response.stop_reason)
            };
            state
                .handle
                .respond(request_id, json!({ "stopReason": stop }));
        }
        Err(_) if cancel.is_cancelled() => {
            // A cancelled turn may surface as an error from the loop; the
            // client only cares that the turn ended by cancel.
            state
                .handle
                .respond(request_id, json!({ "stopReason": "cancelled" }));
        }
        Err(e) => {
            tracing::warn!("acp turn failed: {e:#}");
            state.handle.respond_error(
                request_id,
                protocol::INTERNAL_ERROR,
                crate::brain::agent::format_user_error(&e),
            );
        }
    }
}

/// ACP stop reasons from the provider's.
fn stop_reason(reason: Option<StopReason>) -> &'static str {
    match reason {
        Some(StopReason::MaxTokens) => "max_tokens",
        Some(StopReason::StopSequence) => "stop_sequence",
        // EndTurn/ToolUse/None: the turn completed — ToolUse means the loop
        // ended after tool execution, which from the client's seat is a
        // finished turn.
        _ => "end_turn",
    }
}

/// Map loop progress to `session/update` notifications.
///
/// Tool-call ids are invented here: `ProgressEvent` carries tool names but
/// no call ids, so each `ToolStarted` mints a UUID and the matching
/// `ToolCompleted` pops it FIFO per tool name. Parallel same-name tools pair
/// in start order, which is the only honest pairing the events support.
fn progress_callback(handle: TransportHandle, acp_session_id: String) -> ProgressCallback {
    let open_calls: Arc<StdMutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(StdMutex::new(HashMap::new()));

    Arc::new(move |_session_id, event| {
        let update = match event {
            ProgressEvent::StreamingChunk { text }
            | ProgressEvent::IntermediateText { text, .. } => {
                Some(text_chunk("agent_message_chunk", &text))
            }
            ProgressEvent::ReasoningChunk { text } => {
                Some(text_chunk("agent_thought_chunk", &text))
            }
            ProgressEvent::ToolStarted {
                tool_name,
                tool_input,
            } => {
                let call_id = Uuid::new_v4().to_string();
                if let Ok(mut calls) = open_calls.lock() {
                    calls
                        .entry(tool_name.clone())
                        .or_default()
                        .push_back(call_id.clone());
                }
                Some(json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": call_id,
                    "title": tool_name,
                    "kind": protocol::tool_kind(&tool_name),
                    "status": "in_progress",
                    "rawInput": tool_input,
                }))
            }
            ProgressEvent::ToolCompleted {
                tool_name,
                success,
                summary,
                ..
            } => {
                let call_id = open_calls
                    .lock()
                    .ok()
                    .and_then(|mut calls| calls.get_mut(&tool_name).and_then(VecDeque::pop_front))
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                Some(json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": call_id,
                    "status": if success { "completed" } else { "failed" },
                    "rawOutput": summary,
                }))
            }
            ProgressEvent::TokenCount(used) => Some(json!({
                "sessionUpdate": "usage",
                "usage": { "used": used },
            })),
            // Everything else (compaction notices, retry ticker, provider
            // switches, suggestions, stream strips) has no ACP vocabulary —
            // logged by the loop already, not re-broadcast here.
            _ => None,
        };

        if let Some(update) = update {
            handle.send(session_update(&acp_session_id, update));
        }
    })
}

/// Route approval-gated tools to the client as `session/request_permission`.
/// Any transport failure denies — a client that cannot answer must never
/// become an approval.
fn approval_callback(handle: TransportHandle, acp_session_id: String) -> ApprovalCallback {
    Arc::new(move |info: ToolApprovalInfo| {
        let handle = handle.clone();
        let acp_session_id = acp_session_id.clone();
        Box::pin(async move {
            let params = json!({
                "sessionId": acp_session_id,
                "toolCall": {
                    "toolCallId": Uuid::new_v4().to_string(),
                    "title": info.tool_name,
                    "kind": protocol::tool_kind(&info.tool_name),
                    "rawInput": info.tool_input,
                },
                "options": permission_options(),
            });
            match handle
                .call(protocol::SESSION_REQUEST_PERMISSION, params)
                .await
            {
                Ok(result) => Ok(permission_outcome(&result)),
                Err(e) => {
                    tracing::warn!("acp permission request failed, denying: {e:#}");
                    Ok((false, false))
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reasons_map_to_acp() {
        assert_eq!(stop_reason(Some(StopReason::EndTurn)), "end_turn");
        assert_eq!(stop_reason(Some(StopReason::MaxTokens)), "max_tokens");
        assert_eq!(stop_reason(Some(StopReason::StopSequence)), "stop_sequence");
        assert_eq!(stop_reason(Some(StopReason::ToolUse)), "end_turn");
        assert_eq!(stop_reason(None), "end_turn");
    }
}
