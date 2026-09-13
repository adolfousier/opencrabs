//! Session-scoped seen-skill tracking (issue #131, #150).
//!
//! Records which skill bodies a session has CONSUMED — by any surface — so
//! the post-compaction advisory stamp (#125) can list skills the agent
//! actually read, not only those invoked via slash command.
//!
//! Two hooks feed this registry:
//! - `load_brain_file` with a bare skill slug (the #131 canonical form)
//! - `read_file` on a `skills/<slug>/SKILL.md` path (whole-file reads)
//!
//! In-memory registry stores `(session_id, slug) -> epoch` (issue #150).
//! When context compaction occurs, `note_compaction` bumps the session's epoch.
//! `seen_since_compaction` checks whether a skill was seen at or after the current epoch.
//!
//! This is deliberately SEPARATE from `AgentService::active_skills` (the
//! #219 slash-invocation registry): that set also drives per-turn body
//! re-injection into the system prompt, and read-counted skills must not be
//! re-injected on top of the read already present in conversation history.
//! The compaction stamp is the UNION of both registries.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

/// In-memory registry: (session, slug) → the compaction epoch at which the
/// skill body was last loaded into that session's context (issue #150).
/// Epoch 0 == "loaded before any compaction".
fn registry() -> &'static std::sync::Mutex<HashMap<(Uuid, String), u64>> {
    static REGISTRY: OnceLock<std::sync::Mutex<HashMap<(Uuid, String), u64>>> = OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Per-session compaction epoch counter (issue #150). `note_compaction`
/// bumps it; skills seen at an older epoch are no longer "in context".
fn epochs() -> &'static std::sync::Mutex<HashMap<Uuid, u64>> {
    static EPOCHS: OnceLock<std::sync::Mutex<HashMap<Uuid, u64>>> = OnceLock::new();
    EPOCHS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Extract the skill slug from a path that points at a skill definition
/// file: any path whose second-to-last component is `skills` and whose
/// file name is `SKILL.md` yields `Some(slug)`. Returns `None` for
/// everything else (brain files, regular files, skill assets).
pub fn skill_slug_from_path(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    if file_name != "SKILL.md" {
        return None;
    }
    let mut comps = path.components().rev();
    comps.next()?; // SKILL.md
    let slug = comps.next()?;
    if comps.next()?.as_os_str() != "skills" {
        return None;
    }
    slug.as_os_str().to_str().map(|s| s.to_string())
}

/// Record that `session_id` consumed skill `slug` (via read or slug-form
/// load) at the session's CURRENT compaction epoch.
///
/// Always upserts (issue #150: updating the epoch ensures that re-reading
/// a skill after compaction unblocks the skill glob gate).
pub fn mark_seen(session_id: Uuid, slug: &str) {
    let epoch = current_epoch(session_id);
    registry()
        .lock()
        .expect("seen_skills registry poisoned")
        .insert((session_id, slug.to_string()), epoch);
    // #138: best-effort durability. One row per (session, slug) so the
    // registry can be rebuilt at boot. Detached so the hot path never
    // blocks, and WARN-only on failure — the in-memory registry is the
    // source of truth for this run, durability is a bonus (acceptance 5).
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let slug = slug.to_string();
        handle.spawn(async move {
            match persist_seen(session_id, &slug, epoch).await {
                Ok(()) => {}
                Err(e) => tracing::warn!(
                    "seen_skills: DB persist of ({session_id}, {slug}) failed (in-memory \
                     registry unaffected): {e:#}"
                ),
            }
        });
    }
}

/// Persist one (session, slug, epoch) row. Soft no-op when no DB pool is
/// installed yet (unit tests, pre-`Database::connect` boot) — that is not
/// an error, the caller has no durability target.
async fn persist_seen(session_id: Uuid, slug: &str, epoch: u64) -> anyhow::Result<()> {
    let Some(pool) = crate::db::global_pool() else {
        return Ok(());
    };
    crate::db::repository::SessionSkillsRepository::new(pool.clone())
        .record(session_id, slug, epoch)
        .await
}

