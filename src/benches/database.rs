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
use rusqlite::params;
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

                black_box({
                    let session = Session::new(Some("Test Session".to_string()), Some("claude-3-5-sonnet".to_string()), None);
                    db.pool()
                        .get()
                        .await
                        .unwrap()
                        .interact(move |conn| {
                            conn.execute(
                                "INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                                params![
                                    session.id.to_string(),
                                    session.title,
                                    session.model,
                                    session.created_at.timestamp(),
                                    session.updated_at.timestamp(),
                                    session.token_count,
                                    session.total_cost,
                                ],
                            )?;
                            Ok::<_, rusqlite::Error>(session.id)
                        })
                        .await
                        .unwrap()
                        .unwrap()
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

                // Create a session first
                let session = Session::new(Some("Test Session".to_string()), Some("claude-3-5-sonnet".to_string()), None);
                let sid = session.id.to_string();
                let s = session.clone();
                db.pool()
                    .get()
                    .await
                    .unwrap()
                    .interact(move |conn| {
                        conn.execute(
                            "INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![
                                s.id.to_string(), s.title, s.model,
                                s.created_at.timestamp(), s.updated_at.timestamp(),
                                s.token_count, s.total_cost,
                            ],
                        )
                    })
                    .await
                    .unwrap()
                    .unwrap();

                // Now benchmark retrieving it
                black_box({
                    db.pool()
                        .get()
                        .await
                        .unwrap()
                        .interact(move |conn| {
                            conn.prepare_cached("SELECT * FROM sessions WHERE id = ?1")?
                                .query_row(params![sid], Session::from_row)
                        })
                        .await
                        .unwrap()
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

                    // Create N sessions
                    for i in 0..count {
                        let session = Session::new(
                            Some(format!("Test Session {}", i)),
                            Some("claude-3-5-sonnet".to_string()),
                            None,
                        );
                        let s = session.clone();
                        db.pool()
                            .get()
                            .await
                            .unwrap()
                            .interact(move |conn| {
                                conn.execute(
                                    "INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                                    params![
                                        s.id.to_string(), s.title, s.model,
                                        s.created_at.timestamp(), s.updated_at.timestamp(),
                                        s.token_count, s.total_cost,
                                    ],
                                )
                            })
                            .await
                            .unwrap()
                            .unwrap();
                    }

                    // Benchmark listing them
                    black_box({
                        db.pool()
                            .get()
                            .await
                            .unwrap()
                            .interact(|conn| {
                                let mut stmt = conn.prepare_cached(
                                    "SELECT * FROM sessions ORDER BY created_at DESC",
                                )?;
                                let rows = stmt.query_map([], Session::from_row)?;
                                rows.collect::<std::result::Result<Vec<_>, _>>()
                            })
                            .await
                            .unwrap()
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

                // Create a session first
                let session = Session::new(Some("Test Session".to_string()), Some("claude-3-5-sonnet".to_string()), None);
                let s = session.clone();
                db.pool()
                    .get()
                    .await
                    .unwrap()
                    .interact(move |conn| {
                        conn.execute(
                            "INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![
                                s.id.to_string(), s.title, s.model,
                                s.created_at.timestamp(), s.updated_at.timestamp(),
                                s.token_count, s.total_cost,
                            ],
                        )
                    })
                    .await
                    .unwrap()
                    .unwrap();

                // Benchmark message insertion
                black_box({
                    let message_id = uuid::Uuid::new_v4();
                    let session_id = session.id.to_string();
                    let created_at = chrono::Utc::now().timestamp();

                    db.pool()
                        .get()
                        .await
                        .unwrap()
                        .interact(move |conn| {
                            conn.execute(
                                "INSERT INTO messages (id, session_id, role, content, sequence, created_at)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                params![
                                    message_id.to_string(),
                                    session_id,
                                    "user",
                                    "Hello, this is a test message",
                                    1i32,
                                    created_at,
                                ],
                            )?;
                            Ok::<_, rusqlite::Error>(message_id)
                        })
                        .await
                        .unwrap()
                        .unwrap()
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

                    // Create a session
                    let session = Session::new(Some("Test Session".to_string()), Some("claude-3-5-sonnet".to_string()), None);
                    let s = session.clone();
                    db.pool()
                        .get()
                        .await
                        .unwrap()
                        .interact(move |conn| {
                            conn.execute(
                                "INSERT INTO sessions (id, title, model, created_at, updated_at, token_count, total_cost)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                                params![
                                    s.id.to_string(), s.title, s.model,
                                    s.created_at.timestamp(), s.updated_at.timestamp(),
                                    s.token_count, s.total_cost,
                                ],
                            )
                        })
                        .await
                        .unwrap()
                        .unwrap();

                    // Insert N messages
                    for i in 0..count {
                        let session_id = session.id.to_string();
                        let message_id = uuid::Uuid::new_v4().to_string();
                        let role = if i % 2 == 0 { "user" } else { "assistant" };
                        let content = format!("Test message {}", i);
                        let sequence = i as i32;
                        let created_at = chrono::Utc::now().timestamp();

                        db.pool()
                            .get()
                            .await
                            .unwrap()
                            .interact(move |conn| {
                                conn.execute(
                                    "INSERT INTO messages (id, session_id, role, content, sequence, created_at)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                    params![message_id, session_id, role, content, sequence, created_at],
                                )
                            })
                            .await
                            .unwrap()
                            .unwrap();
                    }

                    // Benchmark querying all messages
                    let sid = session.id.to_string();
                    black_box({
                        db.pool()
                            .get()
                            .await
                            .unwrap()
                            .interact(move |conn| {
                                let mut stmt = conn.prepare_cached(
                                    "SELECT id, role, content, sequence FROM messages WHERE session_id = ?1 ORDER BY sequence ASC",
                                )?;
                                let rows = stmt.query_map(params![sid], |row| {
                                    Ok((
                                        row.get::<_, String>(0)?,
                                        row.get::<_, String>(1)?,
                                        row.get::<_, String>(2)?,
                                        row.get::<_, i32>(3)?,
                                    ))
                                })?;
                                rows.collect::<std::result::Result<Vec<_>, _>>()
                            })
                            .await
                            .unwrap()
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
