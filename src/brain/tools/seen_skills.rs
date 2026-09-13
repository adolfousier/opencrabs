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

/// Process-wide loaded mtime registry (#210): (session, slug) -> mtime of the skill file
/// when it was last loaded/checked for that session.
fn loaded_mtimes() -> &'static std::sync::Mutex<HashMap<(Uuid, String), u64>> {
    static LOADED_MTIMES: OnceLock<std::sync::Mutex<HashMap<(Uuid, String), u64>>> = OnceLock::new();
    LOADED_MTIMES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Retrieve the recorded `loaded_mtime` for an active skill in a session (#210).
pub fn get_skill_loaded_mtime(session_id: Uuid, slug: &str) -> Option<u64> {
    loaded_mtimes()
        .lock()
        .expect("seen_skills loaded_mtimes poisoned")
        .get(&(session_id, slug.to_string()))
        .copied()
}

/// Record or update the `loaded_mtime` for an active skill in a session (#210).
pub fn record_skill_loaded_mtime(session_id: Uuid, slug: &str, mtime: u64) {
    loaded_mtimes()
        .lock()
        .expect("seen_skills loaded_mtimes poisoned")
        .insert((session_id, slug.to_string()), mtime);

    // Best-effort persistence to DB so loaded_mtimes survive restarts (#210).
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let slug = slug.to_string();
        handle.spawn(async move {
            if let Some(pool) = crate::db::global_pool() {
                let repo = crate::db::repository::SessionSkillsRepository::new(pool.clone());
                let _ = repo.set_loaded_mtime(session_id, &slug, mtime).await;
            }
        });
    }
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
    /// `(session, slug)` -> epoch - exactly the in-memory registry's shape.
    pub seen: HashMap<(Uuid, String), u64>,
    /// Per-session epoch floor, seeded from the MAX persisted row epoch.
    pub epochs: HashMap<Uuid, u64>,
    /// `(session, slug)` -> loaded mtime (#210).
    pub loaded_mtimes: HashMap<(Uuid, String), u64>,
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
pub fn hydrate_from_rows(
    rows: Vec<(Uuid, String, Option<i64>, Option<i64>)>,
) -> HydrationSeeds {
    let mut seeds = HydrationSeeds {
        seen: HashMap::new(),
        epochs: HashMap::new(),
        loaded_mtimes: HashMap::new(),
    };
    for (session_id, slug, epoch, loaded_mtime) in rows {
        let epoch = epoch.unwrap_or(0).max(0) as u64;
        seeds
            .seen
            .entry((session_id, slug.clone()))
            .and_modify(|e| *e = (*e).max(epoch))
            .or_insert(epoch);
        seeds
            .epochs
            .entry(session_id)
            .and_modify(|e| *e = (*e).max(epoch))
            .or_insert(epoch);
        if let Some(mtime) = loaded_mtime.filter(|&m| m > 0) {
            seeds.loaded_mtimes.insert((session_id, slug), mtime as u64);
        }
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
    {
        let mut mtimes = loaded_mtimes().lock().expect("seen_skills loaded_mtimes poisoned");
        for (key, mtime) in seeds.loaded_mtimes {
            mtimes.insert(key, mtime);
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
///
/// The once-flag is claimed AFTER the pool and runtime guards, never before:
/// a caller that arrives while the pool is still uninstalled has not hydrated
/// anything, so burning the flag there would disable hydration for the whole
/// process on the strength of a `debug!` line. Construction before
/// `Database::connect` is a real order that exists in this tree
/// (`a2a::test_helpers`), so an early caller must leave the door open for the
/// one that follows it.
pub fn hydrate_from_db() {
    static HYDRATED: AtomicBool = AtomicBool::new(false);
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
    if HYDRATED.swap(true, Ordering::SeqCst) {
        return;
    }
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
