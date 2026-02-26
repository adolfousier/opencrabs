//! Handlers for `tasks/get` and `tasks/cancel` operations.

use super::{CancelStore, TaskStore};
use crate::a2a::{persistence, types::*};

/// Handle `tasks/get` — retrieve a task by ID.
pub async fn handle_get_task(
    id: serde_json::Value,
    params: serde_json::Value,
    store: TaskStore,
) -> JsonRpcResponse {
    let get_params: GetTaskParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                id,
                error_codes::INVALID_PARAMS,
                format!("Invalid params: {}", e),
            );
        }
    };

    let tasks = store.read().await;
    match tasks.get(&get_params.id) {
        Some(task) => {
            let task_json = serde_json::to_value(task)
                .unwrap_or_else(|_| serde_json::json!({"error": "serialize"}));
            JsonRpcResponse::success(id, task_json)
        }
        None => JsonRpcResponse::error(
            id,
            error_codes::TASK_NOT_FOUND,
            format!("Task not found: {}", get_params.id),
        ),
    }
}

/// Handle `tasks/cancel` — cancel a running task and its background agent.
pub async fn handle_cancel_task(
    id: serde_json::Value,
    params: serde_json::Value,
    store: TaskStore,
    cancel_store: CancelStore,
    pool: &sqlx::SqlitePool,
) -> JsonRpcResponse {
    let cancel_params: CancelTaskParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                id,
                error_codes::INVALID_PARAMS,
                format!("Invalid params: {}", e),
            );
        }
    };

    // Cancel the background agent if running
    {
        let tokens = cancel_store.read().await;
        if let Some(token) = tokens.get(&cancel_params.id) {
            token.cancel();
            tracing::info!(
                "A2A: Sent cancellation signal for task {}",
                cancel_params.id
            );
        }
    }

    let mut tasks = store.write().await;
    match tasks.get_mut(&cancel_params.id) {
        Some(task) => match task.status.state {
            TaskState::Completed | TaskState::Failed | TaskState::Canceled => {
                JsonRpcResponse::error(
                    id,
                    error_codes::UNSUPPORTED_OPERATION,
                    format!("Cannot cancel task in {:?} state", task.status.state),
                )
            }
            _ => {
                task.status.state = TaskState::Canceled;
                task.status.timestamp = Some(chrono::Utc::now().to_rfc3339());
                persistence::upsert_task(pool, task).await;
                tracing::info!("A2A: Canceled task {}", cancel_params.id);
                let task_json = serde_json::to_value(&*task)
                    .unwrap_or_else(|_| serde_json::json!({"error": "serialize"}));
                JsonRpcResponse::success(id, task_json)
            }
        },
        None => JsonRpcResponse::error(
            id,
            error_codes::TASK_NOT_FOUND,
            format!("Task not found: {}", cancel_params.id),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2a::handler::{new_cancel_store, new_task_store};

    #[tokio::test]
    async fn test_cancel_task_not_found() {
        use crate::a2a::test_helpers::helpers;
        let store = new_task_store();
        let cancel_store = new_cancel_store();
        let ctx = helpers::placeholder_service_context().await;
        let resp = handle_cancel_task(
            serde_json::json!(1),
            serde_json::json!({"id": "nonexistent"}),
            store,
            cancel_store,
            &ctx.pool(),
        )
        .await;
        assert!(resp.error.is_some());
        assert_eq!(
            resp.error.as_ref().expect("err").code,
            error_codes::TASK_NOT_FOUND
        );
    }
}
