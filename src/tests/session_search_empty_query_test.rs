//! Regression tests for the empty-search-query fallback.
//!
//! `session_search {operation: "search"}` with a missing, empty, or
//! whitespace-only query was a hard error — the #1 avoidable failure class
//! in the feedback ledger (600+ occurrences), driven by autoheal/cron loops
//! that want "the recent sessions" and reach for search with no query. The
//! tool now returns the recent-session list with a banner that teaches the
//! right call instead of erroring into the retry loop.

use crate::brain::tools::session_search::SessionSearchTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use crate::db::Database;
use serde_json::json;
use uuid::Uuid;

async fn setup() -> SessionSearchTool {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    SessionSearchTool::new(db.pool().clone())
}

fn ctx() -> ToolExecutionContext {
    ToolExecutionContext::new(Uuid::new_v4())
}

#[tokio::test]
async fn missing_query_returns_list_not_error() {
    let tool = setup().await;
    let result = tool
        .execute(json!({"operation": "search"}), &ctx())
        .await
        .unwrap();
    assert!(
        result.success,
        "empty search must succeed via fallback, got: {}",
        result.output
    );
    assert!(
        result.output.contains("Empty 'query' on search"),
        "fallback banner must explain what happened: {}",
        result.output
    );
    assert!(
        result.output.contains("operation='tail'")
            || result.output.contains("operation='list'"),
        "banner must teach the correct operations: {}",
        result.output
    );
}

#[tokio::test]
async fn whitespace_query_is_treated_as_empty() {
    let tool = setup().await;
    let result = tool
        .execute(json!({"operation": "search", "query": "   "}), &ctx())
        .await
        .unwrap();
    assert!(
        result.success,
        "whitespace-only query must take the fallback, got: {}",
        result.output
    );
    assert!(result.output.contains("Empty 'query' on search"));
}

#[tokio::test]
async fn real_query_still_searches() {
    let tool = setup().await;
    let result = tool
        .execute(json!({"operation": "search", "query": "definitely-no-such-session"}), &ctx())
        .await
        .unwrap();
    // A real (non-empty) query must NOT hit the fallback banner.
    assert!(!result.output.contains("Empty 'query' on search"));
}
