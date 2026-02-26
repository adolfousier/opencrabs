//! File Repository
//!
//! Database operations for file tracking.

use crate::db::models::File;
use anyhow::{Context, Result};
use sqlx::SqlitePool;
use std::path::Path;
use uuid::Uuid;

/// Repository for file operations
#[derive(Clone)]
pub struct FileRepository {
    pool: SqlitePool,
}

impl FileRepository {
    /// Create a new file repository
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Find file by ID
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<File>> {
        let file = sqlx::query_as::<_, File>("SELECT * FROM files WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .context("Failed to find file")?;

        Ok(file)
    }

    /// Find all files for a session
    pub async fn find_by_session(&self, session_id: Uuid) -> Result<Vec<File>> {
        let files = sqlx::query_as::<_, File>(
            "SELECT * FROM files WHERE session_id = ? ORDER BY created_at DESC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await
        .context("Failed to find files by session")?;

        Ok(files)
    }

    /// Find file by path in a session
    pub async fn find_by_path(&self, session_id: Uuid, path: &Path) -> Result<Option<File>> {
        let path_str = path.to_string_lossy();
        let file =
            sqlx::query_as::<_, File>("SELECT * FROM files WHERE session_id = ? AND path = ?")
                .bind(session_id.to_string())
                .bind(path_str.as_ref())
                .fetch_optional(&self.pool)
                .await
                .context("Failed to find file by path")?;

        Ok(file)
    }

    /// Create a new file record
    pub async fn create(&self, file: &File) -> Result<()> {
        let path_str = file.path.to_string_lossy();

        sqlx::query(
            r#"
            INSERT INTO files (id, session_id, path, content, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(file.id.to_string())
        .bind(file.session_id.to_string())
        .bind(path_str.as_ref())
        .bind(&file.content)
        .bind(file.created_at.timestamp())
        .bind(file.updated_at.timestamp())
        .execute(&self.pool)
        .await
        .context("Failed to create file record")?;

        tracing::debug!("Created file record: {} - {:?}", file.id, file.path);
        Ok(())
    }

    /// Update an existing file record
    pub async fn update(&self, file: &File) -> Result<()> {
        let path_str = file.path.to_string_lossy();

        sqlx::query(
            r#"
            UPDATE files
            SET path = ?, content = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(path_str.as_ref())
        .bind(&file.content)
        .bind(file.updated_at.timestamp())
        .bind(file.id.to_string())
        .execute(&self.pool)
        .await
        .context("Failed to update file")?;

        tracing::debug!("Updated file record: {}", file.id);
        Ok(())
    }

    /// Delete a file record
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM files WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to delete file")?;

        tracing::debug!("Deleted file record: {}", id);
        Ok(())
    }

    /// List all files for a session
    pub async fn list_by_session(&self, session_id: Uuid) -> Result<Vec<File>> {
        self.find_by_session(session_id).await
    }

    /// Count files in a session
    pub async fn count_by_session(&self, session_id: Uuid) -> Result<i64> {
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM files WHERE session_id = ?")
            .bind(session_id.to_string())
            .fetch_one(&self.pool)
            .await
            .context("Failed to count files")?;

        Ok(result.0)
    }

    /// Delete all file records for a session
    pub async fn delete_by_session(&self, session_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM files WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to delete session files")?;

        tracing::debug!("Deleted all file records for session: {}", session_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::models::Session;
    use crate::db::repository::SessionRepository;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_file_crud() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let session_repo = SessionRepository::new(db.pool().clone());
        let file_repo = FileRepository::new(db.pool().clone());

        // Create session first
        let session = Session::new(Some("Test".to_string()), Some("model".to_string()));
        session_repo
            .create(&session)
            .await
            .expect("Failed to create session");

        // Create file
        let file = File::new(session.id, PathBuf::from("/test/file.rs"), None);
        file_repo
            .create(&file)
            .await
            .expect("Failed to create file");

        // Read
        let found = file_repo.find_by_id(file.id).await.expect("Failed to find");
        assert!(found.is_some());
        assert_eq!(found.as_ref().unwrap().path, PathBuf::from("/test/file.rs"));

        // Update
        let mut updated = file.clone();
        updated.content = Some("Updated content".to_string());
        file_repo.update(&updated).await.expect("Failed to update");

        let found = file_repo.find_by_id(file.id).await.expect("Failed to find");
        assert_eq!(found.unwrap().content, Some("Updated content".to_string()));

        // Delete
        file_repo.delete(file.id).await.expect("Failed to delete");
        let found = file_repo.find_by_id(file.id).await.expect("Failed to find");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_file_list_by_session() {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let session_repo = SessionRepository::new(db.pool().clone());
        let file_repo = FileRepository::new(db.pool().clone());

        let session = Session::new(Some("Test".to_string()), Some("model".to_string()));
        session_repo
            .create(&session)
            .await
            .expect("Failed to create session");

        // Create multiple files
        for i in 0..3 {
            let file = File::new(
                session.id,
                PathBuf::from(format!("/test/file{}.rs", i)),
                None,
            );
            file_repo
                .create(&file)
                .await
                .expect("Failed to create file");
        }

        let files = file_repo
            .list_by_session(session.id)
            .await
            .expect("Failed to list");
        assert_eq!(files.len(), 3);

        let count = file_repo
            .count_by_session(session.id)
            .await
            .expect("Failed to count");
        assert_eq!(count, 3);
    }
}
