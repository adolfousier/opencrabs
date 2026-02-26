//! Handler for `message/send` — creates a task and processes it via AgentService.

use super::{CancelStore, TaskStore};
use crate::a2a::{persistence, types::*};
use crate::brain::agent::service::AgentService;
use crate::services::ServiceContext;
use crate::services::SessionService;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Handle `message/send` — create a task and spawn background processing.
pub async fn handle_send_message(
    id: serde_json::Value,
    params: serde_json::Value,
    store: TaskStore,
    cancel_store: CancelStore,
    agent_service: Arc<AgentService>,
    service_context: ServiceContext,
) -> JsonRpcResponse {
    let send_params: SendMessageParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                id,
                error_codes::INVALID_PARAMS,
                format!("Invalid params: {}", e),
            );
        }
    };

    let user_text = send_params
        .message
        .parts
        .iter()
        .filter_map(|p| p.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");

    if user_text.trim().is_empty() {
        return JsonRpcResponse::error(
            id,
            error_codes::INVALID_PARAMS,
            "Message must contain at least one text part",
        );
    }

    let task_id = Uuid::new_v4().to_string();
    let context_id = send_params
        .message
        .context_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let task = Task {
        id: task_id.clone(),
        context_id: Some(context_id.clone()),
        status: TaskStatus {
            state: TaskState::Working,
            message: Some(Message {
                message_id: Some(Uuid::new_v4().to_string()),
                context_id: Some(context_id.clone()),
                task_id: Some(task_id.clone()),
                role: Role::Agent,
                parts: vec![Part::text(format!(
                    "Task created. Processing: {}",
                    if user_text.len() > 100 {
                        format!("{}...", &user_text[..user_text.floor_char_boundary(100)])
                    } else {
                        user_text.clone()
                    }
                ))],
                metadata: None,
            }),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        },
        artifacts: vec![],
        history: vec![send_params.message.clone()],
        metadata: None,
    };

    {
        let mut tasks = store.write().await;
        tasks.insert(task_id.clone(), task.clone());
    }
    persistence::upsert_task(&service_context.pool(), &task).await;

    // Determine skill and read-only mode from configuration metadata
    let read_only = send_params
        .configuration
        .as_ref()
        .and_then(|c| c.get("skill"))
        .and_then(|s| s.as_str())
        .map(|s| s == "research")
        .unwrap_or(false);

    tracing::info!(
        "A2A: Task {} created, spawning agent (read_only={})",
        task_id,
        read_only
    );

    let bg_store = store.clone();
    let bg_cancel_store = cancel_store.clone();
    let bg_task_id = task_id.clone();
    let bg_context_id = context_id.clone();
    let bg_pool = service_context.pool();
    tokio::spawn(async move {
        process_task(
            bg_store,
            bg_cancel_store,
            bg_task_id,
            bg_context_id,
            user_text,
            agent_service,
            service_context,
            read_only,
            bg_pool,
        )
        .await;
    });

    let task_json =
        serde_json::to_value(&task).unwrap_or_else(|_| serde_json::json!({"error": "serialize"}));
    JsonRpcResponse::success(id, task_json)
}

/// Background task processor: creates a DB session, invokes the agent, updates the A2A task.
#[allow(clippy::too_many_arguments)]
async fn process_task(
    store: TaskStore,
    cancel_store: CancelStore,
    task_id: String,
    context_id: String,
    user_text: String,
    agent_service: Arc<AgentService>,
    service_context: ServiceContext,
    read_only: bool,
    pool: sqlx::SqlitePool,
) {
    let session_service = SessionService::new(service_context);
    let title = format!(
        "A2A: {}",
        &user_text[..user_text.floor_char_boundary(60.min(user_text.len()))]
    );
    let session_id = match session_service.create_session(Some(title)).await {
        Ok(session) => session.id,
        Err(e) => {
            tracing::error!("A2A: Failed to create session for task {}: {}", task_id, e);
            update_task_failed(
                &store,
                &task_id,
                &context_id,
                &format!("Session creation failed: {}", e),
                &pool,
            )
            .await;
            return;
        }
    };

    let cancel_token = CancellationToken::new();
    {
        let mut tokens = cancel_store.write().await;
        tokens.insert(task_id.clone(), cancel_token.clone());
    }

    let result = agent_service
        .send_message_with_tools_and_mode(
            session_id,
            user_text,
            None,
            read_only,
            Some(cancel_token),
        )
        .await;

    // Clean up cancel token
    {
        let mut tokens = cancel_store.write().await;
        tokens.remove(&task_id);
    }

    match result {
        Ok(response) => {
            let mut tasks = store.write().await;
            if let Some(task) = tasks.get_mut(&task_id) {
                task.status = TaskStatus {
                    state: TaskState::Completed,
                    message: Some(Message {
                        message_id: Some(Uuid::new_v4().to_string()),
                        context_id: Some(context_id),
                        task_id: Some(task_id.clone()),
                        role: Role::Agent,
                        parts: vec![Part::text("Task completed.")],
                        metadata: None,
                    }),
                    timestamp: Some(chrono::Utc::now().to_rfc3339()),
                };
                task.artifacts.push(Artifact {
                    artifact_id: Some(Uuid::new_v4().to_string()),
                    name: Some("response".to_string()),
                    description: Some("Agent response".to_string()),
                    parts: vec![Part::text(&response.content)],
                    metadata: None,
                });
            }
            if let Some(task) = tasks.get(&task_id) {
                persistence::upsert_task(&pool, task).await;
            }
            tracing::info!(
                "A2A: Task {} completed ({} tokens used)",
                task_id,
                response.usage.input_tokens + response.usage.output_tokens
            );
        }
        Err(e) => {
            tracing::error!("A2A: Task {} failed: {}", task_id, e);
            update_task_failed(&store, &task_id, &context_id, &e.to_string(), &pool).await;
        }
    }
}

/// Mark a task as failed in the store and persist to DB.
async fn update_task_failed(
    store: &TaskStore,
    task_id: &str,
    context_id: &str,
    error_msg: &str,
    pool: &sqlx::SqlitePool,
) {
    let mut tasks = store.write().await;
    if let Some(task) = tasks.get_mut(task_id) {
        task.status = TaskStatus {
            state: TaskState::Failed,
            message: Some(Message {
                message_id: Some(Uuid::new_v4().to_string()),
                context_id: Some(context_id.to_string()),
                task_id: Some(task_id.to_string()),
                role: Role::Agent,
                parts: vec![Part::text(format!("Task failed: {}", error_msg))],
                metadata: None,
            }),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        };
        persistence::upsert_task(pool, task).await;
    }
}
