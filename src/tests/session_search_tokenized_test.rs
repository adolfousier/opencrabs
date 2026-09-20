//! Regression tests for #1626 — tokenized AND-match fallback on
//! `session_search`. Verbatim substring search stays the primary path; when
//! a multi-word query has zero contiguous hits, the tool retries with every
//! word matched independently and labels the rows `~ token match` so the
//! weaker semantics are explicit. Single-token queries never double-search.

use crate::brain::tools::session_search::SessionSearchTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use crate::db::Database;
use crate::db::models::{Message, Session};
use crate::db::repository::{MessageRepository, SessionRepository};
use serde_json::json;
use uuid::Uuid;

async fn setup() -> (Database, SessionSearchTool) {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let tool = SessionSearchTool::new(db.pool().clone());
    (db, tool)
}

fn ctx() -> ToolExecutionContext {
    ToolExecutionContext::new(Uuid::new_v4())
}

/// Seed one session holding the given message bodies.
async fn seed(db: &Database, title: &str, bodies: &[&str]) {
    let srepo = SessionRepository::new(db.pool().clone());
    let mrepo = MessageRepository::new(db.pool().clone());
    let session = Session::new(Some(title.to_string()), Some("m".to_string()), None);
    srepo.create(&session).await.unwrap();
    for (i, body) in bodies.iter().enumerate() {
        mrepo.create(&Message::new(
            session.id,
            "user".into(),
            body.to_string(),
            i as i32 + 1,
        ))
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn verbatim_wins_when_present() {
    let (db, tool) = setup().await;
    seed(
        &db,
        "contiguous",
        &["the quick brown fox jumps over the lazy dog"],
    )
    .await;

    let result = tool
        .execute(
            json!({"operation": "search", "query": "quick brown fox"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(result.success, "error: {:?}", result.error);
    assert!(
        result.output.contains("contiguous"),
        "verbatim hit expected, got: {}",
        result.output
    );
    assert!(
        !result.output.contains("~ token match"),
        "verbatim path must not carry the fallback banner: {}",
        result.output
    );
}

#[tokio::test]
async fn fallback_fires_on_zero_verbatim_multiword() {
    let (db, tool) = setup().await;
    // Same words, scattered: no contiguous "quick brown fox" anywhere.
    seed(&db, "scattered", &["the fox was quick and the sky brown"]).await;

    let result = tool
        .execute(
            json!({"operation": "search", "query": "quick brown fox"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(result.success, "error: {:?}", result.error);
    assert!(
        result.output.contains("~ token match"),
        "tokenized fallback banner expected, got: {}",
        result.output
    );
    assert!(
        result.output.contains("scattered"),
        "tokenized row expected, got: {}",
        result.output
    );
}

#[tokio::test]
async fn single_token_miss_returns_plain_no_match() {
    let (db, tool) = setup().await;
    seed(&db, "other", &["nothing relevant here"]).await;

    let result = tool
        .execute(json!({"operation": "search", "query": "zebra"}), &ctx())
        .await
        .unwrap();
    assert!(result.success);
    assert!(
        result.output.contains("No messages found matching"),
        "plain no-match expected (single token must not double-search), got: {}",
        result.output
    );
    assert!(!result.output.contains("~ token match"));
}

#[tokio::test]
async fn zero_zero_returns_empty() {
    let (db, tool) = setup().await;
    seed(&db, "unrelated", &["some unrelated content"]).await;

    let result = tool
        .execute(
            json!({"operation": "search", "query": "totally absent words"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(result.success);
    assert!(
        result.output.contains("No messages found matching"),
        "both passes empty -> plain no-match, got: {}",
        result.output
    );
    assert!(!result.output.contains("~ token match"));
}

#[tokio::test]
async fn and_semantics_requires_every_token() {
    let (db, tool) = setup().await;
    seed(&db, "partial", &["alpha appears, beta too, gamma nowhere"]).await;

    let result = tool
        .execute(
            json!({"operation": "search", "query": "zzz alpha beta"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(result.success, "error: {:?}", result.error);
    // "zzz" matches nothing, so verbatim AND tokenized both miss entirely —
    // AND-semantics must not degrade to OR.
    assert!(
        result.output.contains("No messages found matching"),
        "AND-match must require every token, got: {}",
        result.output
    );
}

#[tokio::test]
async fn tokenized_rows_rank_by_hit_count_then_recency() {
    let (db, tool) = setup().await;
    let srepo = SessionRepository::new(db.pool().clone());
    let mrepo = MessageRepository::new(db.pool().clone());

    // Sparse match (each token once) in the session pinned NEWER; dense
    // repeats in one pinned OLDER. Both satisfy the AND-match, neither
    // contains the contiguous phrase. Hit-count ranking must list the dense
    // row first even though recency alone would order the other way.
    let newer = Session::new(Some("sparse".to_string()), Some("m".to_string()), None);
    srepo.create(&newer).await.unwrap();
    mrepo
        .create(&Message::new(
            newer.id,
            "user".into(),
            "words: alpha once, beta once".to_string(),
            1,
        ))
        .await
        .unwrap();

    let older = Session::new(Some("dense".to_string()), Some("m".to_string()), None);
    srepo.create(&older).await.unwrap();
    mrepo
        .create(&Message::new(
            older.id,
            "user".into(),
            "alpha alpha beta beta and more alpha beta words".to_string(),
            1,
        ))
        .await
        .unwrap();

    // Pin recency: sparse strictly newer than dense.
    let mut pinned_older = older.clone();
    pinned_older.updated_at = chrono::Utc::now() - chrono::Duration::hours(1);
    srepo.update(&pinned_older).await.unwrap();
    let mut pinned_newer = newer.clone();
    pinned_newer.updated_at = chrono::Utc::now();
    srepo.update(&pinned_newer).await.unwrap();

    let result = tool
        .execute(
            json!({"operation": "search", "query": "words alpha beta"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(result.success, "error: {:?}", result.error);
    assert!(
        result.output.contains("~ token match"),
        "fallback expected, got: {}",
        result.output
    );
    let dense = result.output.find("dense").expect("dense row present");
    let sparse = result.output.find("sparse").expect("sparse row present");
    assert!(
        dense < sparse,
        "dense (more hits) must outrank sparse (newer): {}",
        result.output
    );
}
