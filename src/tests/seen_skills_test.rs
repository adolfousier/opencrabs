// Tests for issue #131: skills are loadable via `load_brain_file` slug form,
// and any skill-body consumption (read or slug-load) marks the skill SEEN so
// the post-compaction inventory stamp (#125) lists it.

use crate::brain::tools::Tool;
use crate::brain::tools::ToolExecutionContext;
use crate::brain::tools::load_brain_file::*;
use crate::brain::tools::seen_skills;
use crate::brain::tools::seen_skills::*;
use crate::db::Database;
use crate::db::repository::SessionSkillsRepository;
use std::path::Path;
use uuid::Uuid;

fn ctx() -> ToolExecutionContext {
    ToolExecutionContext::new(Uuid::new_v4())
}

fn tool() -> LoadBrainFileTool {
    LoadBrainFileTool
}

// ── acceptance 1: slug form loads the body; traversal still refused ────────

#[tokio::test]
async fn slug_form_loads_builtin_skill_body() {
    let c = ctx();
    let result = tool()
        .execute(serde_json::json!({"name": "cost-estimate"}), &c)
        .await
        .unwrap();
    assert!(result.success, "slug form of a built-in skill must succeed");
    let text = result.output;
    assert!(
        text.contains("--- skill: cost-estimate ---"),
        "body must be framed as a skill, got: {}",
        &text[..text.len().min(200)]
    );
    assert!(!text.is_empty(), "skill body must not be empty");
}

#[tokio::test]
async fn slug_form_with_query_returns_matching_sections() {
    let c = ctx();
    let result = tool()
        .execute(
            serde_json::json!({"name": "cost-estimate", "query": "usage"}),
            &c,
        )
        .await
        .unwrap();
    assert!(result.success, "slug+query form must succeed");
}

#[tokio::test]
async fn path_traversal_still_refused_after_slug_form_added() {
    for bad in [
        "../../etc/passwd",
        "sub/file.md",
        "../skills/cost-estimate/SKILL.md",
    ] {
        let result = tool()
            .execute(serde_json::json!({"name": bad}), &ctx())
            .await
            .unwrap();
        assert!(!result.success, "traversal input {bad} must fail");
    }
}

#[tokio::test]
async fn unknown_slug_falls_through_to_brain_file_handling() {
    let result = tool()
        .execute(
            serde_json::json!({"name": "no-such-skill-or-brain"}),
            &ctx(),
        )
        .await
        .unwrap();

    // The point of the assertion is that an unresolvable slug must never be
    // served as if it were a skill body. The shape of the miss is main's to
    // decide, and main answers a missing brain file with a success-carrying
    // "not found" message rather than an error (load_brain_file.rs), so this
    // pins the fall-through by content instead of by the success flag.
    let body = result.output;
    assert!(
        body.contains("not found"),
        "unknown slug must fall through to the brain-file miss, got: {body}"
    );
    assert!(
        !body.contains("--- skill:"),
        "unknown slug must never be answered with a skill body, got: {body}"
    );
}

// ── acceptance 2: slug-load marks the skill SEEN ────────────────────────────

#[tokio::test]
async fn slug_load_marks_skill_seen_for_session() {
    let c = ctx();
    let session = c.session_id;
    assert!(
        !seen_skills::was_seen(session, "cost-estimate"),
        "fresh session must not have the skill seen"
    );
    tool()
        .execute(serde_json::json!({"name": "cost-estimate"}), &c)
        .await
        .unwrap();
    assert!(
        seen_skills::was_seen(session, "cost-estimate"),
        "slug-load must mark the skill seen"
    );
    assert_eq!(
        seen_skills::seen_for_session(session),
        vec!["cost-estimate".to_string()]
    );
}

// ── acceptance 2/3: read_file on a SKILL.md also counts ────────────────────

#[test]
fn read_file_whole_read_marks_skill_seen_via_hook() {
    // The read.rs hook calls skill_slug_from_path before mark_seen; here we
    // verify the registry contract the hook relies on, session-scoped.
    let session = Uuid::new_v4();
    let path = std::path::Path::new("/root/.opencrabs/profiles/ops/skills/grafana/SKILL.md");
    let slug = seen_skills::skill_slug_from_path(path).expect("skill path must yield slug");
    assert_eq!(slug, "grafana");
    seen_skills::mark_seen(session, &slug);
    assert!(seen_skills::was_seen(session, "grafana"));
}

