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
}
