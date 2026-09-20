//! Archive pass for cold MEMORY.md sections (#1657 piece B).
//!
//! Covers pure candidate selection (structural H2+ gate, belief-required,
//! cold yardstick) and the crash-safe `archive_cold_sections_at` core over
//! temp dirs: sections land in the monthly file, survivors stay byte-exact,
//! a backup snapshot precedes the rewrite, and an empty pass writes nothing.

use crate::brain::tools::epistemic::*;
use chrono::{Duration, Utc};

/// Build a store whose belief for `heading` was recorded `days_ago` days
/// ago with `hits` accesses — the minimal cold/hot fixture.
fn store_with_belief(heading: &str, days_ago: i64, hits: u64) -> EpistemicStore {
    let mut store = EpistemicStore::new();
    store.add_belief(
        &format!("MEMORY.md##{heading}"),
        "fixture value",
        Confidence::Inferred,
        "test",
    );
    let key = format!("MEMORY.md##{heading}");
    let belief = store.beliefs.get_mut(&key).expect("belief just added");
    belief.source.recorded_at = Utc::now() - Duration::days(days_ago);
    belief.source.last_verified = Utc::now() - Duration::days(days_ago);
    belief.hits = hits;
    store
}

const CONTENT: &str = "Preamble line that is never a section of its own file title.\n\
                      \n\
                      # MEMORY\n\
                      \n\
                      ## Cold Section\n\
                      body for the cold section, long enough to count as real content\n\
                      \n\
                      ## Hot Section\n\
                      body for the hot section, long enough to count as real content\n\
                      \n\
                      ## Untracked Section\n\
                      body for the untracked section, long enough to count as content\n";

#[test]
fn archive_cold_selection_skips_preamble_h1_and_untracked() {
    let mut store = store_with_belief("Cold Section", 120, 0);
    store_with_belief_into(&mut store, "Hot Section", 120, 5);
    let now = Utc::now();

    let selected = select_archive_candidates(&store, CONTENT, 90, now);

    // Only the cold tracked section; hot (has hits), untracked (no belief),
    // preamble and H1 (structurally excluded) all stay put.
    assert_eq!(selected.len(), 1, "selection: {selected:?}");
    assert_eq!(selected[0].title, "Cold Section");
    assert_eq!(selected[0].heading, "## Cold Section");
    assert_eq!(selected[0].belief_key, "MEMORY.md##Cold Section");
    assert!(selected[0].rendered.contains("body for the cold section"));
}

#[test]
fn archive_cold_selection_requires_cold_belief() {
    // Recently recorded, zero hits: not cold yet.
    let store = store_with_belief("Cold Section", 10, 0);
    let selected = select_archive_candidates(&store, CONTENT, 90, Utc::now());
    assert!(selected.is_empty(), "10 days old is not cold: {selected:?}");

    // Cold by age but warmed by use: not cold either.
    let store = store_with_belief("Cold Section", 120, 3);
    let selected = select_archive_candidates(&store, CONTENT, 90, Utc::now());
    assert!(selected.is_empty(), "hits warm the belief: {selected:?}");
}

#[test]
fn archive_cold_at_moves_sections_and_removes_beliefs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let memory_path = dir.path().join("MEMORY.md");
    std::fs::write(&memory_path, CONTENT).expect("write fixture");

    let mut store = store_with_belief("Cold Section", 120, 0);
    store_with_belief_into(&mut store, "Hot Section", 1, 0);
    let now = Utc::now();

    let report = archive_cold_sections_at(
        &mut store,
        &memory_path,
        &dir.path().join("archive"),
        90,
        now,
    )
    .expect("archive pass");

    assert!(!report.skipped);
    assert_eq!(report.removed_beliefs, vec!["MEMORY.md##Cold Section"]);

    // Archive file holds the retired section, under the current month.
    let archive_path = dir
        .path()
        .join("archive")
        .join(format!("{}.md", now.format("%Y-%m")));
    let archived = std::fs::read_to_string(&archive_path).expect("archive file");
    assert!(archived.contains("## Cold Section"));
    assert!(archived.contains("body for the cold section"));

    // MEMORY.md keeps everything else, byte-for-byte semantics: the hot
    // and untracked sections and the preamble survive untouched.
    let memory = std::fs::read_to_string(&memory_path).expect("reread MEMORY.md");
    assert!(!memory.contains("Cold Section"));
    assert!(memory.contains("## Hot Section"));
    assert!(memory.contains("## Untracked Section"));
    assert!(memory.contains("Preamble line"));

    // The hot belief survives the store; the cold one is gone.
    assert!(store.get_belief("MEMORY.md##Hot Section").is_some());
    assert!(store.get_belief("MEMORY.md##Cold Section").is_none());
}

#[test]
fn archive_cold_at_creates_backup_before_rewrite() {
    let dir = tempfile::tempdir().expect("tempdir");
    let memory_path = dir.path().join("MEMORY.md");
    std::fs::write(&memory_path, CONTENT).expect("write fixture");

    let mut store = store_with_belief("Cold Section", 120, 0);
    let report = archive_cold_sections_at(
        &mut store,
        &memory_path,
        &dir.path().join("archive"),
        90,
        Utc::now(),
    )
    .expect("archive pass");

    let backup = report.backup.expect("backup snapshot exists");
    let backed_up = std::fs::read_to_string(&backup).expect("backup readable");
    assert_eq!(backed_up, CONTENT, "backup holds the pre-rewrite content");
}

#[test]
fn archive_cold_at_empty_pass_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let memory_path = dir.path().join("MEMORY.md");
    std::fs::write(&memory_path, CONTENT).expect("write fixture");

    // Nothing cold: the only tracked belief is warm.
    let mut store = store_with_belief("Hot Section", 1, 0);
    let before = std::fs::read_to_string(&memory_path).expect("read fixture");

    let report = archive_cold_sections_at(
        &mut store,
        &memory_path,
        &dir.path().join("archive"),
        90,
        Utc::now(),
    )
    .expect("archive pass");

    assert!(report.skipped);
    assert!(report.archived.is_empty());
    assert!(report.archive_path.is_none());
    assert!(report.backup.is_none());
    assert!(
        !dir.path().join("archive").exists(),
        "no archive dir created"
    );
    assert_eq!(
        std::fs::read_to_string(&memory_path).expect("reread"),
        before,
        "MEMORY.md untouched on an empty pass"
    );
}

/// In-place variant of [`store_with_belief`] for multi-belief fixtures.
fn store_with_belief_into(store: &mut EpistemicStore, heading: &str, days_ago: i64, hits: u64) {
    store.add_belief(
        &format!("MEMORY.md##{heading}"),
        "fixture value",
        Confidence::Inferred,
        "test",
    );
    let key = format!("MEMORY.md##{heading}");
    let belief = store.beliefs.get_mut(&key).expect("belief just added");
    belief.source.recorded_at = Utc::now() - Duration::days(days_ago);
    belief.source.last_verified = Utc::now() - Duration::days(days_ago);
    belief.hits = hits;
}
