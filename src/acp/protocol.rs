//! JSON-RPC 2.0 framing plus the ACP vocabulary the server speaks.
//!
//! ACP is JSON-RPC over NDJSON: one message per line on stdin/stdout. Three
//! message shapes arrive from the client — requests (`id` + `method`),
//! notifications (`method`, no `id`), and responses to OUR outbound requests
//! (`id` + `result`/`error`, no `method`) — so parsing is a classify step,
//! not a deserialize-into-one-struct step.

use serde_json::{Value, json};

/// Standard JSON-RPC error codes plus ACP's auth-required code.
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

// Inbound methods (client -> agent).
pub const INITIALIZE: &str = "initialize";
pub const SESSION_NEW: &str = "session/new";
pub const SESSION_LOAD: &str = "session/load";
pub const SESSION_PROMPT: &str = "session/prompt";
pub const SESSION_SET_MODEL: &str = "session/set_model";
pub const SESSION_CANCEL: &str = "session/cancel";
pub const SESSION_STEER: &str = "session/steer";

// Outbound frames (agent -> client).
pub const SESSION_UPDATE: &str = "session/update";
pub const SESSION_REQUEST_PERMISSION: &str = "session/request_permission";

/// A classified inbound JSON-RPC message.
#[derive(Debug)]
pub enum ClientMessage {
    /// `id` kept as a raw Value: JSON-RPC allows string or number ids and the
    /// response must echo it verbatim.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// Response to a server-initiated request. Only numeric ids are ours.
    Response {
        id: u64,
        result: Result<Value, RpcError>,
    },
}

/// A JSON-RPC error object.
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// Parse one NDJSON line into a classified message.
///
/// Classification rule: presence of `method` decides request vs notification
/// (by presence of `id`); absence of `method` with an `id` is a response.
/// Anything else is an invalid-request error the caller reports.
pub fn parse_line(line: &str) -> Result<ClientMessage, (Value, RpcError)> {
    let value: Value = serde_json::from_str(line).map_err(|e| {
        (
            Value::Null,
            RpcError {
                code: PARSE_ERROR,
                message: format!("invalid JSON: {e}"),
            },
        )
    })?;

    let method = value.get("method").and_then(Value::as_str);
    let id = value.get("id").cloned();

    match (method, id) {
        (Some(m), Some(id)) => Ok(ClientMessage::Request {
            id,
            method: m.to_string(),
            params: value.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(m), None) => Ok(ClientMessage::Notification {
            method: m.to_string(),
            params: value.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(id)) => {
            let Some(n) = id.as_u64() else {
                // Not one of ours — outbound ids are always u64.
                return Err((
                    id,
                    RpcError {
                        code: INVALID_REQUEST,
                        message: "response id is not a server-issued id".into(),
                    },
                ));
            };
            let result = if let Some(err) = value.get("error") {
                Err(RpcError {
                    code: err
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or(INTERNAL_ERROR),
                    message: err
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                })
            } else {
                Ok(value.get("result").cloned().unwrap_or(Value::Null))
            };
            Ok(ClientMessage::Response { id: n, result })
        }
        (None, None) => Err((
            Value::Null,
            RpcError {
                code: INVALID_REQUEST,
                message: "message has neither method nor id".into(),
            },
        )),
    }
}

/// A successful response frame.
pub fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An error response frame.
pub fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}

/// A notification frame (no id).
pub fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

/// A server-initiated request frame.
pub fn request_frame(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// The `session/update` notification for one session.
pub fn session_update(session_id: &str, update: Value) -> Value {
    notification(
        SESSION_UPDATE,
        json!({ "sessionId": session_id, "update": update }),
    )
}

/// A text `agent_message_chunk` / `agent_thought_chunk` update body.
pub fn text_chunk(kind: &str, text: &str) -> Value {
    json!({
        "sessionUpdate": kind,
        "content": { "type": "text", "text": text },
    })
}

/// `initialize` result: protocol version plus the capabilities we honor.
/// fs/terminal are false — MonoCode's adapter declares them false too, so
/// file ops stay on the agent side where the tool loop already has them.
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": 1,
        "agentCapabilities": {
            "loadSession": true,
            "promptCapabilities": { "text": true, "image": false, "embeddedContext": false },
        },
        "agentInfo": {
            "name": "opencrabs",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

/// Extract the text of a `session/prompt` (or `session/steer`) prompt block
/// list. Text blocks pass through; resource/resource_link blocks contribute
/// their uri as a readable reference so attachments survive as paths.
pub fn prompt_text(params: &Value) -> Option<String> {
    let blocks = params.get("prompt")?.as_array()?;
    let mut parts = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    parts.push(t.to_string());
                }
            }
            Some("resource_link") => {
                if let Some(uri) = block.get("uri").and_then(Value::as_str) {
                    parts.push(format!("Attached file (read from disk): {uri:?}"));
                }
            }
            Some("resource") => {
                if let Some(uri) = block
                    .get("resource")
                    .and_then(|r| r.get("uri"))
                    .and_then(Value::as_str)
                {
                    parts.push(format!("Attached file (read from disk): {uri:?}"));
                }
            }
            _ => {}
        }
    }
    let text = parts.join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Map an opencrabs tool name to an ACP tool-call `kind`. The client uses