// ── acceptance 3: union of both registries is deduplicated ─────────────────

#[test]
fn stamp_union_dedupes_active_and_seen() {
    let mut active = std::collections::BTreeSet::new();
    active.insert("opencrabs-dev".to_string());
    let mut seen = std::collections::BTreeSet::new();
    seen.insert("opencrabs-dev".to_string());
    seen.insert("grafana".to_string());
    let union: Vec<String> = active.union(&seen).cloned().collect();
    assert_eq!(union.len(), 2, "overlap must dedupe");
    assert_eq!(
        union,
        vec!["grafana".to_string(), "opencrabs-dev".to_string()]
    );
}

// ── acceptance 5: stamp-build observability is a DEBUG line ────────────────
// Verified by code inspection of continuation_prompt (tracing::debug! with
// the full inventory list); zero-skill silence is covered by the existing
// skill_stamp_is_silent_when_no_skills_active test on append_skill_stamp.

// ── moved out of an inline `mod tests` in src/brain/tools/seen_skills.rs ──
//
// Tests live under src/tests/ (house rule); the slug/registry unit cases
// arrived inline with the #131 port and are exercised here through the same
// public API instead.

#[test]
fn slug_extraction_from_skill_paths() {
    assert_eq!(
        seen_skills::skill_slug_from_path(Path::new(
            "/root/.opencrabs/profiles/ops/skills/opencrabs-dev/SKILL.md"
        )),
        Some("opencrabs-dev".to_string())
    );
    assert_eq!(
        seen_skills::skill_slug_from_path(Path::new("skills/foo/SKILL.md")),
        Some("foo".to_string())
    );
}

#[test]
fn non_skill_paths_yield_none() {
    assert_eq!(
        seen_skills::skill_slug_from_path(Path::new("/home/user/MEMORY.md")),
        None
    );
    assert_eq!(
        seen_skills::skill_slug_from_path(Path::new("skills/foo/other.md")),
        None
    );
    assert_eq!(
        seen_skills::skill_slug_from_path(Path::new("not-skills/foo/SKILL.md")),
        None
    );
    assert_eq!(
        seen_skills::skill_slug_from_path(Path::new("skills/foo/")),
        None
    );
}

#[test]
fn mark_seen_is_idempotent_and_session_scoped() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seen_skills::mark_seen(a, "opencrabs-dev");
    seen_skills::mark_seen(a, "opencrabs-dev");
    assert!(seen_skills::was_seen(a, "opencrabs-dev"));
    assert_eq!(
        seen_skills::seen_for_session(a),
        vec!["opencrabs-dev".to_string()]
    );
    assert!(!seen_skills::was_seen(b, "opencrabs-dev"));
    assert!(seen_skills::seen_for_session(b).is_empty());
}

#[test]
fn seen_list_is_sorted_and_multi() {
    let a = Uuid::new_v4();
    seen_skills::mark_seen(a, "zeta");
    seen_skills::mark_seen(a, "alpha");
    assert_eq!(
        seen_skills::seen_for_session(a),
        vec!["alpha".to_string(), "zeta".to_string()]
    );
}

// ══ issue #138: the registry survives daemon restarts ══════════════════════
//
// Before #138 the registry was in-memory only, so a restart made a
// skill-consuming session look skill-less to the post-compaction stamp. The
// row write is best-effort (acceptance 5); the read side is exercised here
// against a real in-memory DB with the migrations applied.

/// In-memory DB with all migrations applied — including the #138 table.
async fn test_db() -> Database {
    let db = Database::connect_in_memory()
        .await
        .expect("in-memory DB should connect");
    db.run_migrations().await.expect("migrations should apply");
    db
}

/// Minimal `sessions` row: every later migration's column on that table is
/// either nullable or defaulted, so (id, created_at, updated_at) suffices.
async fn insert_session(db: &Database, id: Uuid) {
    db.pool()
        .get()
        .await
        .expect("pool connection")
        .interact(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![id.to_string(), 0i64, 0i64],
            )
        })
        .await
        .expect("interact")
        .expect("insert session");
}

// ── acceptance 2: filename form registers consumption (the live gap) ───────

