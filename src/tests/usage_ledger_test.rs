//! Usage Ledger Tests
//!
//! Tests for usage tracking, model name normalization, and stats aggregation.

use crate::db::Database;
use crate::db::repository::usage_ledger::{UsageLedgerRepository, normalize_model_name};

#[tokio::test]
async fn test_record_and_totals() {
    let db = Database::connect_in_memory()
        .await
        .expect("Failed to create database");
    db.run_migrations().await.expect("Failed to run migrations");
    let repo = UsageLedgerRepository::new(db.pool().clone());

    repo.record("s1", "sonnet-4-5", 100, 0.05).await.unwrap();
    repo.record("s1", "sonnet-4-5", 200, 0.10).await.unwrap();
    repo.record("s2", "opus-4-6", 500, 0.50).await.unwrap();

    let (tokens, cost) = repo.totals().await.unwrap();
    assert_eq!(tokens, 800);
    assert!((cost - 0.65).abs() < 0.001);
}

#[tokio::test]
async fn test_stats_by_model() {
    let db = Database::connect_in_memory()
        .await
        .expect("Failed to create database");
    db.run_migrations().await.expect("Failed to run migrations");
    let repo = UsageLedgerRepository::new(db.pool().clone());

    repo.record("s1", "sonnet", 100, 0.05).await.unwrap();
    repo.record("s2", "opus", 500, 0.50).await.unwrap();
    repo.record("s3", "sonnet", 200, 0.10).await.unwrap();

    let stats = repo.stats_by_model().await.unwrap();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].model, "opus");
    assert_eq!(stats[0].total_tokens, 500);
    assert_eq!(stats[1].model, "sonnet");
    assert_eq!(stats[1].total_tokens, 300);
}

#[tokio::test]
async fn test_stats_by_model_merges_claude_prefix() {
    let db = Database::connect_in_memory()
        .await
        .expect("Failed to create database");
    db.run_migrations().await.expect("Failed to run migrations");
    let repo = UsageLedgerRepository::new(db.pool().clone());

    repo.record("s1", "claude-opus-4-6", 1000, 1.0).await.unwrap();
    repo.record("s2", "opus-4-6", 500, 0.50).await.unwrap();
    repo.record("s3", "claude-sonnet-4-6", 200, 0.10).await.unwrap();

    let stats = repo.stats_by_model().await.unwrap();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].model, "opus-4-6");
    assert_eq!(stats[0].total_tokens, 1500);
    assert_eq!(stats[1].model, "sonnet-4-6");
    assert_eq!(stats[1].total_tokens, 200);
}

#[test]
fn test_normalize_model_name() {
    assert_eq!(normalize_model_name("claude-opus-4-6"), "opus-4-6");
    assert_eq!(normalize_model_name("claude-sonnet-4-6"), "sonnet-4-6");
    assert_eq!(normalize_model_name("opus-4-6"), "opus-4-6");
    assert_eq!(normalize_model_name("MiniMax-M2.5"), "MiniMax-M2.5");
}
