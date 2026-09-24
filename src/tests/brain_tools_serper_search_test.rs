//! Tests for the Serper (Google SERP) search engine (#1731).
//!
//! Response shape is pinned to the fields the engine consumes (`organic[]`
//! with `title`/`link`/`snippet`) plus the extras real payloads carry
//! (`position`, `date`, `sitelinks`, `attributes`, `knowledgeGraph`,
//! `searchParameters`) to prove the parser tolerates them. Field names
//! cross-verified against Serper's OpenAPI spec and independent production
//! consumers (deepset-ai/haystack, fetchium-core, rustsandbox/serper).

use crate::brain::tools::Tool;
use crate::brain::tools::ToolCapability;
use crate::brain::tools::serper_search::*;

fn make_tool() -> SerperSearchTool {
    SerperSearchTool::new("test-key".to_string())
}

#[test]
fn test_tool_name() {
    let tool = make_tool();
    assert_eq!(tool.name(), "serper_search");
}

#[test]
fn test_tool_capabilities() {
    let tool = make_tool();
    let caps = tool.capabilities();
    assert_eq!(caps.len(), 1);
    assert!(matches!(caps[0], ToolCapability::Network));
}

#[test]
fn test_tool_no_approval_required() {
    let tool = make_tool();
    assert!(!tool.requires_approval());
}

#[test]
fn test_input_schema_has_query_and_locale_params() {
    let tool = make_tool();
    let schema = tool.input_schema();
    let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("query")));
    let props = schema.get("properties").unwrap();
    for key in ["query", "max_results", "gl", "hl"] {
        assert!(props.get(key).is_some(), "schema must expose {key}");
    }
}

#[test]
fn test_validate_valid_input() {
    let tool = make_tool();
    let input = serde_json::json!({ "query": "rust programming" });
    assert!(tool.validate_input(&input).is_ok());
}

#[test]
fn test_validate_locale_input() {
    let tool = make_tool();
    let input = serde_json::json!({ "query": "rust", "gl": "pt", "hl": "en" });
    assert!(tool.validate_input(&input).is_ok());
}

#[test]
fn test_validate_empty_query() {
    let tool = make_tool();
    let input = serde_json::json!({ "query": "" });
    assert!(tool.validate_input(&input).is_err());
}

#[test]
fn test_validate_missing_query() {
    let tool = make_tool();
    let input = serde_json::json!({ "max_results": 5 });
    assert!(tool.validate_input(&input).is_err());
}

#[test]
fn test_validate_max_results_bounds() {
    let tool = make_tool();
    assert!(
        tool.validate_input(&serde_json::json!({ "query": "q", "max_results": 0 }))
            .is_err()
    );
    assert!(
        tool.validate_input(&serde_json::json!({ "query": "q", "max_results": 11 }))
            .is_err()
    );
}

#[test]
fn test_validate_empty_locale_code_rejected() {
    let tool = make_tool();
    let input = serde_json::json!({ "query": "rust", "gl": "  " });
    assert!(tool.validate_input(&input).is_err());
}

#[test]
fn test_default_deserialization() {
    let input: SerperSearchInput =
        serde_json::from_value(serde_json::json!({ "query": "hello" })).unwrap();
    assert_eq!(input.query, "hello");
    assert_eq!(input.max_results, 5);
    assert!(input.gl.is_none());
    assert!(input.hl.is_none());
}

#[test]
fn test_locale_deserialization() {
    let input: SerperSearchInput = serde_json::from_value(serde_json::json!({
        "query": "hello", "gl": "pt", "hl": "pt"
    }))
    .unwrap();
    assert_eq!(input.gl.as_deref(), Some("pt"));
    assert_eq!(input.hl.as_deref(), Some("pt"));
}