#[tokio::test]
async fn filename_form_marks_skill_seen_for_session() {
    let c = ctx();
    let session = c.session_id;
    let result = tool()
        .execute(serde_json::json!({"name": "cost-estimate.md"}), &c)
        .await
        .unwrap();
    assert!(
        result.success,
        "filename form of a built-in skill must succeed"
    );
    assert!(
        seen_skills::was_seen(session, "cost-estimate"),
        "filename-form load must mark the skill seen"
    );
}

#[tokio::test]
async fn filename_form_with_query_marks_skill_seen_for_session() {
    // The #138 probe caught this live: a query-filtered filename-form load
    // resolved nothing and registered nothing, so the skill never reached
    // the stamp inventory.
    let c = ctx();
    let session = c.session_id;
    let result = tool()
        .execute(
            serde_json::json!({"name": "cost-estimate.md", "query": "estimate"}),
            &c,
        )
        .await
        .unwrap();
    assert!(result.success);
    assert!(
        seen_skills::was_seen(session, "cost-estimate"),
        "a query-filtered filename-form load must still register consumption"
    );
}

// ── acceptance 1: restart does not lose seen-skill state ───────────────────

#[tokio::test]
async fn record_and_read_round_trip() {
    let db = test_db().await;
    let repo = SessionSkillsRepository::new(db.pool().clone());
    let sid = Uuid::new_v4();
    repo.record(sid, "opencrabs-dev", 0).await.unwrap();
    repo.record(sid, "cost-estimate", 2).await.unwrap();

    let rows = repo.all().await.unwrap();
    assert_eq!(rows.len(), 2);
    let ce = rows
        .iter()
        .find(|r| r.1 == "cost-estimate")
        .expect("cost-estimate row must round-trip");
    let od = rows
        .iter()
        .find(|r| r.1 == "opencrabs-dev")
        .expect("opencrabs-dev row must round-trip");
    assert_eq!(ce.0, sid);
    assert_eq!(ce.2, Some(2));
    assert_eq!(od.0, sid);
    assert_eq!(od.2, Some(0));
}

#[tokio::test]
async fn record_upserts_epoch_instead_of_duplicating() {
    let db = test_db().await;
    let repo = SessionSkillsRepository::new(db.pool().clone());
    let sid = Uuid::new_v4();
    repo.record(sid, "sk", 0).await.unwrap();
    repo.record(sid, "sk", 3).await.unwrap();

    let rows = repo.all().await.unwrap();
    assert_eq!(rows.len(), 1, "same (session, slug) must upsert");
    assert_eq!(
        rows[0].2,
        Some(3),
        "the newer epoch must win — a stale row would re-gate the skill (#150)"
    );
}

#[tokio::test]
async fn restart_hydrates_registry_from_persisted_rows() {
    let db = test_db().await;
    let repo = SessionSkillsRepository::new(db.pool().clone());
    let sid = Uuid::new_v4();
    repo.record(sid, "opencrabs-dev", 1).await.unwrap();

    // Simulate a daemon restart: the row survives, the in-memory registry
    // does not. Boot rehydrates from what was persisted.
    let rows = repo.all().await.unwrap();
    seen_skills::apply_seeds(seen_skills::hydrate_from_rows(rows));

    assert!(seen_skills::was_seen(sid, "opencrabs-dev"));
    assert_eq!(
        seen_skills::seen_for_session(sid),
        vec!["opencrabs-dev".to_string()],
        "the #125 stamp inventory must list a pre-restart skill"
    );
    assert!(
        seen_skills::seen_since_compaction(sid, "opencrabs-dev"),
        "the restored epoch counter must keep the skill in context"
    );
}

#[tokio::test]
async fn prune_drops_orphans_and_keeps_live_sessions() {
    let db = test_db().await;
    let repo = SessionSkillsRepository::new(db.pool().clone());
    let live = Uuid::new_v4();
    let dead = Uuid::new_v4();
    insert_session(&db, live).await;
    repo.record(live, "sk", 0).await.unwrap();
    repo.record(dead, "sk", 0).await.unwrap();

    let pruned = repo.prune_missing_sessions().await.unwrap();
    assert_eq!(pruned, 1, "only the orphaned row is dropped");

    let rows = repo.all().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, live, "a live session's rows must survive");
}

