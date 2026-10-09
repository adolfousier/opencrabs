//! Regression for #1457: pending requests must carry the origin forum topic.
//!
//! A `/rebuild` fired from a Telegram forum topic reported completion into
//! the chat's default topic (General) because the pending-request row had
//! nowhere to store the thread id — the channel layer parsed it and threw it
//! away at write time. The row now persists `channel_thread_id`, and the
//! rebuild tool formats it into the `telegram:chat:thread` deliver_to target
//! (grammar owned by PR #1451's `parse_telegram_target`).

use crate::db::Database;
use crate::db::repository::PendingRequestRepository;

#[tokio::test]
async fn pending_request_round_trips_origin_thread() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let id = uuid::Uuid::new_v4();
    let session_id = uuid::Uuid::new_v4();
    repo.insert(
        id,
        session_id,
        "rebuild from OC Dev topic",
        "telegram",
        Some("-1004428873948"),
        Some("249"),
        "user",
    )
    .await
    .unwrap();

    let row = repo
        .find_latest_for_session(session_id)
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(row.channel_chat_id.as_deref(), Some("-1004428873948"));
    assert_eq!(row.channel_thread_id.as_deref(), Some("249"));
}

#[tokio::test]
async fn legacy_row_without_thread_reads_back_none() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let id = uuid::Uuid::new_v4();
    let session_id = uuid::Uuid::new_v4();
    repo.insert(
        id,
        session_id,
        "DM turn",
        "telegram",
        Some("7711740248"),
        None,
        "user",
    )
    .await
    .unwrap();

    let row = repo
        .find_latest_for_session(session_id)
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(row.channel_thread_id, None, "non-topic turns stay NULL");
}

#[tokio::test]
async fn thread_survives_in_interrupted_scan() {
    // Boot recovery reads via get_interrupted — the thread must survive that
    // path too, or a resumed session loses its origin topic for later turns.
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let id = uuid::Uuid::new_v4();
    let session_id = uuid::Uuid::new_v4();
    repo.insert(
        id,
        session_id,
        "crashed mid-turn",
        "telegram",
        Some("-100"),
        Some("7198"),
        "user",
    )
    .await
    .unwrap();

    let rows = repo.get_interrupted().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].channel_thread_id.as_deref(), Some("7198"));
}

// ─────────────────────────────────────────────────────────────────────────────
// #2010: the row also carries the Telegram bubble the turn streams into, so a
// boot resume EDITS that bubble instead of opening a second one beside the
// partial answer the killed process left on screen.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn bubble_id_round_trips_through_the_setter() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let id = uuid::Uuid::new_v4();
    let session_id = uuid::Uuid::new_v4();
    repo.insert(
        id,
        session_id,
        "turn in a forum topic",
        "telegram",
        Some("-1004428873948"),
        Some("15"),
        "user",
    )
    .await
    .unwrap();

    // A row is born with no bubble: the streaming surface does not exist yet.
    let row = repo
        .find_latest_for_session(session_id)
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(
        row.channel_message_id, None,
        "no bubble until one is opened"
    );

    repo.set_channel_message_id_for_session(session_id, "flow:159")
        .await
        .unwrap();
    let row = repo
        .find_latest_for_session(session_id)
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(row.channel_message_id.as_deref(), Some("flow:159"));

    // Idempotent: rewriting the same surface is not an error (the write hook
    // re-checks per edit tick, but the row may be re-pointed, never doubled).
    repo.set_channel_message_id_for_session(session_id, "flow:159")
        .await
        .unwrap();

    // A surface move rewrites: the LAST live bubble is the adoptable one.
    repo.set_channel_message_id_for_session(session_id, "answer:161")
        .await
        .unwrap();
    let row = repo
        .find_latest_for_session(session_id)
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(row.channel_message_id.as_deref(), Some("answer:161"));

    // The boot path reads through get_interrupted — the id must survive there
    // too, or the resume that most needs it finds nothing.
    let rows = repo.get_interrupted().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].channel_message_id.as_deref(), Some("answer:161"));
}

#[tokio::test]
async fn legacy_row_without_bubble_reads_back_none() {
    // NULL by design for every pre-#2010 row and for turns that never reached
    // a streaming bubble. Reading one must not error the boot resume.
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let id = uuid::Uuid::new_v4();
    let session_id = uuid::Uuid::new_v4();
    repo.insert(
        id,
        session_id,
        "tui turn, no bubble ever",
        "tui",
        None,
        None,
        "user",
    )
    .await
    .unwrap();

    let row = repo
        .find_latest_for_session(session_id)
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(row.channel_message_id, None);
    let rows = repo.get_interrupted().await.unwrap();
    assert_eq!(rows[0].channel_message_id, None);
}

#[test]
fn setter_does_not_touch_updated_at() {
    // updated_at drives the 24h crash-debris prune in get_interrupted; a
    // bubble write must not keep debris alive by refreshing it.
    let src = std::fs::read_to_string(format!(
        "{}/src/db/repository/pending_request.rs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("read db/repository/pending_request.rs");
    let fn_at = src
        .find("pub async fn set_channel_message_id_for_session")
        .expect("setter exists");
    let body = &src[fn_at..];
    assert!(
        body.contains("SET channel_message_id = ?2 WHERE session_id = ?1"),
        "the setter must write only the carried id"
    );
    let sql_at = body.find("conn.execute(").expect("setter SQL");
    assert!(
        !body[sql_at..].contains("updated_at"),
        "the setter must not refresh updated_at (it drives the 24h prune)"
    );
}