/// The session's current compaction epoch (0 before any compaction).
fn current_epoch(session_id: Uuid) -> u64 {
    *epochs()
        .lock()
        .expect("seen_skills epochs poisoned")
        .get(&session_id)
        .unwrap_or(&0)
}

/// Bump the session's compaction epoch (issue #150): called from the
/// compaction path. AFTER a compaction, the registry entries keep their
/// old epoch, so `seen_since_compaction` flips false for every skill
/// until the body is re-read. Clears NOTHING — the #125 stamp's
/// seen-inventory (`seen_for_session`) remains intact.
pub fn note_compaction(session_id: Uuid) {
    let mut epochs = epochs().lock().expect("seen_skills epochs poisoned");
    let next = epochs.get(&session_id).copied().unwrap_or(0) + 1;
    epochs.insert(session_id, next);
}

/// Whether `session_id`'s context currently holds skill `slug`'s body:
/// the stored epoch is >= the session's current epoch. Fresh sessions
/// (no rows) report false — the gate fires on the first matching call.
pub fn seen_since_compaction(session_id: Uuid, slug: &str) -> bool {
    let stored = registry()
        .lock()
        .expect("seen_skills registry poisoned")
        .get(&(session_id, slug.to_string()))
        .copied();
    if let Some(epoch) = stored {
        epoch >= current_epoch(session_id)
    } else {
        false
    }
}

/// Whether `session_id` has consumed skill `slug` this run (any epoch —
/// legacy stamp inventory semantics; the gate uses
/// [`seen_since_compaction`]).
pub fn was_seen(session_id: Uuid, slug: &str) -> bool {
    registry()
        .lock()
        .expect("seen_skills registry poisoned")
        .contains_key(&(session_id, slug.to_string()))
}

/// All skills `session_id` has consumed, sorted (deterministic stamp order).
pub fn seen_for_session(session_id: Uuid) -> Vec<String> {
    let all: BTreeSet<String> = registry()
        .lock()
        .expect("seen_skills registry poisoned")
        .iter()
        .filter(|((sid, _), _)| *sid == session_id)
        .map(|((_, slug), _)| slug.clone())
        .collect();
    all.into_iter().collect()
}

// ---------------------------------------------------------------------------
// #138: boot hydration — the registry survives daemon restarts
// ---------------------------------------------------------------------------

/// Rows loaded from `session_seen_skills`, shaped for the pure fold below.
/// Deliberately separate from the DB row type so the restart semantics stay
/// unit-testable without a pool.
pub struct HydrationSeeds {
    /// `(session, slug)` → epoch — exactly the in-memory registry's shape.
    pub seen: HashMap<(Uuid, String), u64>,
    /// Per-session epoch floor, seeded from the MAX persisted row epoch.
    pub epochs: HashMap<Uuid, u64>,
}

/// Fold persisted rows into registry seeds. PURE — no I/O, no globals — so
/// the restart semantics are testable directly.
///
/// `epoch` is NULL for rows written before the column existed (#150); NULL
/// reads as 0 == "loaded before any compaction", the permissive end, so a
/// pre-feature row can never wrongly gate a skill.
///
/// The session's epoch counter is seeded from the MAX row epoch: a restart
/// must not leave the counter BELOW an epoch the session already reached,
/// or every skill loaded after that compaction would falsely re-gate.
pub fn hydrate_from_rows(rows: Vec<(Uuid, String, Option<i64>)>) -> HydrationSeeds {
    let mut seeds = HydrationSeeds {
        seen: HashMap::new(),
        epochs: HashMap::new(),
    };
    for (session_id, slug, epoch) in rows {
        let epoch = epoch.unwrap_or(0).max(0) as u64;
        seeds
            .seen
            .entry((session_id, slug))
            .and_modify(|e| *e = (*e).max(epoch))
            .or_insert(epoch);
        seeds
            .epochs
            .entry(session_id)
            .and_modify(|e| *e = (*e).max(epoch))
            .or_insert(epoch);
    }
    seeds
}

