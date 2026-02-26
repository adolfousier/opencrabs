//! A2A Gateway HTTP server powered by axum.
//!
//! Serves:
//! - `GET  /.well-known/agent.json` — Agent Card discovery
//! - `POST /a2a/v1`                 — JSON-RPC 2.0 endpoint
//! - `GET  /a2a/health`             — Health check

use crate::a2a::{agent_card, handler, types::*};
use crate::brain::agent::service::AgentService;
use crate::config::A2aConfig;
use crate::services::ServiceContext;
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Json, Sse, sse},
    routing::{get, post},
};
use futures::stream;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Shared state for the A2A gateway.
#[derive(Clone)]
pub struct A2aState {
    pub task_store: handler::TaskStore,
    pub cancel_store: handler::CancelStore,
    pub host: String,
    pub port: u16,
    pub agent_service: Arc<AgentService>,
    pub service_context: ServiceContext,
    pub api_key: Option<String>,
}

/// Bearer token auth middleware. Skipped when no api_key is configured.
async fn require_bearer(
    State(state): State<A2aState>,
    req: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> axum::response::Response {
    let Some(ref expected) = state.api_key else {
        // No key configured, allow all requests
        return next.run(req).await;
    };

    let authorized = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected);

    if authorized {
        next.run(req).await
    } else {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "error": { "code": -32001, "message": "Unauthorized: invalid or missing Bearer token" },
            "id": null
        });
        (StatusCode::UNAUTHORIZED, Json(body)).into_response()
    }
}

/// Build the axum router for the A2A gateway.
pub fn build_router(state: A2aState, allowed_origins: &[String]) -> Router {
    let cors = if allowed_origins.is_empty() {
        CorsLayer::new()
    } else {
        let origins: Vec<_> = allowed_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new().allow_origin(AllowOrigin::list(origins))
    };

    // Auth-protected JSON-RPC endpoint
    let protected = Router::new()
        .route("/a2a/v1", post(handle_jsonrpc))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ));

    // Public endpoints (discovery + health)
    Router::new()
        .route("/.well-known/agent.json", get(get_agent_card))
        .route("/a2a/health", get(health_check))
        .merge(protected)
        .layer(cors)
        .with_state(state)
}

/// Start the A2A gateway server.
///
/// Runs as a background task — call from `tokio::spawn`.
pub async fn start_server(
    config: &A2aConfig,
    agent_service: Arc<AgentService>,
    service_context: ServiceContext,
) -> anyhow::Result<()> {
    if !config.enabled {
        tracing::info!("A2A gateway disabled in config");
        return Ok(());
    }

    // Restore any in-flight tasks from the database
    let task_store = handler::new_task_store();
    let persisted = super::persistence::load_active_tasks(&service_context.pool()).await;
    if !persisted.is_empty() {
        let mut store = task_store.write().await;
        for task in persisted {
            tracing::info!(
                "A2A: Restored task {} (state: {:?})",
                task.id,
                task.status.state
            );
            store.insert(task.id.clone(), task);
        }
    }

    let state = A2aState {
        task_store,
        cancel_store: handler::new_cancel_store(),
        host: config.bind.clone(),
        port: config.port,
        agent_service,
        service_context,
        api_key: config.api_key.clone(),
    };

    let app = build_router(state, &config.allowed_origins);
    let addr: SocketAddr = format!("{}:{}", config.bind, config.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid A2A gateway address: {}", e))?;

    tracing::info!("A2A Gateway starting on http://{}", addr);
    tracing::info!("   Agent Card: http://{}/.well-known/agent.json", addr);
    tracing::info!("   JSON-RPC:   http://{}/a2a/v1", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// GET /.well-known/agent.json — Agent Card discovery.
async fn get_agent_card(State(state): State<A2aState>) -> Json<AgentCard> {
    let registry = state.agent_service.tool_registry();
    let card = agent_card::build_agent_card(&state.host, state.port, Some(registry));
    Json(card)
}

/// POST /a2a/v1 -- JSON-RPC 2.0 endpoint.
/// Returns JSON for most methods, SSE stream for `message/stream`.
async fn handle_jsonrpc(
    State(state): State<A2aState>,
    Json(req): Json<JsonRpcRequest>,
) -> axum::response::Response {
    if req.jsonrpc != "2.0" {
        return (
            StatusCode::OK,
            Json(JsonRpcResponse::error(
                req.id,
                error_codes::INVALID_REQUEST,
                "Invalid JSON-RPC version, expected 2.0",
            )),
        )
            .into_response();
    }

    // message/stream returns SSE instead of JSON
    if req.method == "message/stream" {
        return handle_stream(state, req).await;
    }

    let response = handler::dispatch(
        req,
        state.task_store,
        state.cancel_store,
        state.agent_service,
        state.service_context.clone(),
    )
    .await;
    (StatusCode::OK, Json(response)).into_response()
}

/// Handle `message/stream` -- returns an SSE stream of task updates.
async fn handle_stream(state: A2aState, req: JsonRpcRequest) -> axum::response::Response {
    match handler::stream::handle_stream_message(
        req.id,
        req.params,
        state.task_store,
        state.cancel_store,
        state.agent_service,
        state.service_context,
    )
    .await
    {
        Ok((id, rx)) => {
            let stream = stream::unfold((id, rx), |(id, mut rx)| async move {
                let event = rx.recv().await?;
                let result = serde_json::to_value(&event).unwrap_or_default();
                let rpc_response = JsonRpcResponse::success(id.clone(), result);
                let data = serde_json::to_string(&rpc_response).unwrap_or_default();
                let sse_event = Ok::<_, std::convert::Infallible>(sse::Event::default().data(data));
                Some((sse_event, (id, rx)))
            });
            Sse::new(stream).into_response()
        }
        Err(error_response) => (StatusCode::OK, Json(error_response)).into_response(),
    }
}

/// GET /a2a/health — Health check.
async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": crate::VERSION,
        "protocol": "A2A",
        "protocol_version": "1.0",
        "provider": "OpenCrabs Community"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn test_state() -> A2aState {
        use crate::a2a::test_helpers::helpers;
        A2aState {
            task_store: handler::new_task_store(),
            cancel_store: handler::new_cancel_store(),
            host: "127.0.0.1".to_string(),
            port: 18790,
            agent_service: helpers::placeholder_agent_service().await,
            service_context: helpers::placeholder_service_context().await,
            api_key: None,
        }
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = build_router(test_state().await, &[]);
        let req = Request::builder()
            .uri("/a2a/health")
            .body(Body::empty())
            .expect("request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_agent_card_endpoint() {
        let app = build_router(test_state().await, &[]);
        let req = Request::builder()
            .uri("/.well-known/agent.json")
            .body(Body::empty())
            .expect("request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