/// Request body omits `gl`/`hl` when unset so the API defaults apply.
#[test]
fn test_request_omits_unset_locale_fields() {
    let req = SerperRequest {
        q: "rust",
        num: 5,
        gl: None,
        hl: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert!(json.get("gl").is_none(), "unset gl must be omitted");
    assert!(json.get("hl").is_none(), "unset hl must be omitted");
    assert_eq!(json.get("q").and_then(|v| v.as_str()), Some("rust"));
    assert_eq!(json.get("num").and_then(|v| v.as_u64()), Some(5));
}

#[test]
fn test_request_includes_set_locale_fields() {
    let req = SerperRequest {
        q: "rust",
        num: 5,
        gl: Some("pt"),
        hl: Some("en"),
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json.get("gl").and_then(|v| v.as_str()), Some("pt"));
    assert_eq!(json.get("hl").and_then(|v| v.as_str()), Some("en"));
}

/// Fixture modeled on Serper's documented payload: `organic[]` entries with
/// the consumed trio plus the fields real responses carry and the parser must
/// ignore (`position`, `date`, `sitelinks`, `attributes`, `searchParameters`).
#[test]
fn test_serper_response_parsing() {
    let json = serde_json::json!({
        "searchParameters": { "q": "rust programming language", "gl": "us", "hl": "en", "type": "search", "num": 5 },
        "organic": [
            {
                "position": 1,
                "title": "Rust Programming Language",
                "link": "https://www.rust-lang.org/",
                "snippet": "A language empowering everyone to build reliable and efficient software."
            },
            {
                "position": 2,
                "title": "GitHub - rust-lang/rust",
                "link": "https://github.com/rust-lang/rust",
                "snippet": "Empowering everyone to build reliable and efficient software.",
                "date": "2 days ago",
                "sitelinks": [
                    { "title": "Releases", "link": "https://github.com/rust-lang/rust/releases" }
                ],
                "attributes": { "Stars": "100k" }
            }
        ],
        "credits": 1
    });
    let response: SerperResponse = serde_json::from_value(json).unwrap();
    assert_eq!(response.organic.len(), 2);
    assert_eq!(response.organic[0].title, "Rust Programming Language");
    assert_eq!(response.organic[0].link, "https://www.rust-lang.org/");
    assert_eq!(
        response.organic[0].snippet,
        Some(
            "A language empowering everyone to build reliable and efficient software.".to_string()
        )
    );
    // Enriched entry: consumed fields intact, extras silently ignored.
    assert_eq!(
        response.organic[1].link,
        "https://github.com/rust-lang/rust"
    );
    assert_eq!(
        response.organic[1].snippet.as_deref(),
        Some("Empowering everyone to build reliable and efficient software.")
    );
}

/// Real error bodies (and query shapes with no organic hits) must not crash
/// the parser: `organic` defaults to empty.
#[test]
fn test_serper_response_without_organic_is_empty() {
    let response: SerperResponse =
        serde_json::from_value(serde_json::json!({ "credits": 1 })).unwrap();
    assert!(response.organic.is_empty());

    let error_body: SerperResponse = serde_json::from_value(serde_json::json!({
        "message": "Unauthorized. Sign up for a free account.", "statusCode": 403
    }))
    .unwrap();
    assert!(error_body.organic.is_empty());
}

// ---- Registration gating (#1731): paid API never registers implicitly ----
//
// Drives the real registration path (`tool_setup::register_core_agent_tools`
// with an in-memory DB), not just `register_config_dependent_tools`, so the
// test pins the same wiring a booting binary uses (pattern: headless surface
// test, #129).

use crate::brain::tools::registry::ToolRegistry;
use crate::cli::tool_setup::register_core_agent_tools;
use crate::config::{Config, ProviderConfig, WebSearchProviders};
use crate::db::Database;
use std::sync::Arc;

fn serper_config(enabled: bool, api_key: Option<&str>) -> Config {
    let mut config = Config::default();
    config.providers.web_search = Some(WebSearchProviders {
        serper: api_key.map(|k| ProviderConfig {
            enabled,
            api_key: Some(k.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    });
    config
}

async fn gated_registry(config: &Config) -> Arc<ToolRegistry> {
    let db = Database::connect_in_memory().await.expect("in-memory db");
    db.run_migrations().await.expect("migrations");
    let registry = Arc::new(ToolRegistry::new());
    let _subagent_manager = register_core_agent_tools(&registry, &db, config, false);
    registry
}

#[tokio::test]
async fn serper_registers_only_when_enabled_and_keyed() {
    // enabled + key -> registered, and web_search fans out with it.
    let registry = gated_registry(&serper_config(true, Some("k"))).await;
    assert!(registry.has_tool("serper_search"));
    assert!(registry.has_tool("web_search"));

    // enabled but EMPTY key -> not registered.
    let registry = gated_registry(&serper_config(true, Some(""))).await;
    assert!(!registry.has_tool("serper_search"));

    // key but disabled -> not registered (explicit opt-in required).
    let registry = gated_registry(&serper_config(false, Some("k"))).await;
    assert!(!registry.has_tool("serper_search"));

    // no section at all -> not registered; web_search still available.
    let registry = gated_registry(&serper_config(true, None)).await;
    assert!(!registry.has_tool("serper_search"));
    assert!(registry.has_tool("web_search"));
}
