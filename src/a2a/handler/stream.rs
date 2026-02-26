//! Handler for `message/stream` -- SSE streaming variant of `message/send`.

use super::{CancelStore, TaskStore};
use crate::a2a::{persistence, types::*};
use crate::brain::agent::service::AgentService;
use crate::services::{ServiceContext, SessionService};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// A streaming event sent through the mpsc channel to the SSE handler.
pub type StreamTx = mpsc::Sender<StreamEvent>;

/// Handle `message/stream` -- creates a task, spawns background processing,
/// returns a receiver that yields SSE events.
pub async fn handle_stream_message(
    id: serde_json::Value,
    params: serde_json::Value,
    store: TaskStore,
    cancel_store: CancelStore,
    agent_service: Arc<AgentService>,
    service_context: ServiceContext,
) -> Result<(serde_json::Value, mpsc::Receiver<StreamEvent>), JsonRpcResponse> {
    let send_params: SendMessageParams = serde_json::from_value(params).map_err(|e| {
        JsonRpcResponse::error(
            id.clone(),
            error_codes::INVALID_PARAMS,
            format!("Invalid params: {}", e),
        )
    })?;

    let user_text = send_params
        .message
        .parts
        .iter()
        .filter_map(|p| p.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");

    if user_text.trim().is_empty() {
        return Err(JsonRpcResponse::error(
            id,
            error_codes::INVALID_PARAMS,
            "Message must contain at least one text part",
        ));
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

    let read_only = send_params
        .configuration
        .as_ref()
        .and_then(|c| c.get("skill"))
        .and_then(|s| s.as_str())
        .map(|s| s == "research")
        .unwrap_or(false);

    // Channel for SSE events -- buffer a few events
    let (tx, rx) = mpsc::channel::<StreamEvent>(32);

    // Send initial Task object as first SSE event
    let _ = tx.send(StreamEvent::Task(task)).await;

    let pool = service_context.pool();
    tokio::spawn(async move {
        process_task_streaming(
            store,
            cancel_store,
            task_id,
            context_id,
            user_text,
            agent_service,
            service_context,
            read_only,
            pool,
            tx,
        )
        .await;
    });

    Ok((id, rx))
}

/// Background task processor with SSE event emission.
#[allow(clippy::too_many_arguments)]
async fn process_task_streaming(
    store: TaskStore,
    cancel_store: CancelStore,
    task_id: String,
    context_id: String,
    user_text: String,
    agent_service: Arc<AgentService>,
    service_context: ServiceContext,
    read_only: bool,
    pool: sqlx::SqlitePool,
    tx: StreamTx,
) {
    let session_service = SessionService::new(service_context);
    let title = format!(
        "A2A: {}",
        &user_text[..user_text.floor_char_boundary(60.min(user_text.len()))]
    );
    let session_id = match session_service.create_session(Some(title)).await {
        Ok(session) => session.id,
        Err(e) => {
            tracing::error!(
                "A2A stream: Failed to create session for task {}: {}",
                task_id,
                e
            );
            send_final_status(
                &store,
                &task_id,
                &context_id,
                TaskState::Failed,
                &format!("Session creation failed: {}", e),
                &pool,
                &tx,
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
            // Send artifact update
            let artifact = Artifact {
                artifact_id: Some(Uuid::new_v4().to_string()),
                name: Some("response".to_string()),
                description: Some("Agent response".to_string()),
                parts: vec![Part::text(&response.content)],
                metadata: None,
            };

            let _ = tx
                .send(StreamEvent::ArtifactUpdate(TaskArtifactUpdateEvent {
                    kind: "artifact-update".to_string(),
                    task_id: task_id.clone(),
                    context_id: context_id.clone(),
                    artifact: artifact.clone(),
                    append: Some(false),
                    last_chunk: Some(true),
                    metadata: None,
                }))
                .await;

            // Update store
            {
                let mut tasks = store.write().await;
                if let Some(task) = tasks.get_mut(&task_id) {
                    task.status = TaskStatus {
                        state: TaskState::Completed,
                        message: Some(Message {
                            message_id: Some(Uuid::new_v4().to_string()),
                            context_id: Some(context_id.clone()),
                            task_id: Some(task_id.clone()),
                            role: Role::Agent,
                            parts: vec![Part::text("Task completed.")],
                            metadata: None,
                        }),
                        timestamp: Some(chrono::Utc::now().to_rfc3339()),
                    };
                    task.artifacts.push(artifact);
                    persistence::upsert_task(&pool, task).await;
                }
            }

            // Send final status update
            send_final_status(
                &store,
                &task_id,
                &context_id,
                TaskState::Completed,
                "Task completed.",
                &pool,
                &tx,
            )
            .await;

            tracing::info!(
                "A2A stream: Task {} completed ({} tokens used)",
                task_id,
                response.usage.input_tokens + response.usage.output_tokens
            );
        }
        Err(e) => {
            tracing::error!("A2A stream: Task {} failed: {}", task_id, e);
            send_final_status(
                &store,
                &task_id,
                &context_id,
                TaskState::Failed,
                &format!("Task failed: {}", e),
                &pool,
                &tx,
            )
            .await;
        }
    }
}

/// Send a terminal status update event and persist the state.
async fn send_final_status(
    store: &TaskStore,
    task_id: &str,
    context_id: &str,
    state: TaskState,
    message_text: &str,
    pool: &sqlx::SqlitePool,
    tx: &StreamTx,
) {
    let status = TaskStatus {
        state: state.clone(),
        message: Some(Message {
            message_id: Some(Uuid::new_v4().to_string()),
            context_id: Some(context_id.to_string()),
            task_id: Some(task_id.to_string()),
            role: Role::Agent,
            parts: vec![Part::text(message_text)],
            metadata: None,
        }),
        timestamp: Some(chrono::Utc::now().to_rfc3339()),
    };

    // Update store
    {
        let mut tasks = store.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = status.clone();
            persistence::upsert_task(pool, task).await;
        }
    }

    // Send final SSE event
    let _ = tx
        .send(StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
            kind: "status-update".to_string(),
            task_id: task_id.to_string(),
            context_id: context_id.to_string(),
            status,
            is_final: true,
            metadata: None,
        }))
        .await;
}