/// Install seeds into the in-memory registries. Synchronous on purpose — a
/// `MutexGuard` is not `Send`, so this must never be held across an await.
/// Returns the number of seen rows installed (the log line's count).
///
/// Merge is MAX-wins: a skill marked seen earlier in THIS run keeps its
/// epoch rather than being rewound to the persisted one.
pub fn apply_seeds(seeds: HydrationSeeds) -> usize {
    let n = seeds.seen.len();
    {
        let mut registry = registry().lock().expect("seen_skills registry poisoned");
        for (key, epoch) in seeds.seen {
            registry
                .entry(key)
                .and_modify(|e| *e = (*e).max(epoch))
                .or_insert(epoch);
        }
    }
    {
        let mut epochs = epochs().lock().expect("seen_skills epochs poisoned");
        for (session_id, epoch) in seeds.epochs {
            epochs
                .entry(session_id)
                .and_modify(|e| *e = (*e).max(epoch))
                .or_insert(epoch);
        }
    }
    n
}

/// Once-per-process boot hydrate (issue #138): read every persisted
/// `(session, slug, epoch)` row into the in-memory registry, then drop rows
/// whose session no longer exists.
///
/// Detached, because the caller (`AgentService::new`) must never block on
/// I/O. A missing pool is not an error — unit tests and any construction
/// before `Database::connect` simply have no durability target (acceptance
/// 5: no panic paths, the in-memory registry keeps working regardless).
pub fn hydrate_from_db() {
    static HYDRATED: AtomicBool = AtomicBool::new(false);
    if HYDRATED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(pool) = crate::db::global_pool().cloned() else {
        tracing::debug!(
            "seen_skills: no DB pool at boot — registry stays in-memory only (restart \
             durability unavailable this run)"
        );
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::debug!("seen_skills: no tokio runtime at boot — skipping DB hydration");
        return;
    };
    handle.spawn(async move {
        let repo = crate::db::repository::SessionSkillsRepository::new(pool);
        match repo.all().await {
            Ok(rows) => {
                let n = apply_seeds(hydrate_from_rows(rows));
                tracing::debug!("seen_skills: hydrated registry from DB ({n} seen rows)");
            }
            Err(e) => tracing::warn!(
                "seen_skills: DB hydration failed (registry starts empty, restart durability \
                 lost this run): {e:#}"
            ),
        }
        match repo.prune_missing_sessions().await {
            Ok(0) => {}
            Ok(n) => tracing::debug!("seen_skills: pruned {n} rows for deleted sessions"),
            Err(e) => tracing::warn!("seen_skills: prune of orphaned rows failed: {e:#}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_extraction_from_skill_paths() {
        assert_eq!(
            skill_slug_from_path(Path::new(
                "/root/.opencrabs/profiles/ops/skills/opencrabs-dev/SKILL.md"
            )),
            Some("opencrabs-dev".to_string())
        );
        assert_eq!(
            skill_slug_from_path(Path::new("skills/foo/SKILL.md")),
            Some("foo".to_string())
        );
    }

    #[test]
    fn non_skill_paths_yield_none() {
        assert_eq!(
            skill_slug_from_path(Path::new("/home/user/MEMORY.md")),
            None
        );
        assert_eq!(skill_slug_from_path(Path::new("skills/foo/other.md")), None);
        assert_eq!(
            skill_slug_from_path(Path::new("not-skills/foo/SKILL.md")),
            None
        );
        assert_eq!(skill_slug_from_path(Path::new("skills/foo/")), None);
    }

    #[test]
    fn mark_seen_is_idempotent_and_session_scoped() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        mark_seen(a, "opencrabs-dev");
        mark_seen(a, "opencrabs-dev");
        assert!(was_seen(a, "opencrabs-dev"));
        assert_eq!(seen_for_session(a), vec!["opencrabs-dev".to_string()]);
        assert!(!was_seen(b, "opencrabs-dev"));
        assert!(seen_for_session(b).is_empty());
    }

    #[test]
    fn seen_list_is_sorted_and_multi() {
        let a = Uuid::new_v4();
        mark_seen(a, "zeta");
        mark_seen(a, "alpha");
        assert_eq!(
            seen_for_session(a),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }

    // --- issue #150: epoch-carrying registry + skill-gate semantics ---

    #[test]
    fn fresh_session_reports_not_seen_since_compaction() {
        let a = Uuid::new_v4();
        assert!(!seen_since_compaction(a, "anything"));
    }

    #[test]
    fn seen_passes_and_compaction_rearms_gate() {
        let a = Uuid::new_v4();
        mark_seen(a, "my-skill");
        assert!(seen_since_compaction(a, "my-skill"));
        // A compaction bumps the epoch; the stored row keeps the old one.
        note_compaction(a);
        assert!(!seen_since_compaction(a, "my-skill"));
        // Re-reading re-arms at the new epoch.
        mark_seen(a, "my-skill");
        assert!(seen_since_compaction(a, "my-skill"));
    }

    #[test]
    fn compaction_clears_nothing_stamp_inventory_intact() {
        let a = Uuid::new_v4();
        mark_seen(a, "one");
        mark_seen(a, "two");
        note_compaction(a);
        // The #125 stamp's seen-inventory survives.
        assert_eq!(
            seen_for_session(a),
            vec!["one".to_string(), "two".to_string()]
        );
        // ...but neither body is "in context" for gate purposes.
        assert!(!seen_since_compaction(a, "one"));
        assert!(!seen_since_compaction(a, "two"));
    }

    #[test]
    fn sessions_are_independent() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        mark_seen(a, "sk");
        note_compaction(a);
        // b never compacted: its row stays current.
        mark_seen(b, "sk");
        assert!(seen_since_compaction(b, "sk"));
        assert!(!seen_since_compaction(a, "sk"));
    }

    // --- issue #138: boot hydration (registry survives restarts) ---

    #[test]
    fn hydration_takes_max_epoch_per_session() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let seeds = hydrate_from_rows(vec![
            (a, "one".to_string(), Some(1)),
            (a, "two".to_string(), Some(3)),
            (b, "one".to_string(), Some(1)),
        ]);
        assert_eq!(seeds.seen.len(), 3);
        // The counter floors at the HIGHEST epoch this session reached.
        assert_eq!(seeds.epochs.get(&a), Some(&3));
        assert_eq!(seeds.epochs.get(&b), Some(&1));
    }

    #[test]
    fn hydration_restores_epoch_counter_so_next_compaction_gates() {
        let a = Uuid::new_v4();
        apply_seeds(hydrate_from_rows(vec![(a, "sk".to_string(), Some(2))]));
        assert!(was_seen(a, "sk"));
        assert!(seen_since_compaction(a, "sk"));
        // The counter came back at 2, so the NEXT compaction is epoch 3 and
        // the epoch-2 row is correctly stale. Without epoch seeding the
        // counter would restart at 0, this bump would land on 1, and the
        // 2 >= 1 compare would wrongly keep the skill "in context".
        note_compaction(a);
        assert!(!seen_since_compaction(a, "sk"));
    }

    #[test]
    fn hydration_null_epoch_reads_as_zero_and_stays_permissive() {
        let a = Uuid::new_v4();
        apply_seeds(hydrate_from_rows(vec![(a, "legacy".to_string(), None)]));
        assert!(was_seen(a, "legacy"));
        assert!(seen_since_compaction(a, "legacy"));
        // A negative epoch (never written by us, but possible in a
        // hand-edited row) clamps to 0 rather than wrapping to u64::MAX.
        let b = Uuid::new_v4();
        apply_seeds(hydrate_from_rows(vec![(b, "odd".to_string(), Some(-5))]));
        assert!(seen_since_compaction(b, "odd"));
    }

    #[test]
    fn hydration_preserves_stamp_inventory_across_restart() {
        let a = Uuid::new_v4();
        apply_seeds(hydrate_from_rows(vec![
            (a, "zeta".to_string(), Some(0)),
            (a, "alpha".to_string(), Some(1)),
        ]));
        // The #125 stamp reads this list — sorted, both rows present.
        assert_eq!(
            seen_for_session(a),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }
}
