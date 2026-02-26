//! Indexing — insert memory/brain files into the qmd store and generate embeddings.

use qmd::Store;
use std::path::Path;
use std::sync::Mutex;

use super::embedding::{backfill_embeddings, embed_content};
use super::{COLLECTION_BRAIN, COLLECTION_MEMORY};

/// Brain files loaded from the workspace root (`~/.opencrabs/`).
const BRAIN_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "AGENTS.md",
    "TOOLS.md",
    "SECURITY.md",
    "MEMORY.md",
    "BOOT.md",
    "BOOTSTRAP.md",
    "HEARTBEAT.md",
];

/// Index a single `.md` file into the qmd store under the `"memory"` collection.
///
/// Skips re-indexing if the file's SHA-256 hash hasn't changed.
/// Generates an embedding when the engine is already initialized.
pub async fn index_file(store: &'static Mutex<Store>, path: &Path) -> Result<(), String> {
    let body = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let indexed = {
            let s = store
                .lock()
                .map_err(|e| format!("Store lock poisoned: {e}"))?;
            index_file_sync(&s, COLLECTION_MEMORY, &path, &body)?
        };

        if indexed {
            embed_content(store, &body);
        }

        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Synchronous inner implementation for indexing a single file into a given collection.
/// Returns `true` if new content was indexed, `false` if hash-skipped.
fn index_file_sync(
    store: &Store,
    collection: &str,
    path: &Path,
    body: &str,
) -> Result<bool, String> {
    let hash = Store::hash_content(body);
    let rel_path = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    if let Ok(Some((_id, existing_hash, _title))) =
        store.find_active_document(collection, &rel_path)
        && existing_hash == hash
    {
        return Ok(false);
    }

    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let title = Store::extract_title(body);

    store
        .insert_content(&hash, body, &now)
        .map_err(|e| format!("Failed to insert content: {e}"))?;
    store
        .insert_document(collection, &rel_path, &title, &hash, &now, &now)
        .map_err(|e| format!("Failed to insert document: {e}"))?;

    tracing::debug!("Indexed {collection} file: {}", path.display());
    Ok(true)
}

/// Walk `~/.opencrabs/memory/*.md` and `~/.opencrabs/*.md` brain files, indexing all.
///
/// Also deactivates entries for files that no longer exist on disk.
/// After indexing, backfills embeddings for any documents missing them.
/// Returns the number of files indexed.
pub async fn reindex(store: &'static Mutex<Store>) -> Result<usize, String> {
    let home = crate::config::opencrabs_home();
    let dir = home.join("memory");
    let mut indexed = 0usize;
    let mut memory_on_disk: Vec<String> = Vec::new();
    let mut brain_on_disk: Vec<String> = Vec::new();

    // --- Index daily memory logs ---
    if dir.exists() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("Failed to read memory dir: {e}"))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let rel = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                memory_on_disk.push(rel);

                if let Err(e) = index_file(store, &path).await {
                    tracing::warn!("Failed to index {}: {}", path.display(), e);
                } else {
                    indexed += 1;
                }
            }
        }
    }

    // --- Index brain workspace files ---
    for &name in BRAIN_FILES {
        let path = home.join(name);
        if path.exists() {
            let body = match tokio::fs::read_to_string(&path).await {
                Ok(b) if !b.trim().is_empty() => b,
                _ => continue,
            };
            brain_on_disk.push(name.to_string());

            let result: Result<bool, String> = tokio::task::spawn_blocking({
                let path = path.clone();
                move || {
                    let store = store
                        .lock()
                        .map_err(|e| format!("Store lock poisoned: {e}"))?;
                    index_file_sync(&store, COLLECTION_BRAIN, &path, &body)
                }
            })
            .await
            .map_err(|e| format!("spawn_blocking failed: {e}"))?;

            match result {
                Ok(_) => indexed += 1,
                Err(e) => tracing::warn!("Failed to index brain file {name}: {e}"),
            }
        }
    }

    // --- Prune deleted files from both collections ---
    let prune_result: Result<(), String> = tokio::task::spawn_blocking({
        move || {
            let store = store
                .lock()
                .map_err(|e| format!("Store lock poisoned: {e}"))?;

            if let Ok(db_paths) = store.get_active_document_paths(COLLECTION_MEMORY) {
                for db_path in &db_paths {
                    if !memory_on_disk.contains(db_path) {
                        let _ = store.deactivate_document(COLLECTION_MEMORY, db_path);
                        tracing::debug!("Pruned missing memory file: {}", db_path);
                    }
                }
            }

            if let Ok(db_paths) = store.get_active_document_paths(COLLECTION_BRAIN) {
                for db_path in &db_paths {
                    if !brain_on_disk.contains(db_path) {
                        let _ = store.deactivate_document(COLLECTION_BRAIN, db_path);
                        tracing::debug!("Pruned missing brain file: {}", db_path);
                    }
                }
            }

            Ok(())
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?;

    if let Err(e) = prune_result {
        tracing::warn!("Memory prune failed: {e}");
    }

    // --- Backfill embeddings for documents missing them ---
    tokio::task::spawn_blocking(move || backfill_embeddings(store))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?;

    tracing::info!("Memory reindex complete: {} files", indexed);
    Ok(indexed)
}
