//! Tests for memory garbage collection, orphan pruning, and maintenance sweeps (#241).

use crate::memory::db::{MemoryGcReport, Store};
use crate::services::maintenance::{leave, try_enter};
use tempfile::tempdir;

#[test]
fn test_memory_gc_orphans() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let store = Store::open(&db_path).expect("open store");

    store.ensure_vector_table(4).expect("ensure vector table");

    // Insert content and documents
    store
        .insert_content("hash_active", "Active doc content", "2026-09-15T00:00:00Z")
        .expect("insert active content");
    store
        .insert_content("hash_orphan", "Orphan doc content", "2026-09-15T00:00:00Z")
        .expect("insert orphan content");

    store
        .insert_document(
            "brain",
            "ACTIVE.md",
            "Active Doc",
            "hash_active",
            "2026-09-15T00:00:00Z",
            "2026-09-15T00:00:00Z",
        )
        .expect("insert active doc");

    // Insert embedding for active and orphan
    store
        .insert_embedding(
            "hash_active",
            0,
            0,
            &[1.0, 2.0, 3.0, 4.0],
            "test-model",
            "2026-09-15T00:00:00Z",
            Some("hash_active_chunk"),
        )
        .expect("insert active embedding");
    store
        .insert_embedding(
            "hash_orphan",
            0,
            0,
            &[5.0, 6.0, 7.0, 8.0],
            "test-model",
            "2026-09-15T00:00:00Z",
            Some("hash_orphan_chunk"),
        )
        .expect("insert orphan embedding");

    // Pre-GC state check: content_vectors has 2 rows
    let stats_before = store.vector_stats().expect("stats before");
    assert_eq!(stats_before.vector_rows, 2);

    // Run GC
    let report: MemoryGcReport = store.gc_orphans().expect("gc_orphans");

    assert_eq!(report.orphaned_vectors_pruned, 1);
    assert_eq!(report.orphaned_embeddings_pruned, 1);
    assert_eq!(report.unreferenced_content_pruned, 1);

    // Post-GC stats: only active vector row remains
    let stats_after = store.vector_stats().expect("stats after");
    assert_eq!(stats_after.vector_rows, 1);

    // Running GC again is idempotent
    let report2 = store.gc_orphans().expect("gc_orphans again");
    assert_eq!(report2.orphaned_vectors_pruned, 0);
    assert_eq!(report2.orphaned_embeddings_pruned, 0);
    assert_eq!(report2.unreferenced_content_pruned, 0);

    // Test vacuum_memory
    store.vacuum_memory().expect("vacuum_memory");
}

#[test]
fn test_maintenance_single_flight() {
    assert!(try_enter());
    // Second entry fails while holding
    assert!(!try_enter());
    leave();
    // Re-entry succeeds after leave
    assert!(try_enter());
    leave();
}
