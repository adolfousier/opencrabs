//! Database connection management, pool configuration, and extension traits.

use anyhow::{Context, Result};
use deadpool_sqlite::{Config, Hook, InteractError, Pool as DeadPool, Runtime};
use rusqlite_migration::{M, Migrations};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

/// Flag set when the startup integrity check detects corruption.
static DB_INTEGRITY_FAILED: AtomicBool = AtomicBool::new(false);

/// Global pool handle set once on startup. Used by components that can't
/// otherwise receive a pool via dependency injection (e.g. CLI providers
/// that need to persist streaming progress directly to the DB without
/// waiting for tool_loop to batch-write at end of iteration).
static GLOBAL_POOL: OnceLock<Pool> = OnceLock::new();

/// Get the global DB pool if it has been set. Returns `None` before the
/// first `Database::connect` call (e.g. in unit tests).
pub fn global_pool() -> Option<&'static Pool> {
    GLOBAL_POOL.get()
}

/// Returns true (once) if the last startup integrity check detected corruption.
pub fn db_integrity_failed() -> bool {
    DB_INTEGRITY_FAILED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Type alias for database pool
pub type Pool = DeadPool;

/// Map deadpool InteractError to anyhow
pub fn interact_err(e: InteractError) -> anyhow::Error {
    anyhow::anyhow!("Database interact error: {}", e)
}

/// Build the full ordered migration list.
///
/// Extracted from [`Database::run_migrations`] so tests can prepare a
/// database at a specific version (e.g. `to_version(32)`) before exercising
/// the migration runner. Order and content are the migration contract:
/// never reorder or edit applied entries, only append new ones.
/// Every migration, oldest first. Adding one is adding its `include_str!`
/// here and nothing else: [`Database::MIGRATION_COUNT`] is this list's
/// length, so the count cannot drift from the list (#1354, after #1292).
pub(crate) const MIGRATION_SQL: &[&str] = &[
    include_str!("../migrations/20251028000001_initial_schema.sql"),
    include_str!("../migrations/20251028000002_modernize_schema.sql"),
    include_str!("../migrations/20251111000001_add_plans.sql"),
    include_str!("../migrations/20251113000001_add_plan_enhancements.sql"),
    include_str!("../migrations/20260224000001_add_a2a_tasks.sql"),
    include_str!("../migrations/20260226000001_add_session_provider.sql"),
    include_str!("../migrations/20260305000001_add_channel_messages.sql"),
    include_str!("../migrations/20260305000002_add_cron_jobs.sql"),
    include_str!("../migrations/20260306000001_add_usage_ledger.sql"),
    include_str!("../migrations/20260307000001_add_session_working_dir.sql"),
    include_str!("../migrations/20260308000001_add_pending_requests.sql"),
    include_str!("../migrations/20260330000001_pending_requests_channel_chat_id.sql"),
    include_str!("../migrations/20260402000001_add_cron_job_runs.sql"),
    include_str!("../migrations/20260412000001_add_feedback_ledger.sql"),
    include_str!("../migrations/20260415000001_add_tool_executions.sql"),
    include_str!("../migrations/20260415000002_add_session_category.sql"),
    include_str!("../migrations/20260415000003_fix_tool_executions_schema.sql"),
    include_str!("../migrations/20260416000001_add_message_input_tokens.sql"),
    include_str!("../migrations/20260421000001_add_message_thinking.sql"),
    include_str!("../migrations/20260426000001_add_recent_paths.sql"),
    include_str!("../migrations/20260507000001_add_cron_deliver_api_key.sql"),
    include_str!("../migrations/20260517000001_cron_jobs_text_recast.sql"),
    include_str!("../migrations/20260522000001_add_auto_title_attempted.sql"),
    include_str!("../migrations/20260529000001_add_channel_thread_id.sql"),
    include_str!("../migrations/20260606000001_add_message_cache_tokens.sql"),
    include_str!("../migrations/20260608000001_add_cron_job_profile.sql"),
    include_str!("../migrations/20260614000001_add_projects_and_file_size.sql"),
    include_str!("../migrations/20260626000001_add_goal_state.sql"),
    include_str!("../migrations/20260706000001_add_usage_ledger_provider.sql"),
    include_str!("../migrations/20260713000001_drop_orphaned_plans_tables.sql"),
    include_str!("../migrations/20260725000001_add_background_tasks.sql"),
    include_str!("../migrations/20260726000001_add_plan_cards.sql"),
    include_str!("../migrations/20260731000001_add_analytics_events.sql"),
    include_str!("../migrations/20260808000001_add_message_duration.sql"),
    include_str!("../migrations/20260822000001_add_a2a_context_sessions.sql"),
    include_str!("../migrations/20260826000001_add_session_bindings.sql"),
    include_str!("../migrations/20260828000001_pending_requests_origin.sql"),
    include_str!("../migrations/20260902000001_add_pending_followups.sql"),
    include_str!("../migrations/20260904000000_add_pending_followups_host_markdown.sql"),
];

pub(crate) fn build_migrations() -> Migrations<'static> {
    Migrations::new(MIGRATION_SQL.iter().copied().map(M::up).collect())
}

