//! Database connection management, pool configuration, and extension traits.

use anyhow::{Context, Result};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use std::path::Path;

/// Type alias for database pool
pub type Pool = SqlitePool;

/// Database connection manager
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Connect to a SQLite database file
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
        let url = format!("sqlite://{}?mode=rwc", path_str);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    // Set busy_timeout to 5 seconds to handle database locks
                    sqlx::query("PRAGMA busy_timeout = 5000")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .context("Failed to connect to database")?;

        tracing::info!("Connected to database: {} (busy_timeout: 5s)", path_str);
        Ok(Self { pool })
    }

    /// Connect to an in-memory database (for testing)
    pub async fn connect_in_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect("sqlite::memory:")
            .await
            .context("Failed to connect to in-memory database")?;

        tracing::debug!("Connected to in-memory database");
        Ok(Self { pool })
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Check if the database connection is still valid
    pub fn is_connected(&self) -> bool {
        !self.pool.is_closed()
    }

    /// Run database migrations
    pub async fn run_migrations(&self) -> Result<()> {
        sqlx::migrate!("./src/migrations")
            .run(&self.pool)
            .await
            .context("Failed to run database migrations")?;

        tracing::info!("Database migrations completed");
        Ok(())
    }

    /// Close the database connection
    pub async fn close(self) -> Result<()> {
        self.pool.close().await;
        tracing::info!("Database connection closed");
        Ok(())
    }
}

/// Extension trait for SqlitePool to add convenience methods
#[allow(async_fn_in_trait)]
pub trait PoolExt {
    /// Connect to a database file
    async fn connect_file<P: AsRef<Path>>(path: P) -> Result<Self>
    where
        Self: Sized;

    /// Connect to an in-memory database
    async fn connect_in_memory() -> Result<Self>
    where
        Self: Sized;

    /// Check if the pool is connected
    fn is_connected(&self) -> bool;
}

impl PoolExt for SqlitePool {
    async fn connect_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = Database::connect(path).await?;
        Ok(db.pool)
    }

    async fn connect_in_memory() -> Result<Self> {
        let db = Database::connect_in_memory().await?;
        Ok(db.pool)
    }

    fn is_connected(&self) -> bool {
        !self.is_closed()
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
    async fn test_pool_connect_in_memory() {
        let pool = Pool::connect_in_memory().await.unwrap();
        assert!(pool.is_connected());
    }
}
