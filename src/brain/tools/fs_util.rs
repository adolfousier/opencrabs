//! Filesystem utilities for brain tools.
//!
//! Provides atomic file write operations to prevent in-place truncation
//! and file corruption while processes may be executing scripts or reading files.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::brain::tools::error::ToolError;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempFileGuard<'a> {
    path: &'a Path,
    active: bool,
}

impl<'a> TempFileGuard<'a> {
    fn new(path: &'a Path) -> Self {
        Self { path, active: true }
    }

    fn defuse(&mut self) {
        self.active = false;
    }
}

impl<'a> Drop for TempFileGuard<'a> {
    fn drop(&mut self) {
        if self.active {
            let _ = std::fs::remove_file(self.path);
        }
    }
}

/// Atomically write `content` to `path` using a sibling temporary file and rename.
///
/// If `path` already exists, its file permissions are preserved on the replaced file.
/// Writes and flushes to disk before atomically replacing the destination via `fs::rename`.
pub async fn atomic_write_file(path: &Path, content: &[u8]) -> Result<(), ToolError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    // Capture existing permissions if the target file exists
    let existing_permissions = if fs::try_exists(path).await.unwrap_or(false) {
        fs::metadata(path).await.ok().map(|m| m.permissions())
    } else {
        None
    };

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");

    let tmp_path = parent.join(format!(
        ".tmp_{file_name}.{}.{}",
        std::process::id(),
        TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let mut guard = TempFileGuard::new(&tmp_path);

    {
        let mut file = fs::File::create(&tmp_path).await.map_err(ToolError::Io)?;
        file.write_all(content).await.map_err(ToolError::Io)?;
        file.flush().await.map_err(ToolError::Io)?;
        file.sync_all().await.map_err(ToolError::Io)?;
    }

    if let Some(perms) = existing_permissions {
        let _ = fs::set_permissions(&tmp_path, perms).await;
    }

    if let Err(e) = fs::rename(&tmp_path, path).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(ToolError::Io(e));
    }

    guard.defuse();
    Ok(())
}
