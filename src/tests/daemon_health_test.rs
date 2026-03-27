//! Daemon mode tests: health endpoint, config, and channel resilience.

use crate::config::{Config, DaemonConfig};

// ── DaemonConfig ────────────────────────────────────────────────

#[test]
fn daemon_config_default_has_no_health_port() {
    let cfg = DaemonConfig::default();
    assert!(cfg.health_port.is_none());
}

#[test]
fn daemon_config_deserializes_health_port() {
    let toml_str = r#"
        [daemon]
        health_port = 8080
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.daemon.health_port, Some(8080));
}

#[test]
fn daemon_config_deserializes_empty() {
    let toml_str = r#"
        [daemon]
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert!(config.daemon.health_port.is_none());
}

#[test]
fn daemon_config_missing_section_uses_default() {
    let toml_str = "";
    let config: Config = toml::from_str(toml_str).unwrap();
    assert!(config.daemon.health_port.is_none());
}

#[test]
fn config_default_includes_daemon() {
    let config = Config::default();
    assert!(config.daemon.health_port.is_none());
}

// ── Health endpoint ─────────────────────────────────────────────

#[tokio::test]
async fn health_endpoint_returns_ok() {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    // Build the same router as daemon_health::serve
    let app = Router::new().route(
        "/health",
        get(|| async {
            axum::Json(serde_json::json!({
                "status": "ok",
                "version": crate::VERSION,
                "mode": "daemon",
            }))
        }),
    );

    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["mode"], "daemon");
    assert_eq!(json["version"], crate::VERSION);
}

#[tokio::test]
async fn health_endpoint_wrong_path_returns_404() {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    let app = Router::new().route(
        "/health",
        get(|| async { axum::Json(serde_json::json!({"status": "ok"})) }),
    );

    let req = Request::builder()
        .uri("/wrong")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── CLI provider listing (configured_providers) ─────────────────

#[test]
fn cli_providers_always_listed() {
    // CLI providers (claude-cli, opencode-cli) should appear in configured_providers
    // even without any config, since they don't need API keys.
    let providers = crate::config::ProviderConfigs::default();
    let configured = crate::utils::providers::configured_providers(&providers);
    let ids: Vec<&str> = configured.iter().map(|(id, _)| id.as_str()).collect();
    assert!(
        ids.contains(&"claude-cli"),
        "claude-cli should always be listed"
    );
    assert!(
        ids.contains(&"opencode-cli"),
        "opencode-cli should always be listed"
    );
}

#[test]
fn api_key_providers_not_listed_without_key() {
    // Providers that need API keys should NOT appear without one
    let providers = crate::config::ProviderConfigs::default();
    let configured = crate::utils::providers::configured_providers(&providers);
    let ids: Vec<&str> = configured.iter().map(|(id, _)| id.as_str()).collect();
    assert!(!ids.contains(&"anthropic"));
    assert!(!ids.contains(&"openai"));
    assert!(!ids.contains(&"gemini"));
}

#[test]
fn api_key_provider_listed_with_key() {
    use crate::config::{ProviderConfig, ProviderConfigs};
    let providers = ProviderConfigs {
        anthropic: Some(ProviderConfig {
            api_key: Some("sk-test-key".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let configured = crate::utils::providers::configured_providers(&providers);
    let ids: Vec<&str> = configured.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"anthropic"));
}
