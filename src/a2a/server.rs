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
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
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

    Router::new()
        .route("/.well-known/agent.json", get(get_agent_card))
        .route("/a2a/v1", post(handle_jsonrpc))
        .route("/a2a/health", get(health_check))
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

    let state = A2aState {
        task_store: handler::new_task_store(),
        cancel_store: handler::new_cancel_store(),
        host: config.bind.clone(),
        port: config.port,
        agent_service,
        service_context,
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

/// POST /a2a/v1 — JSON-RPC 2.0 endpoint.
async fn handle_jsonrpc(
    State(state): State<A2aState>,
    Json(req): Json<JsonRpcRequest>,
) -> (StatusCode, Json<JsonRpcResponse>) {
    if req.jsonrpc != "2.0" {
        return (
            StatusCode::OK,
            Json(JsonRpcResponse::error(
                req.id,
                error_codes::INVALID_REQUEST,
                "Invalid JSON-RPC version, expected 2.0",
            )),
        );
    }

    let response = handler::dispatch(
        req,
        state.task_store,
        state.cancel_store,
        state.agent_service,
        state.service_context,
    )
    .await;
    (StatusCode::OK, Json(response))
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
