//! Lightweight health endpoint for daemon mode.
//!
//! Binds `0.0.0.0:<port>` and responds to `GET /health` with 200 OK + JSON status.
//! Configure via `[daemon] health_port = 8080` in config.toml.

use axum::{Json, Router, routing::get};
use std::net::SocketAddr;

pub async fn serve(port: u16) -> anyhow::Result<()> {
    let app = Router::new().route("/health", get(health));

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    tracing::info!("Daemon health endpoint listening on http://{}/health", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": crate::VERSION,
        "mode": "daemon",
    }))
}
