//! JSON-RPC 2.0 handler for A2A protocol operations.
//!
//! Dispatches JSON-RPC methods:
//! - `message/send` → create task + process message via AgentService
//! - `tasks/get`    → retrieve task by ID
//! - `tasks/cancel` → cancel a running task

mod send;
mod tasks;

use crate::a2a::types::*;
use crate::brain::agent::service::AgentService;
use crate::services::ServiceContext;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// In-memory task store.
pub type TaskStore = Arc<RwLock<HashMap<String, Task>>>;

/// Cancellation token store — keyed by task ID.
pub type CancelStore = Arc<RwLock<HashMap<String, CancellationToken>>>;

/// Create a new empty task store.
pub fn new_task_store() -> TaskStore {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Create a new empty cancel store.
pub fn new_cancel_store() -> CancelStore {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Dispatch a JSON-RPC request to the appropriate handler.
pub async fn dispatch(
    req: JsonRpcRequest,
    store: TaskStore,
    cancel_store: CancelStore,
    agent_service: Arc<AgentService>,
    service_context: ServiceContext,
) -> JsonRpcResponse {
    match req.method.as_str() {
        "message/send" => {
            send::handle_send_message(
                req.id, req.params, store, cancel_store, agent_service, service_context,
            ).await
        }
        "tasks/get" => tasks::handle_get_task(req.id, req.params, store).await,
        "tasks/cancel" => tasks::handle_cancel_task(req.id, req.params, store, cancel_store).await,
        _ => JsonRpcResponse::error(
            req.id,
            error_codes::METHOD_NOT_FOUND,
            format!("Method not found: {}", req.method),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_send_request() -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "message/send".to_string(),
            params: serde_json::json!({
                "message": {
                    "role": "user",
                    "parts": [{"text": "Hello, agent!"}]
                }
            }),
            id: serde_json::json!(1),
        }
    }

    // tasks/get and tasks/cancel tests don't need AgentService
    #[tokio::test]
    async fn test_get_task_not_found() {
        let store = new_task_store();
        let cancel_store = new_cancel_store();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tasks/get".to_string(),
            params: serde_json::json!({"id": "nonexistent"}),
            id: serde_json::json!(2),
        };
        let resp = tasks::handle_get_task(req.id, req.params, store).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().expect("err").code, -32001);
    }

    #[tokio::test]
    async fn test_unknown_method() {
        use crate::a2a::test_helpers::helpers;
        let store = new_task_store();
        let cancel_store = new_cancel_store();
        let agent = helpers::placeholder_agent_service().await;
        let ctx = helpers::placeholder_service_context().await;
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "unknown/method".to_string(),
            params: serde_json::json!({}),
            id: serde_json::json!(99),
        };
        let resp = dispatch(req, store, cancel_store, agent, ctx).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().expect("err").code, -32601);
    }
}