/// the kind for icons and for plan-mode read-only gating (read/search pass,
/// everything else asks).
pub fn tool_kind(tool_name: &str) -> &'static str {
    match tool_name {
        "read_file" | "load_brain_file" => "read",
        "edit_file" | "write_file" | "hashline_edit" | "write_opencrabs_file" => "edit",
        "grep" | "glob" | "memory_search" | "kb_search" | "web_search" | "exa_search"
        | "brave_search" | "session_search" | "channel_search" => "search",
        "bash" | "execute_code" | "slash_command" => "execute",
        "http_request" | "web_scrape" => "fetch",
        "plan" | "bankai_task" | "bankai_prime" | "session_context" => "think",
        _ => "other",
    }
}

/// The option list sent with every `session/request_permission`. Ids match
/// the adapter's preference order (`allow-once`, `allow-always`, `reject-once`).
pub fn permission_options() -> Value {
    json!([
        { "optionId": "allow-once", "name": "Allow once", "kind": "allow_once" },
        { "optionId": "allow-always", "name": "Always allow", "kind": "allow_always" },
        { "optionId": "reject-once", "name": "Reject", "kind": "reject_once" },
    ])
}

/// Map a permission response onto the loop's `(approved, always)` pair.
/// `cancelled` outcomes and unknown option ids deny — a confused client must
/// never become an approval.
pub fn permission_outcome(result: &Value) -> (bool, bool) {
    let outcome = result.get("outcome").unwrap_or(result);
    let selected = outcome.get("outcome").and_then(Value::as_str);
    if selected != Some("selected") {
        return (false, false);
    }
    match outcome.get("optionId").and_then(Value::as_str) {
        Some("allow-always") | Some("allow_always") => (true, true),
        Some("allow-once") | Some("allow_once") | Some("allow") => (true, false),
        _ => (false, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request() {
        let msg = parse_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/tmp"}}"#,
        )
        .unwrap();
        match msg {
            ClientMessage::Request { id, method, params } => {
                assert_eq!(id, json!(1));
                assert_eq!(method, "session/new");
                assert_eq!(params["cwd"], json!("/tmp"));
            }
            other => panic!("expected request, got {other:?}"),
        }
    }

    #[test]
    fn parses_notification() {
        let msg = parse_line(r#"{"jsonrpc":"2.0","method":"session/cancel","params":{}}"#).unwrap();
        assert!(matches!(msg, ClientMessage::Notification { .. }));
    }

    #[test]
    fn parses_response_result_and_error() {
        let ok = parse_line(r#"{"jsonrpc":"2.0","id":7,"result":{"x":1}}"#).unwrap();
        match ok {
            ClientMessage::Response { id, result } => {
                assert_eq!(id, 7);
                assert_eq!(result.unwrap()["x"], json!(1));
            }
            other => panic!("expected response, got {other:?}"),
        }
        let err = parse_line(r#"{"jsonrpc":"2.0","id":8,"error":{"code":-32601,"message":"no"}}"#)
            .unwrap();
        match err {
            ClientMessage::Response { id, result } => {
                assert_eq!(id, 8);
                let e = result.unwrap_err();
                assert_eq!(e.code, -32601);
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_line("not json").is_err());
        assert!(parse_line(r#"{"jsonrpc":"2.0"}"#).is_err());
    }

    #[test]
    fn extracts_prompt_text_blocks() {
        let params = json!({
            "prompt": [
                { "type": "text", "text": "hello" },
                { "type": "resource_link", "uri": "file:///tmp/a.png" },
                { "type": "image", "data": "..." }
            ]
        });
        let text = prompt_text(&params).unwrap();
        assert!(text.starts_with("hello"));
        assert!(text.contains("file:///tmp/a.png"));
    }

    #[test]
    fn permission_outcome_maps_option_ids() {
        assert_eq!(
            permission_outcome(&json!({"outcome":{"outcome":"selected","optionId":"allow-once"}})),
            (true, false)
        );
        assert_eq!(
            permission_outcome(
                &json!({"outcome":{"outcome":"selected","optionId":"allow-always"}})
            ),
            (true, true)
        );
        assert_eq!(
            permission_outcome(&json!({"outcome":{"outcome":"selected","optionId":"reject-once"}})),
            (false, false)
        );
        assert_eq!(
            permission_outcome(&json!({"outcome":{"outcome":"cancelled"}})),
            (false, false)
        );
    }

    #[test]
    fn tool_kind_covers_loop_tools() {
        assert_eq!(tool_kind("bash"), "execute");
        assert_eq!(tool_kind("read_file"), "read");
        assert_eq!(tool_kind("edit_file"), "edit");
        assert_eq!(tool_kind("grep"), "search");
        assert_eq!(tool_kind("http_request"), "fetch");
        assert_eq!(tool_kind("spawn_agent"), "other");
    }
}
