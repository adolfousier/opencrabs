//! Database connection management, pool configuration, and extension traits.

use anyhow::{Context, Result};
use deadpool_sqlite::{Config, Hook, InteractError, Pool as DeadPool, Runtime};
use rusqlite_migration::{M, Migrations};
use std::path::Path;

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
        ]);

        self.pool
            .get()
            .await
            .context("Failed to get connection for migrations")?
            .interact(move |conn| migrations.to_latest(conn))
            .await
            .map_err(interact_err)?
            .context("Failed to run database migrations")?;

        tracing::info!("Database migrations completed");
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
}