/// Heal a database that partially carries migration 33's artifacts (#937).
///
/// If any of the three `tool_executions` columns added by migration 33
/// already exist, add whichever are missing and make sure the analytics
/// tables and indexes exist, then return `true` so the caller stamps
/// `user_version` to 33. When nothing pre-exists returns `false` and the
/// normal migration runs as-is.
///
/// The schema below mirrors `src/migrations/20260731000001_add_analytics_events.sql`,
/// which stays the source of truth — keep both in sync.
pub(crate) fn heal_analytics_migration_33(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    let cols: std::collections::HashSet<String> = conn
        .prepare("PRAGMA table_info(tool_executions)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .collect();

    let pre_existing = ["provider", "model", "duration_ms"]
        .iter()
        .any(|c| cols.contains(*c));
    if !pre_existing {
        return Ok(false);
    }

    if !cols.contains("provider") {
        conn.execute_batch("ALTER TABLE tool_executions ADD COLUMN provider TEXT;")?;
    }
    if !cols.contains("model") {
        conn.execute_batch("ALTER TABLE tool_executions ADD COLUMN model TEXT;")?;
    }
    if !cols.contains("duration_ms") {
        conn.execute_batch("ALTER TABLE tool_executions ADD COLUMN duration_ms INTEGER;")?;
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS phantom_events (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL DEFAULT '',
            provider TEXT,
            model TEXT,
            detected_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            resolved INTEGER NOT NULL DEFAULT 0,
            retry_count INTEGER NOT NULL DEFAULT 0,
            tools_after_retry INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_phantom_events_detected_at ON phantom_events(detected_at);
        CREATE INDEX IF NOT EXISTS idx_phantom_events_session ON phantom_events(session_id);
        CREATE TABLE IF NOT EXISTS streaming_recoveries (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL DEFAULT '',
            provider TEXT,
            model TEXT,
            recovered_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            tool_count INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_streaming_recoveries_at ON streaming_recoveries(recovered_at);
        CREATE TABLE IF NOT EXISTS brain_verify_events (
            id TEXT PRIMARY KEY,
            file_name TEXT NOT NULL,
            event_type TEXT NOT NULL DEFAULT 'pass',
            violations TEXT,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        CREATE INDEX IF NOT EXISTS idx_brain_verify_events_at ON brain_verify_events(created_at);",
    )?;

    tracing::info!(
        "Healed partial migration 33 state in tool_executions — analytics schema completed, advancing to version 33"
    );
    Ok(true)
}

/// Database connection manager
pub struct Database {
    pub(crate) pool: Pool,
}

/// Apply PRAGMA settings to a rusqlite connection.
///
/// WAL mode, busy timeout, synchronous NORMAL, 64 MB page cache.
fn apply_pragmas(conn: &rusqlite::Connection) -> std::result::Result<(), rusqlite::Error> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 30000;
         PRAGMA synchronous = NORMAL;
         PRAGMA cache_size = -65536;",
    )
}

/// PRAGMA settings for in-memory test databases.
///
/// WAL is a no-op (no file to journal to) but with `cache=shared` it triggers
/// spurious `database table is locked` (SQLITE_LOCKED, code 262) failures
/// under concurrent writers in release mode — the auto_title_e2e_test reliably
/// fell over here. MEMORY journal is the right mode for `:memory:` and shared
/// in-memory URIs; foreign keys stay on so FK violations still surface.
fn apply_pragmas_in_memory(
    conn: &rusqlite::Connection,
) -> std::result::Result<(), rusqlite::Error> {
    conn.execute_batch(
        "PRAGMA journal_mode = MEMORY;
         PRAGMA busy_timeout = 30000;
         PRAGMA synchronous = OFF;
         PRAGMA foreign_keys = ON;",
    )
}

impl Database {
    /// Connect to a SQLite database file.
    ///
    /// Pool is tuned for concurrent access:
    /// - WAL journal mode: readers never block on writers (eliminates the
    ///   "slow statement" timeouts seen under heavy TUI load)
    /// - 16 connections: enough headroom for TUI + all channel handlers
    /// - 30 s busy_timeout: graceful queuing instead of fast-fail on contention
    /// - synchronous = NORMAL: safe with WAL, ~3× faster writes than FULL
    pub async fn connect<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            tracing::debug!("Creating database directory: {:?}", parent);
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create database directory: {:?}", parent))?;
        }

        let path_str = path.to_string_lossy().into_owned();

        let pool = Config::new(&path_str)
            .builder(Runtime::Tokio1)
            .context("Failed to build pool config")?
            .max_size(16)
            .post_create(Hook::async_fn(|conn, _| {
                Box::pin(async move {
                    conn.interact(|conn| apply_pragmas(conn))
                        .await
                        .map_err(|e| deadpool_sqlite::HookError::Message(e.to_string().into()))?
                        .map_err(|e| deadpool_sqlite::HookError::Message(e.to_string().into()))?;
                    Ok(())
                })
            }))
            .build()
            .context("Failed to create connection pool")?;

        tracing::info!(
            "Connected to database: {} (WAL, pool=16, busy_timeout=30s)",
            path_str
        );
        // Publish pool globally so components without DI access (e.g. qwen-cli
        // provider streaming persistence) can still write to the DB. Safe to
        // ignore the error — only the first connect wins.
        let _ = GLOBAL_POOL.set(pool.clone());
        Ok(Self { pool })
    }

    /// Connect to an in-memory database (for testing)
    ///
    /// Pool is capped to `max_size = 1` — a single serialized connection.
    /// This is critical for correctness, not just isolation: SQLite's
    /// in-memory `cache=shared` mode handles cross-connection locking
    /// erratically (release-mode regularly hit `SQLITE_LOCKED` code 262
    /// with WAL pragma, and `MEMORY` journal mode shifted the symptom
    /// without removing it). With one connection there is no
    /// cross-connection lock contention, every interact() runs
    /// sequentially, and tests behave identically in debug and release.
    ///
    /// Each call still uses a unique URI so parallel tests never see
    /// each other's data.
    pub async fn connect_in_memory() -> Result<Self> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let uri = format!("file:mem_{}?mode=memory&cache=shared", id);
        let pool = Config::new(uri)
            .builder(Runtime::Tokio1)
            .context("Failed to build pool config")?
            .max_size(1)
            .post_create(Hook::async_fn(|conn, _| {
                Box::pin(async move {
                    conn.interact(|conn| apply_pragmas_in_memory(conn))
                        .await
                        .map_err(|e| deadpool_sqlite::HookError::Message(e.to_string().into()))?
                        .map_err(|e| deadpool_sqlite::HookError::Message(e.to_string().into()))?;
                    Ok(())
                })
            }))
            .build()
            .context("Failed to create in-memory pool")?;

        tracing::debug!("Connected to in-memory database");
        Ok(Self { pool })
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Check if the database connection is still valid
    pub fn is_connected(&self) -> bool {
        self.pool.status().size > 0 || self.pool.status().max_size > 0
    }

    /// Total number of migrations, derived from `MIGRATION_SQL` so it can
    /// never lag behind the list.
    pub const MIGRATION_COUNT: usize = MIGRATION_SQL.len();

    /// Run database migrations
    pub async fn run_migrations(&self) -> Result<()> {
        let migrations = build_migrations();

        self.pool
            .get()
            .await
            .context("Failed to get connection for migrations")?
            .interact(
                move |conn| -> std::result::Result<(), rusqlite_migration::Error> {
                    // Detect databases previously managed by sqlx: if the _sqlx_migrations
                    // table exists but rusqlite_migration hasn't run yet (user_version == 0),
                    // stamp the current version so we don't re-run already-applied migrations.
                    let user_version: i64 =
                        conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
                    let has_sqlx: bool = conn
                        .prepare(
                            "SELECT COUNT(*) FROM sqlite_master \
                         WHERE type='table' AND name='_sqlx_migrations'",
                        )?
                        .query_row([], |r| r.get::<_, i64>(0))
                        .map(|c| c > 0)?;

                    if has_sqlx && user_version == 0 {
                        tracing::info!(
                            "Detected sqlx-managed database — stamping migration version to {}",
                            Self::MIGRATION_COUNT
                        );
                        conn.pragma_update(None, "user_version", Self::MIGRATION_COUNT as i64)?;
                    }

                    // Defensive heal for migration 33 (add_analytics_events), #937:
                    // databases that already carry some of migration 33's artifacts
                    // (e.g. a provider column added by an intermediate build) would
                    // crash the migration's ALTER TABLE with "duplicate column name".
                    // Only fires when migration 33 is the NEXT migration to run
                    // (user_version == 32) so it can never skip migrations 1-32.
                    // Heals the schema to the full migration-33 state and stamps
                    // user_version so to_latest skips it; when nothing pre-exists
                    // the normal migration runs untouched.
                    if user_version == 32 && heal_analytics_migration_33(conn)? {
                        conn.pragma_update(None, "user_version", 33i64)?;
                    }

                    migrations.to_latest(conn)?;

                    // Schema-checked heals run AFTER the stamp-driven pass: a
                    // migration that two branches appended at the same index can
                    // be skipped with the stamp already past it (#1401), and only
                    // the schema itself can say so.
                    crate::db::migration_heal::heal_pending_requests_origin(conn)?;
                    Ok(())
                },
            )
            .await
            .map_err(interact_err)?
            .context("Failed to run database migrations")?;

        tracing::info!("Database migrations completed");

        // Run integrity check on startup
        let integrity_ok = self
            .pool
            .get()
            .await
            .context("Failed to get connection for integrity check")?
            .interact(|conn| -> rusqlite::Result<bool> {
                let result: String =
                    conn.pragma_query_value(None, "integrity_check", |r| r.get(0))?;
                Ok(result == "ok")
            })
            .await
            .map_err(interact_err)?
            .context("Failed to run integrity check")?;

        if !integrity_ok {
            tracing::error!(
                "Database integrity check FAILED — data may be corrupted. \
                 Consider backing up and recreating the database."
            );
            DB_INTEGRITY_FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
        } else {
            tracing::debug!("Database integrity check passed");
        }

        Ok(())
    }

    /// Close the database connection pool
    pub fn close(&self) {
        self.pool.close();
        tracing::info!("Database connection closed");
    }

    /// Get the current database user_version (migration level).
    ///
    /// This is used by the evolve tool to check if the current binary
    /// can handle the database before swapping.
    pub async fn get_user_version(&self) -> Result<i64> {
        let version = self
            .pool
            .get()
            .await
            .context("Failed to get connection for user_version")?
            .interact(|conn| conn.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .map_err(interact_err)?
            .context("Failed to read user_version")?;
        Ok(version)
    }
}

/// Extension trait for Pool convenience methods
pub trait PoolExt {
    fn is_connected(&self) -> bool;
}

impl PoolExt for Pool {
    fn is_connected(&self) -> bool {
        self.status().size > 0 || self.status().max_size > 0
    }
}
