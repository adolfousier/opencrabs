//! The age reap must run AFTER the redelivery pass, never before (#1617).
//!
//! `reap_stale_unclaimed` deletes by `created_at` alone: its SQL carries no
//! claim or route predicate, so "unclaimed" is true only because the caller
//! has already offered every row to a live route and kept what re-parked.
//! Run before the offer, the same query eats live pushes after any long
//! downtime — a sleeping laptop, a machine off over a holiday — which is the
//! one outcome this module forbids ("a DUPLICATE after restart, never a lost
//! message", `notify_queue.rs` module doc).
//!
//! Two layers pin it. The data layer proves `all()` is age-blind, so nothing
//! below the caller protects an old-but-live row. The source sentinel proves
//! the call order inside `redeliver_persisted`, which cannot be exercised
//! directly: it resolves its pool through the process-wide `global_pool()`
//! `OnceLock` that only a real `Database::connect` sets.

use crate::db::Database;
use crate::db::repository::NotifyQueueRepository;
use uuid::Uuid;

const SERVICE_SRC: &str = include_str!("../brain/agent/service/notify_queue.rs");

async fn setup() -> (NotifyQueueRepository, Database) {
    let db = Database::connect_in_memory()
        .await
        .expect("in-memory database");
    db.run_migrations().await.expect("migrations");
    (NotifyQueueRepository::new(db.pool().clone()), db)
}

#[tokio::test]
async fn redelivery_read_is_age_blind_so_only_call_order_protects_an_old_live_row() {
    let (repo, db) = setup().await;
    let now = 1_700_000_000i64;
    let session = Uuid::new_v4();
    let id = Uuid::new_v4();
    let sess_str = session.to_string();
    let id_str = id.to_string();

    // A push parked 100h ago for a session whose route comes back at boot:
    // the machine was simply off for four days.
    raw(&db, move |conn| {
        conn.execute(
            "INSERT INTO notify_queue \
             (id, session_id, context_text, display_text, origin, bg_meta, created_at) \
             VALUES (?1, ?2, 'four days parked', 'four days parked', 'session_notify', NULL, ?3)",
            rusqlite::params![id_str, sess_str, now - 100 * 3600],
        )
    })
    .await;

    // The redelivery pass reads through `all()`, which applies no age filter:
    // the row is still offered no matter how old it is. That is the whole
    // reason the reap has to wait until after the offer.
    let offered = repo.all().await.expect("all");
    assert_eq!(
        offered.len(),
        1,
        "all() must surface the 100h row for redelivery, it is age-blind by design"
    );
    assert_eq!(offered[0].id, id);

    // Only once that offer has happened does reaping it become correct.
    let reaped = repo
        .reap_stale_unclaimed(now - 72 * 3600)
        .await
        .expect("reap");
    assert_eq!(reaped.len(), 1, "after the offer, the same row is reapable");
    assert_eq!(reaped[0].id, id);
}

#[test]
fn reap_call_sits_after_the_deliver_or_park_loop() {
    let deliver = SERVICE_SRC
        .find("deliver_or_park(row.session_id, msg)")
        .expect("redeliver_persisted must still offer rows through deliver_or_park");
    let reap = SERVICE_SRC
        .find("reap_stale_after_redelivery(&repo).await")
        .expect("redeliver_persisted must still call the age reap");
    assert!(
        reap > deliver,
        "the age reap must be called AFTER the redelivery loop: moved before it, \
         a 72h+ row for a live session is deleted instead of delivered"
    );
}

#[test]
fn dead_session_reap_stays_before_the_loop() {
    // The other reaper is safe to run first and belongs first: a row whose
    // session is gone can never be claimed, so offering it only re-parks a
    // push nobody can receive.
    let dead = SERVICE_SRC
        .find("repo.clear_dead_sessions().await")
        .expect("redeliver_persisted must still reap rows for dead sessions");
    let read = SERVICE_SRC
        .find("repo.all().await")
        .expect("redeliver_persisted must still read the rows it offers");
    assert!(
        dead < read,
        "the dead-session reap belongs before the read: its rows are unclaimable"
    );
}

async fn raw<F>(db: &Database, f: F)
where
    F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<usize> + Send + 'static,
{
    db.pool()
        .get()
        .await
        .expect("pool connection")
        .interact(move |conn| f(conn))
        .await
        .expect("interact")
        .expect("raw sql");
}