// ───────────────────────────────────────────────────────────────────────────
// Registry unit tests, moved out of the inline `#[cfg(test)] mod tests` block
// at the bottom of `src/brain/tools/seen_skills.rs`. CONTRIBUTING.md: every
// test lives under `src/tests/` as a dedicated `*_test.rs` file, and an
// inline block found while working a file moves as part of that change.
// Bodies are unchanged — `super::*` became the explicit glob at the top.
// Four of the moved tests (slug extraction, non-skill paths, idempotent
// mark, sorted list) were already sitting above: an earlier move copied
// them out and left the originals behind, so the inline block had been a
// stale duplicate of them since. Only the copies above survive.
// ───────────────────────────────────────────────────────────────────────────

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
        (a, "one".to_string(), Some(1), None),
        (a, "two".to_string(), Some(3), None),
        (b, "one".to_string(), Some(1), None),
    ]);
    assert_eq!(seeds.seen.len(), 3);
    // The counter floors at the HIGHEST epoch this session reached.
    assert_eq!(seeds.epochs.get(&a), Some(&3));
    assert_eq!(seeds.epochs.get(&b), Some(&1));
}

#[test]
fn hydration_restores_epoch_counter_so_next_compaction_gates() {
    let a = Uuid::new_v4();
    apply_seeds(hydrate_from_rows(vec![(a, "sk".to_string(), Some(2), None)]));
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
    apply_seeds(hydrate_from_rows(vec![(a, "legacy".to_string(), None, None)]));
    assert!(was_seen(a, "legacy"));
    assert!(seen_since_compaction(a, "legacy"));
    // A negative epoch (never written by us, but possible in a
    // hand-edited row) clamps to 0 rather than wrapping to u64::MAX.
    let b = Uuid::new_v4();
    apply_seeds(hydrate_from_rows(vec![(b, "odd".to_string(), Some(-5), None)]));
    assert!(seen_since_compaction(b, "odd"));
}

#[test]
fn hydration_preserves_stamp_inventory_across_restart() {
    let a = Uuid::new_v4();
    apply_seeds(hydrate_from_rows(vec![
        (a, "zeta".to_string(), Some(0), None),
        (a, "alpha".to_string(), Some(1), None),
    ]));
    // The #125 stamp reads this list — sorted, both rows present.
    assert_eq!(
        seen_for_session(a),
        vec!["alpha".to_string(), "zeta".to_string()]
    );
}


#[test]
fn hydration_restores_loaded_mtimes() {
    let a = Uuid::new_v4();
    let seeds = hydrate_from_rows(vec![
        (a, "alpha".to_string(), Some(1), Some(1726000000)),
        (a, "beta".to_string(), Some(1), None),
    ]);
    assert_eq!(seeds.loaded_mtimes.get(&(a, "alpha".to_string())), Some(&1726000000));
    assert_eq!(seeds.loaded_mtimes.get(&(a, "beta".to_string())), None);
}

// ══ the boot-hydration once-flag is claimed AFTER the readiness guards ═════
//
// Source-scan sentinel, because the flag is a function-local `static` with no
// reader and no reset: a behavioural test would have to drive a process-wide
// one-shot, which makes it order-dependent on every other test in the binary
// — the exact flakiness class #1535/#1536 just removed. What can be pinned is
// the ordering itself, and the ordering is the whole bug: claiming the flag
// above the pool check lets a caller that hydrated NOTHING spend the
// process's single attempt, silently disabling restart durability for that
// run. `a2a::test_helpers` already builds a service before `Database::connect`
// installs the pool, so the early-caller order is not hypothetical.

#[test]
fn boot_hydration_claims_its_flag_after_the_pool_guard() {
    let src = include_str!("../brain/tools/seen_skills.rs");
    let body = src
        .split("pub fn hydrate_from_db()")
        .nth(1)
        .expect("hydrate_from_db must exist");
    let pool = body
        .find("global_pool()")
        .expect("hydrate_from_db must still guard on the global pool");
    let claim = body
        .find("HYDRATED.swap")
        .expect("hydrate_from_db must still claim a once-flag");
    assert!(
        claim > pool,
        "the once-flag is claimed at byte {claim}, before the pool guard at \
         {pool}: a boot with no pool would burn the process's only hydration \
         attempt and restart durability would be lost for the whole run"
    );
}
