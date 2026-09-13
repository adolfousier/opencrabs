//! Repair a schema that the version stamp says is complete but is not (#1401).
//!
//! `rusqlite_migration` applies migrations by list INDEX. When two branches
//! each append "migration N" and the merge re-sorts the list by date, any
//! database that was stamped N by the first branch's build skips the other
//! branch's migration forever: the stamp says done, so `to_latest` never
//! looks at it. That is how `pending_requests.origin` (migration 37) went
//! missing on a database stamped 38, and with it every restart-recovery row
//! for three days: the INSERT failed on every turn and boot found nothing to
//! resume.
//!
//! A heal is idempotent and checks the schema, not the stamp. It runs after
//! `to_latest` on every boot, so a healed database stays healed and a
//! correct one is untouched.

/// Does `table` carry a column named `column`?
pub(crate) fn has_column(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
) -> rusqlite::Result<bool> {
    let cols: Vec<String> = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .collect();
    Ok(cols.iter().any(|c| c == column))
}

fn has_table(conn: &rusqlite::Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
}

/// Create `notify_queue` when its migration was skipped.
///
/// The #111 migration carries an earlier filename date than
/// `20260908000001_pending_requests_thread_id.sql`, so merging it in name
/// order places it at an index that databases stamped by the newer build have
/// already passed — `to_latest` never looks at it and the durable notify queue
/// silently does not exist. Mirrors
/// `src/migrations/20260906000001_add_notify_queue.sql`, which stays the source
/// of truth. Returns `true` when it changed the schema.
pub(crate) fn heal_notify_queue(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    if has_table(conn, "notify_queue")? {
        return Ok(false);
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS notify_queue (
            id           TEXT PRIMARY KEY NOT NULL,
            session_id   TEXT NOT NULL,
            context_text TEXT NOT NULL,
            display_text TEXT NOT NULL,
            origin       TEXT NOT NULL,
            bg_meta      TEXT,
            created_at   INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_notify_queue_session ON notify_queue(session_id);",
    )?;
    tracing::warn!(
        "Healed notify_queue: the #111 table was missing although the schema was stamped past \
         its migration index (#1401). Parked pushes could not survive a restart until now."
    );
    Ok(true)
}

/// Index of the `pending_requests_thread_id` migration in the current list.
///
/// Kept next to the guard that uses it so the two cannot drift: if the list is
/// re-sorted again this constant is what has to move.
const THREAD_ID_MIGRATION_INDEX: i64 = 41;

/// Stamp past the `channel_thread_id` migration when its column is already
/// there, so `to_latest` does not re-run an ALTER that has already happened.
///
/// The mirror image of the misses this module was written for. A database that
/// ran that migration while it sat at index 40 was stamped 40. Merging
/// `20260906000001_add_notify_queue.sql` re-sorted the list by filename date
/// and pushed the thread-id migration to 41, so `to_latest` on a database
/// stamped 40 now replays it and SQLite rejects the duplicate column, which
/// fails the whole startup rather than any one migration.
///
/// The other heals in this module repair a migration that was skipped and can
/// run afterwards. This one has to run BEFORE `to_latest`, because the crash it
/// prevents happens inside it.
///
/// Only fires when the stamp is exactly one behind and the column is genuinely
/// present, so it can never skip an unapplied migration. `notify_queue`, the
/// migration this database really is missing, is created by
/// [`heal_notify_queue`] on the pass that follows.
pub(crate) fn skip_applied_thread_id_migration(
    conn: &rusqlite::Connection,
    user_version: i64,
) -> rusqlite::Result<bool> {
    if user_version != THREAD_ID_MIGRATION_INDEX - 1 {
        return Ok(false);
    }
    if !has_column(conn, "pending_requests", "channel_thread_id")? {
        return Ok(false);
    }
    conn.pragma_update(None, "user_version", THREAD_ID_MIGRATION_INDEX)?;
    tracing::warn!(
        "Stamped past the pending_requests.channel_thread_id migration: the column was already \
         present at version {user_version}, so replaying it would have failed startup on a \
         duplicate column (#1401)."
    );
    Ok(true)
}

/// Add `pending_requests.origin` when migration 37 was skipped.
///
/// Mirrors `src/migrations/20260828000001_pending_requests_origin.sql`, which
/// stays the source of truth. Returns `true` when it changed the schema.
pub(crate) fn heal_pending_requests_origin(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    if !has_table(conn, "pending_requests")? || has_column(conn, "pending_requests", "origin")? {
        return Ok(false);
    }
    conn.execute_batch(
        "ALTER TABLE pending_requests ADD COLUMN origin TEXT NOT NULL DEFAULT 'user';",
    )?;
    tracing::warn!(
        "Healed pending_requests: the origin column of migration 37 was missing although the \
         schema was stamped past it (#1401). Restart recovery could not record turns until now."
    );
    Ok(true)
}

/// Add `session_seen_skills.loaded_mtime` when migration was skipped or partially applied (#210).
pub(crate) fn heal_session_seen_skills_loaded_mtime(
    conn: &rusqlite::Connection,
) -> rusqlite::Result<bool> {
    if !has_table(conn, "session_seen_skills")?
        || has_column(conn, "session_seen_skills", "loaded_mtime")?
    {
        return Ok(false);
    }
    conn.execute_batch("ALTER TABLE session_seen_skills ADD COLUMN loaded_mtime INTEGER;")?;
    tracing::warn!("Healed session_seen_skills: added missing loaded_mtime column (#210).");
    Ok(true)
}
