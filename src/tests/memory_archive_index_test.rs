//! Archived memory must stay searchable (#1657).
//!
//! The archive pass retires cold MEMORY.md sections into
//! `memory/archive/YYYY-MM.md`. `reindex`'s daily walk is flat (a single
//! non-recursive `read_dir`), so without the dedicated archive walk those
//! files would be invisible to `memory_search` and archiving would be
//! deletion in disguise. These tests pin the walk (`archive_memory_files`)
//! and the indexed doc keys at the store level, without touching embeddings,
//! external config, or the live home.

use crate::memory::COLLECTION_MEMORY;
use crate::memory::db::Store;
use crate::memory::index::{archive_memory_files, index_file_sync_keyed};

/// The walk finds `.md` files under `memory/archive/`, keys them with the
/// `archive/` prefix, ignores other extensions, and returns them sorted.
#[test]
fn archive_walk_keys_files_with_prefix_and_ignores_non_md() {
    let home = tempfile::tempdir().unwrap();
    let memory = home.path().join("memory");
    std::fs::create_dir_all(memory.join("archive")).unwrap();
    std::fs::write(memory.join("2026-09-20.md"), "# daily").unwrap();
    std::fs::write(memory.join("archive/2026-09.md"), "# september").unwrap();
    std::fs::write(memory.join("archive/2026-08.md"), "# august").unwrap();
    std::fs::write(memory.join("archive/notes.txt"), "not markdown").unwrap();

    let files = archive_memory_files(&memory);
    let keys: Vec<&str> = files.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, vec!["archive/2026-08.md", "archive/2026-09.md"]);
    for (_, path) in &files {
        assert!(path.starts_with(memory.join("archive")));
    }
}

/// A memory dir with no archive subdir yields nothing and does not error:
/// the walk runs on every reindex, including fresh installs.
#[test]
fn archive_walk_is_empty_without_archive_dir() {
    let home = tempfile::tempdir().unwrap();
    let memory = home.path().join("memory");
    std::fs::create_dir_all(&memory).unwrap();
    std::fs::write(memory.join("2026-09-20.md"), "# daily").unwrap();
    assert!(archive_memory_files(&memory).is_empty());
}

/// End-to-end at the store level: an archived month indexes under its
/// prefixed key, lands active in the memory collection, and re-indexing
/// unchanged content hash-skips under the SAME key (proving key stability,
/// which the reindex prune relies on).
#[test]
fn archived_file_indexes_under_prefixed_key_and_stays_active() {
    let home = tempfile::tempdir().unwrap();
    let memory = home.path().join("memory");
    std::fs::create_dir_all(memory.join("archive")).unwrap();
    let body = "# Archived MEMORY.md sections 2026-09\n\n\
                ## Old staging layout\n\n\
                The staging box lived on Hetzner until 2025.\n";
    std::fs::write(memory.join("archive/2026-09.md"), body).unwrap();

    let store = Store::open(&home.path().join("test.db")).unwrap();
    for (key, path) in archive_memory_files(&memory) {
        let content = std::fs::read_to_string(&path).unwrap();
        let indexed = index_file_sync_keyed(&store, COLLECTION_MEMORY, &key, &content).unwrap();
        assert!(indexed, "first index must write the document");
        let again = index_file_sync_keyed(&store, COLLECTION_MEMORY, &key, &content).unwrap();
        assert!(
            !again,
            "unchanged content must hash-skip under the same key"
        );
    }

    let paths = store.get_active_document_paths(COLLECTION_MEMORY).unwrap();
    assert!(
        paths.iter().any(|p| p == "archive/2026-09.md"),
        "archived month must be active in the memory collection so memory_search can see it: {paths:?}"
    );
    let found = store
        .find_active_document(COLLECTION_MEMORY, "archive/2026-09.md")
        .unwrap();
    assert!(found.is_some(), "archive doc must be findable by its key");
}
