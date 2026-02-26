//! Database Performance Benchmarks
//!
//! Benchmarks for core database operations including:
//! - Session creation and retrieval
//! - Message insertion and querying
//! - Bulk operations
//! - Query performance

#![allow(clippy::all)]

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use opencrabs::db::{Database, models::Session};
use tempfile::TempDir;

/// Helper to create a test database in memory
async fn setup_test_db() -> (Database, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let db = Database::connect(&db_path).await.unwrap();
    db.run_migrations().await.unwrap();

    (db, temp_dir)
}

/// Benchmark: Create a new session
fn bench_session_create(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_create");
    group.bench_function("create", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let (db, _temp) = setup_test_db().await;
                let pool = db.pool();

                black_box({
                    let session = Session::new(Some("Test Session".to_string()), Some("claude-3-5-sonnet".to_string()));
                    sqlx::query(
                        r#"
                        INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                        VALUES (?, ?, ?, ?, ?, ?, ?)
                        "#
                    )
                    .bind(session.id.to_string())
                    .bind(&session.title)
                    .bind(&session.model)
                    .bind(session.created_at.timestamp())
                    .bind(session.updated_at.timestamp())
                    .bind(session.token_count)
                    .bind(session.total_cost)
                    .execute(pool)
                    .await
                    .unwrap();

                    session.id
                })
            })
        });
    });
    group.finish();
}

/// Benchmark: Query session by ID
fn bench_session_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_get");
    group.bench_function("get", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let (db, _temp) = setup_test_db().await;
                let pool = db.pool();

                // Create a session first
                let session = Session::new(Some("Test Session".to_string()), Some("claude-3-5-sonnet".to_string()));
                sqlx::query(
                    r#"
                    INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    "#
                )
                .bind(session.id.to_string())
                .bind(&session.title)
                .bind(&session.model)
                .bind(session.created_at.timestamp())
                .bind(session.updated_at.timestamp())
                .bind(session.token_count)
                .bind(session.total_cost)
                .execute(pool)
                .await
                .unwrap();

                // Now benchmark retrieving it
                black_box({
                    sqlx::query_as::<_, Session>(
                        r#"
                        SELECT id, title, model, created_at, updated_at, archived_at, token_count, total_cost
                        FROM sessions
                        WHERE id = ?
                        "#
                    )
                    .bind(session.id.to_string())
                    .fetch_one(pool)
                    .await
                    .unwrap()
                })
            })
        });
    });
    group.finish();
}

/// Benchmark: List all sessions
fn bench_session_list(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_list");

    for count in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    let (db, _temp) = setup_test_db().await;
                    let pool = db.pool();

                    // Create N sessions
                    for i in 0..count {
                        let session = Session::new(
                            Some(format!("Test Session {}", i)),
                            Some("claude-3-5-sonnet".to_string())
                        );
                        sqlx::query(
                            r#"
                            INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                            VALUES (?, ?, ?, ?, ?, ?, ?)
                            "#
                        )
                        .bind(session.id.to_string())
                        .bind(&session.title)
                        .bind(&session.model)
                        .bind(session.created_at.timestamp())
                        .bind(session.updated_at.timestamp())
                        .bind(session.token_count)
                        .bind(session.total_cost)
                        .execute(pool)
                        .await
                        .unwrap();
                    }

                    // Benchmark listing them
                    black_box({
                        sqlx::query_as::<_, Session>(
                            r#"
                            SELECT id, title, model, created_at, updated_at, archived_at, token_count, total_cost
                            FROM sessions
                            ORDER BY created_at DESC
                            "#
                        )
                        .fetch_all(pool)
                        .await
                        .unwrap()
                    })
                })
            });
        });
    }

    group.finish();
}

/// Benchmark: Insert message
fn bench_message_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_insert");
    group.bench_function("insert", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let (db, _temp) = setup_test_db().await;
                let pool = db.pool();

                // Create a session first
                let session = Session::new(Some("Test Session".to_string()), Some("claude-3-5-sonnet".to_string()));
                sqlx::query(
                    r#"
                    INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    "#
                )
                .bind(session.id.to_string())
                .bind(&session.title)
                .bind(&session.model)
                .bind(session.created_at.timestamp())
                .bind(session.updated_at.timestamp())
                .bind(session.token_count)
                .bind(session.total_cost)
                .execute(pool)
                .await
                .unwrap();

                // Benchmark message insertion
                black_box({
                    let message_id = uuid::Uuid::new_v4();
                    let role = "user";
                    let content = "Hello, this is a test message";
                    let sequence = 1i32;
                    let created_at = chrono::Utc::now();

                    sqlx::query(
                        r#"
                        INSERT INTO messages (id, session_id, role, content, sequence, created_at)
                        VALUES (?, ?, ?, ?, ?, ?)
                        "#
                    )
                    .bind(message_id.to_string())
                    .bind(session.id.to_string())
                    .bind(role)
                    .bind(content)
                    .bind(sequence)
                    .bind(created_at.timestamp())
                    .execute(pool)
                    .await
                    .unwrap();

                    message_id
                })
            })
        });
    });
    group.finish();
}

/// Benchmark: Query messages for a session
fn bench_message_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_query");

    for count in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    let (db, _temp) = setup_test_db().await;
                    let pool = db.pool();

                    // Create a session
                    let session = Session::new(Some("Test Session".to_string()), Some("claude-3-5-sonnet".to_string()));
                    sqlx::query(
                        r#"
                        INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                        VALUES (?, ?, ?, ?, ?, ?, ?)
                        "#
                    )
                    .bind(session.id.to_string())
                    .bind(&session.title)
                    .bind(&session.model)
                    .bind(session.created_at.timestamp())
                    .bind(session.updated_at.timestamp())
                    .bind(session.token_count)
                    .bind(session.total_cost)
                    .execute(pool)
                    .await
                    .unwrap();

                    // Insert N messages
                    for i in 0..count {
                        let message_id = uuid::Uuid::new_v4();
                        let role = if i % 2 == 0 { "user" } else { "assistant" };
                        let content = format!("Test message {}", i);
                        let sequence = i as i32;
                        let created_at = chrono::Utc::now();

                        sqlx::query(
                            r#"
                            INSERT INTO messages (id, session_id, role, content, sequence, created_at)
                            VALUES (?, ?, ?, ?, ?, ?)
                            "#
                        )
                        .bind(message_id.to_string())
                        .bind(session.id.to_string())
                        .bind(role)
                        .bind(&content)
                        .bind(sequence)
                        .bind(created_at.timestamp())
                        .execute(pool)
                        .await
                        .unwrap();
                    }

                    // Benchmark querying all messages
                    black_box({
                        #[derive(sqlx::FromRow)]
                        #[allow(dead_code)]
                        struct Message {
                            id: String,
                            role: String,
                            content: String,
                            sequence: i32,
                        }

                        sqlx::query_as::<_, Message>(
                            r#"
                            SELECT id, role, content, sequence
                            FROM messages
                            WHERE session_id = ?
                            ORDER BY sequence ASC
                            "#
                        )
                        .bind(session.id.to_string())
                        .fetch_all(pool)
                        .await
                        .unwrap()
                    })
                })
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_session_create,
    bench_session_get,
    bench_session_list,
    bench_message_insert,
    bench_message_query
);
criterion_main!(benches);
