//! Store — singleton qmd Store for the memory database.

use once_cell::sync::OnceCell;
use qmd::Store;
use std::path::PathBuf;
use std::sync::Mutex;

static STORE: OnceCell<Mutex<Store>> = OnceCell::new();

/// Get (or create) the shared memory qmd Store.
///
/// The database lives at `~/.opencrabs/memory/memory.db`.
/// First call initializes the schema via `Store::open` and creates the vector table.
pub fn get_store() -> Result<&'static Mutex<Store>, String> {
    STORE.get_or_try_init(|| {
        let db_path = memory_dir().join("memory.db");

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create memory dir: {e}"))?;
        }

        let store =
            Store::open(&db_path).map_err(|e| format!("Failed to open memory store: {e}"))?;

        store
            .ensure_vector_table(768)
            .map_err(|e| format!("Failed to create vector table: {e}"))?;

        tracing::info!("Memory qmd store ready at {}", db_path.display());
        Ok(Mutex::new(store))
    })
}

/// Path to the memory directory: `~/.opencrabs/memory/`
fn memory_dir() -> PathBuf {
    crate::config::opencrabs_home().join("memory")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_dir() {
        let dir = memory_dir();
        assert!(dir.to_string_lossy().contains("memory"));
    }

    #[test]
    fn test_index_and_search_integration() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = Store::open(&db_path).unwrap();

        let body = "# Session\nFixed the authentication bug in login flow";
        let hash = Store::hash_content(body);
        let now = "2024-01-01T00:00:00";
        let title = Store::extract_title(body);

        store.insert_content(&hash, body, now).unwrap();
        store
            .insert_document("test", "2024-01-01.md", &title, &hash, now, now)
            .unwrap();

        let results = store
            .search_fts("\"authentication\"", 5, Some("test"))
            .unwrap();
        assert!(!results.is_empty());

        let found = store.find_active_document("test", "2024-01-01.md").unwrap();
        assert!(found.is_some());
        let (_id, found_hash, _title) = found.unwrap();
        assert_eq!(found_hash, hash);
    }

    #[test]
    fn test_vector_search_returns_results() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vec.db");
        let store = Store::open(&db_path).unwrap();
        store.ensure_vector_table(768).unwrap();

        let body = "# Debug\nTracked down the memory leak in the render loop";
        let hash = Store::hash_content(body);
        let title = Store::extract_title(body);
        let now = "2024-01-01T00:00:00";

        store.insert_content(&hash, body, now).unwrap();
        store
            .insert_document("memory", "2024-01-01.md", &title, &hash, now, now)
            .unwrap();

        // Insert a fake 768-dim embedding
        let mut embedding = vec![0.0f32; 768];
        embedding[0] = 1.0;
        embedding[1] = 0.5;
        store
            .insert_embedding(&hash, 0, 0, &embedding, "test-model", now)
            .unwrap();

        // Query with a similar vector — should find the document
        let mut query_emb = vec![0.0f32; 768];
        query_emb[0] = 0.9;
        query_emb[1] = 0.6;

        let results = store.search_vec(&query_emb, 5, None).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].doc.title, "Debug");
    }

    #[test]
    fn test_vector_search_empty_when_no_embeddings() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("novec.db");
        let store = Store::open(&db_path).unwrap();
        store.ensure_vector_table(768).unwrap();

        let body = "# Session\nSome content without embeddings";
        let hash = Store::hash_content(body);
        let title = Store::extract_title(body);
        let now = "2024-01-01T00:00:00";

        store.insert_content(&hash, body, now).unwrap();
        store
            .insert_document("memory", "2024-01-01.md", &title, &hash, now, now)
            .unwrap();

        // Vector search with no embeddings stored — should return empty
        let query_emb = vec![0.1f32; 768];
        let results = store.search_vec(&query_emb, 5, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_hashes_needing_embedding() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("need.db");
        let store = Store::open(&db_path).unwrap();
        store.ensure_vector_table(768).unwrap();

        let body1 = "# First\nDocument without embedding";
        let hash1 = Store::hash_content(body1);
        let now = "2024-01-01T00:00:00";

        store.insert_content(&hash1, body1, now).unwrap();
        store
            .insert_document(
                "memory",
                "first.md",
                &Store::extract_title(body1),
                &hash1,
                now,
                now,
            )
            .unwrap();

        let body2 = "# Second\nDocument with embedding";
        let hash2 = Store::hash_content(body2);

        store.insert_content(&hash2, body2, now).unwrap();
        store
            .insert_document(
                "memory",
                "second.md",
                &Store::extract_title(body2),
                &hash2,
                now,
                now,
            )
            .unwrap();

        // Embed only the second document
        let emb = vec![0.1f32; 768];
        store
            .insert_embedding(&hash2, 0, 0, &emb, "test", now)
            .unwrap();

        // Only the first should need embedding
        let needing = store.get_hashes_needing_embedding().unwrap();
        assert_eq!(needing.len(), 1);
        assert_eq!(needing[0].0, hash1);
    }

    #[test]
    fn test_rrf_merges_fts_and_vector() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("rrf.db");
        let store = Store::open(&db_path).unwrap();
        store.ensure_vector_table(768).unwrap();

        let now = "2024-01-01T00:00:00";

        // Doc A: matches FTS for "authentication", has embedding
        let body_a = "# Auth Fix\nFixed the authentication bug in the login flow";
        let hash_a = Store::hash_content(body_a);
        store.insert_content(&hash_a, body_a, now).unwrap();
        store
            .insert_document(
                "memory",
                "auth.md",
                &Store::extract_title(body_a),
                &hash_a,
                now,
                now,
            )
            .unwrap();
        let mut emb_a = vec![0.0f32; 768];
        emb_a[0] = 1.0;
        store
            .insert_embedding(&hash_a, 0, 0, &emb_a, "test", now)
            .unwrap();

        // Doc B: different content, different embedding direction
        let body_b = "# Refactor\nRefactored the database connection pooling layer";
        let hash_b = Store::hash_content(body_b);
        store.insert_content(&hash_b, body_b, now).unwrap();
        store
            .insert_document(
                "memory",
                "refactor.md",
                &Store::extract_title(body_b),
                &hash_b,
                now,
                now,
            )
            .unwrap();
        let mut emb_b = vec![0.0f32; 768];
        emb_b[1] = 1.0;
        store
            .insert_embedding(&hash_b, 0, 0, &emb_b, "test", now)
            .unwrap();

        // FTS finds doc A for "authentication"
        let fts = store
            .search_fts("\"authentication\"", 5, Some("memory"))
            .unwrap();
        assert!(!fts.is_empty());
        assert_eq!(fts[0].doc.title, "Auth Fix");

        // Vector search close to emb_a finds doc A first
        let mut q = vec![0.0f32; 768];
        q[0] = 0.9;
        let vec_results = store.search_vec(&q, 5, Some("memory")).unwrap();
        assert!(!vec_results.is_empty());
        assert_eq!(vec_results[0].doc.title, "Auth Fix");

        // RRF combines both — doc A should rank highest (appears in both lists)
        use qmd::hybrid_search_rrf;
        let fts_tuples: Vec<_> = fts
            .iter()
            .map(|r| {
                (
                    r.doc.path.clone(),
                    r.doc.path.clone(),
                    r.doc.title.clone(),
                    body_a.to_string(),
                )
            })
            .collect();
        let vec_tuples: Vec<_> = vec_results
            .iter()
            .map(|r| {
                let body = if r.doc.title == "Auth Fix" {
                    body_a
                } else {
                    body_b
                };
                (
                    r.doc.path.clone(),
                    r.doc.path.clone(),
                    r.doc.title.clone(),
                    body.to_string(),
                )
            })
            .collect();

        let rrf = hybrid_search_rrf(fts_tuples, vec_tuples, 60);
        assert!(!rrf.is_empty());
        assert_eq!(rrf[0].title, "Auth Fix");
        assert!(rrf[0].score > 0.0);
    }
}
