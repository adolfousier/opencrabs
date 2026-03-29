//! Database connection management, pool configuration, and extension traits.

use anyhow::{Context, Result};
use deadpool_sqlite::{Config, Hook, InteractError, Pool as DeadPool, Runtime};
use rusqlite_migration::{M, Migrations};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Flag set when the startup integrity check detects corruption.
static DB_INTEGRITY_FAILED: AtomicBool = AtomicBool::new(false);

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

/// Database connection manager
pub struct Database {
    pool: Pool,
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
        Ok(Self { pool })
    }

    /// Connect to an in-memory database (for testing)
    ///
    /// Each call creates a uniquely-named shared in-memory database so that
    /// parallel tests never collide, while all connections *within* a single
    /// test still see the same data.
    pub async fn connect_in_memory() -> Result<Self> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let uri = format!("file:mem_{}?mode=memory&cache=shared", id);
        let pool = Config::new(uri)
            .builder(Runtime::Tokio1)
            .context("Failed to build pool config")?
            .max_size(5)
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

    /// Total number of migrations defined below — keep in sync when adding new ones.
    const MIGRATION_COUNT: usize = 11;

    /// Run database migrations
    pub async fn run_migrations(&self) -> Result<()> {
        let migrations = Migrations::new(vec![
            M::up(include_str!(
                "../migrations/20251028000001_initial_schema.sql"
            )),
            M::up(include_str!(
                "../migrations/20251028000002_modernize_schema.sql"
            )),
            M::up(include_str!("../migrations/20251111000001_add_plans.sql")),
            M::up(include_str!(
                "../migrations/20251113000001_add_plan_enhancements.sql"
            )),
            M::up(include_str!(
                "../migrations/20260224000001_add_a2a_tasks.sql"
            )),
            M::up(include_str!(
                "../migrations/20260226000001_add_session_provider.sql"
            )),
            M::up(include_str!(
                "../migrations/20260305000001_add_channel_messages.sql"
            )),
            M::up(include_str!(
                "../migrations/20260305000002_add_cron_jobs.sql"
            )),
            M::up(include_str!(
                "../migrations/20260306000001_add_usage_ledger.sql"
            )),
            M::up(include_str!(
                "../migrations/20260307000001_add_session_working_dir.sql"
            )),
            M::up(include_str!(
                "../migrations/20260308000001_add_pending_requests.sql"
            )),
        ]);

        self.pool
            .get()
            .await
            .context("Failed to get connection for migrations")?
            .interact(move |conn| {
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

                migrations.to_latest(conn)
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect_in_memory() {
        let db = Database::connect_in_memory().await.unwrap();
        assert!(db.is_connected());
    }

    #[tokio::test]
    async fn test_migrations() {
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
    }

    #[tokio::test]
    async fn test_migrations_idempotent() {
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        // Running migrations a second time should be a no-op
        db.run_migrations().await.unwrap();

        let version: i64 = db
            .pool
            .get()
            .await
            .unwrap()
            .interact(|conn| conn.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(version, Database::MIGRATION_COUNT as i64);
    }

    /// Simulate upgrading from an sqlx-managed database: the _sqlx_migrations
    /// table exists, all tables are already present, and user_version is 0.
    /// run_migrations() must detect this and stamp the version without failing.
    #[tokio::test]
    async fn test_sqlx_upgrade_stamps_user_version() {
        let db = Database::connect_in_memory().await.unwrap();

        // 1. Run migrations normally to create all tables
        db.run_migrations().await.unwrap();

        // 2. Simulate a pre-existing sqlx DB: add _sqlx_migrations and reset user_version
        db.pool
            .get()
            .await
            .unwrap()
            .interact(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS _sqlx_migrations (
                        version INTEGER PRIMARY KEY,
                        description TEXT NOT NULL,
                        installed_on TEXT NOT NULL DEFAULT (datetime('now'))
                    );
                    PRAGMA user_version = 0;",
                )
            })
            .await
            .unwrap()
            .unwrap();

        // 3. run_migrations should detect sqlx and stamp, not fail
        db.run_migrations().await.unwrap();

        // 4. Verify user_version was set to MIGRATION_COUNT
        let version: i64 = db
            .pool
            .get()
            .await
            .unwrap()
            .interact(|conn| conn.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(version, Database::MIGRATION_COUNT as i64);
    }

    /// Fresh database (no _sqlx_migrations, user_version=0) should run all
    /// migrations normally and end at MIGRATION_COUNT.
    #[tokio::test]
    async fn test_fresh_db_runs_all_migrations() {
        let db = Database::connect_in_memory().await.unwrap();

        // Verify starts at 0
        let before: i64 = db
            .pool
            .get()
            .await
            .unwrap()
            .interact(|conn| conn.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before, 0);

        db.run_migrations().await.unwrap();

        let after: i64 = db
            .pool
            .get()
            .await
            .unwrap()
            .interact(|conn| conn.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after, Database::MIGRATION_COUNT as i64);
    }
}
