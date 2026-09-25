//! #1721 regression tests: stable chat identity on sessions.
//!
//! Two live sessions could claim the same channel chat because identity
//! lived only in the title text and nothing at the schema level enforced
//! uniqueness. The in-process single-flight gate (#1201/#1228) serializes
//! resolution inside one process; it cannot span processes (daemon + TUI or
//! multiple daemons on one profile database). These tests pin the storage
//! layer: the migration's dedup, the unique index, the atomic
//! insert-or-resolve path, and the 409 poll-conflict classifier.

use crate::channels::session_init::create_channel_session;
use crate::channels::telegram::raw_updates::{PollFailure, classify_poll_failure};
use crate::db::models::Session;
use crate::db::repository::SessionRepository;
use crate::db::{Database, database::build_migrations};
use crate::services::{ServiceContext, SessionService};
use serde_json::json;

// ─── fixtures ────────────────────────────────────────────────────────────

async fn fresh_repo() -> (Database, SessionRepository) {
    let db = Database::connect_in_memory()
        .await
        .expect("in-memory DB connect");
    db.run_migrations().await.expect("migrations");
    let repo = SessionRepository::new(db.pool().clone());
    (db, repo)
}

async fn fresh_service() -> (Database, SessionService) {
    let db = Database::connect_in_memory()
        .await
        .expect("in-memory DB connect");
    db.run_migrations().await.expect("migrations");
    let ctx = ServiceContext::new(db.pool().clone());
    let svc = SessionService::new(ctx);
    (db, svc)
}

// ─── migration: dedup + unique index ─────────────────────────────────────

#[test]
fn migration_archives_duplicate_losers_and_enforces_uniqueness() {
    let migrations = build_migrations();
    let mut conn = rusqlite::Connection::open_in_memory().expect("raw conn");
    // All migrations EXCEPT the new one, so the table is pre-#1721 shaped.
    let last = crate::db::database::MIGRATION_SQL.len() - 1;
    migrations
        .to_version(&mut conn, last)
        .expect("prefix apply");

    // Three live rows claiming the same chat (10 of 13 real keys looked
    // like this), plus one unrelated row that must not be touched.
    for (id, title, updated) in [
        (
            "11111111-1111-1111-1111-111111111111",
            "Telegram: Old [chat:-100]",
            1000,
        ),
        (
            "22222222-2222-2222-2222-222222222222",
            "Telegram: New [chat:-100]",
            3000,
        ),
        (
            "33333333-3333-3333-3333-333333333333",
            "Telegram: Mid [chat:-100]",
            2000,
        ),
        (
            "44444444-4444-4444-4444-444444444444",
            "Other chat [chat:-200]",
            500,
        ),
    ] {
        conn.execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, archived_at, token_count, \
             total_cost, auto_title_attempted) VALUES (?1, ?2, ?3, ?3, NULL, 0, 0.0, 0)",
            rusqlite::params![id, title, updated],
        )
        .expect("insert pre-migration row");
    }

    migrations.to_latest(&mut conn).expect("final apply");

    // The newest row per key survives live; older duplicates are archived.
    let live_newest: String = conn
        .query_row(
            "SELECT id FROM sessions WHERE channel_chat_key = '[chat:-100]' AND archived_at IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("one live winner");
    assert_eq!(live_newest, "22222222-2222-2222-2222-222222222222");

    let archived_dups: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE channel_chat_key = '[chat:-100]' \
             AND archived_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("archived count");
    assert_eq!(archived_dups, 2);

    let unrelated: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = '44444444-4444-4444-4444-444444444444' \
             AND archived_at IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("unrelated live");
    assert_eq!(unrelated, 1);

    // The index is live: a direct second INSERT for the key is refused.
    let dup = conn.execute(
        "INSERT INTO sessions (id, title, created_at, updated_at, archived_at, token_count, \
         total_cost, auto_title_attempted, channel_chat_key) \
         VALUES ('55555555-5555-5555-5555-555555555555', 'x [chat:-100]', 9, 9, NULL, 0, 0.0, 0, \
         '[chat:-100]')",
        [],
    );
    assert!(dup.is_err(), "unique index must refuse a second live row");
}

// ─── insert-or-resolve: the loser joins the winner ───────────────────────

#[tokio::test]
async fn insert_or_resolve_routes_second_creator_to_first_session() {
    let (_db, repo) = fresh_repo().await;
    let key = "[chat:-4242]";

    let first = repo
        .insert_or_resolve_channel(
            &Session::new(Some(format!("Telegram: First {key}")), None, None),
            key,
        )
        .await
        .expect("first insert wins");

    let second = repo
        .insert_or_resolve_channel(
            &Session::new(Some(format!("Telegram: Second {key}")), None, None),
            key,
        )
        .await
        .expect("second routes to winner");

    assert_eq!(second.id, first.id, "loser must join the winner's session");

    let live: i64 = {
        let conn = _db.pool().get().await.expect("conn");
        conn.interact(move |c| {
            c.query_row(
                "SELECT COUNT(*) FROM sessions WHERE channel_chat_key = ?1 \
                 AND archived_at IS NULL",
                [key],
                |r| r.get(0),
            )
        })
        .await
        .expect("interact")
        .expect("count")
    };
    assert_eq!(live, 1, "exactly one live session per chat key");
}

#[tokio::test]
async fn insert_or_resolve_persists_the_chat_key() {
    let (_db, repo) = fresh_repo().await;
    let key = "[chat:-77]";
    let s = repo
        .insert_or_resolve_channel(&Session::new(Some(format!("T {key}")), None, None), key)
        .await
        .expect("insert");
    assert_eq!(s.channel_chat_key.as_deref(), Some(key));
}

// ─── create_channel_session: suffix extraction + race routing ────────────

#[tokio::test]
async fn channel_session_creation_routes_through_chat_identity() {
    let (_db, svc) = fresh_service().await;
    let title = "Telegram: Group / Dev [chat:-900]";

    let a = create_channel_session(&svc, Some(title.to_string()), None)
        .await
        .expect("first create");
    let b = create_channel_session(&svc, Some(title.to_string()), None)
        .await
        .expect("second create routes to a");

    assert_eq!(a.id, b.id);
    assert_eq!(a.channel_chat_key.as_deref(), Some("[chat:-900]"));
}

#[tokio::test]
async fn channel_session_without_suffix_falls_back_to_plain_insert() {
    let (_db, svc) = fresh_service().await;
    let s = create_channel_session(&svc, Some("Untitled".to_string()), None)
        .await
        .expect("create");
    assert!(
        s.channel_chat_key.is_none(),
        "no suffix means no chat identity"
    );
}

// ─── 409 poll-conflict classifier ────────────────────────────────────────

#[test]
fn telegram_conflict_body_classifies_as_conflict() {
    let body = json!({
        "ok": false,
        "error_code": 409,
        "description": "Conflict: terminated by other getUpdates request; \
                        make sure that only one bot instance is running"
    });
    assert_eq!(classify_poll_failure(&body), PollFailure::Conflict);
}

#[test]
fn non_conflict_errors_stay_other() {
    let server_error = json!({"ok": false, "error_code": 500, "description": "Internal"});
    assert_eq!(classify_poll_failure(&server_error), PollFailure::Other);

    // 409 without the polling-conflict phrase is a different 409.
    let other_409 = json!({"ok": false, "error_code": 409, "description": "wrap into group"});
    assert_eq!(classify_poll_failure(&other_409), PollFailure::Other);

    let ok_body = json!({"ok": true, "result": []});
    assert_eq!(classify_poll_failure(&ok_body), PollFailure::Other);
}
