//! Tests for the shared session plan file store (`utils::plan_files`):
//! lifecycle state derivation, the durable pre-init Editing flag, the
//! legacy seven-status load map (with silent Completed archive and
//! Cancelled delete), canonical status strings on write, the design `.md`
//! scaffold, and the Editing markdown-to-description mirror.

use crate::config::profile::{home_for_profile, with_profile_home_async};
use crate::tui::plan::{PlanDocument, PlanStatus, PlanTask, TaskType};
use crate::utils::plan_files::{
    PlanModeState, archive_dir, create_design_md, discard_plan, is_plan_autonomy,
    is_pre_init_editing, load_plan, plan_json_path, plan_md_path, plan_mode_state,
    pre_init_marker_path, save_plan, set_plan_autonomy, set_pre_init_editing, sync_md_to_json,
    template_section_warnings,
};
use uuid::Uuid;

/// Run `f` under a throwaway profile home so nothing touches the real
/// `~/.opencrabs/agents/session/`, then clean the profile dir up.
async fn in_temp_home<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let profile = format!("plan-files-test-{}", Uuid::new_v4());
    let out = with_profile_home_async(Some(&profile), f).await;
    let home = home_for_profile(Some(&profile));
    let _ = std::fs::remove_dir_all(&home);
    out
}

fn task(order: usize, title: &str) -> PlanTask {
    PlanTask::new(order, title.to_string(), "desc".to_string(), TaskType::Edit)
}

async fn write_raw_plan(session_id: Uuid, status: &str, task_count: usize) {
    let mut plan = PlanDocument::new(session_id, "Legacy plan".to_string());
    for i in 0..task_count {
        plan.add_task(task(i + 1, &format!("t{}", i + 1)));
    }
    save_plan(&plan).await.unwrap();
    // Rewrite the status string raw, bypassing the canonical serializer.
    let path = plan_json_path(session_id).await;
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["status"] = serde_json::Value::String(status.to_string());
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

#[tokio::test]
async fn no_plan_state_when_no_file() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        assert_eq!(plan_mode_state(sid).await, PlanModeState::NoPlan);
        assert!(!is_pre_init_editing(sid).await);
        assert!(load_plan(sid).await.is_none());
    })
    .await;
}

#[tokio::test]
async fn pre_init_flag_is_durable_and_gates_state() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        set_pre_init_editing(sid).await.unwrap();

        // The flag is a durable marker file, not process state: a fresh read
        // of disk (what a restart does) still sees it. It is NOT a fake plan
        // JSON (#569): no plan document, no approvable `.md`.
        assert!(pre_init_marker_path(sid).await.exists());
        assert!(!plan_json_path(sid).await.exists());
        assert!(!plan_md_path(sid).await.exists());
        assert!(load_plan(sid).await.is_none());

        assert!(is_pre_init_editing(sid).await);
        assert_eq!(plan_mode_state(sid).await, PlanModeState::PreInitEditing);

        // Setting it again is idempotent.
        set_pre_init_editing(sid).await.unwrap();
        assert!(is_pre_init_editing(sid).await);

        // Discard clears the marker and returns to NoPlan.
        discard_plan(sid).await;
        assert_eq!(plan_mode_state(sid).await, PlanModeState::NoPlan);
        assert!(!pre_init_marker_path(sid).await.exists());
    })
    .await;
}

#[tokio::test]
async fn init_clears_pre_init_marker_and_archive_does_not_resurrect_it() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        set_pre_init_editing(sid).await.unwrap();
        assert!(pre_init_marker_path(sid).await.exists());

        // `plan init` writes the first real plan JSON via save_plan, which must
        // clear the marker so a later archive can't leave the session looking
        // pre-init (#569).
        let mut plan = PlanDocument::new(sid, "Design".to_string());
        save_plan(&plan).await.unwrap();
        assert!(!pre_init_marker_path(sid).await.exists());

        // Take it Active, then archive: back to NoPlan, never PreInitEditing.
        plan.add_task(task(1, "t1"));
        plan.status = PlanStatus::Active;
        save_plan(&plan).await.unwrap();
        crate::utils::plan_files::archive_plan(sid).await.unwrap();
        assert_eq!(plan_mode_state(sid).await, PlanModeState::NoPlan);
    })
    .await;
}

#[tokio::test]
async fn legacy_stub_pre_init_json_still_resolves() {
    // Back-compat: sessions that already have the old stub plan JSON on disk
    // (pre_init_editing=true, empty tasks, no .md) must still resolve to
    // PreInitEditing even though new pre-init uses a marker file (#569).
    in_temp_home(async {
        let sid = Uuid::new_v4();
        let mut stub = PlanDocument::new(sid, String::new());
        stub.pre_init_editing = true;
        stub.status = PlanStatus::Editing;
        save_plan(&stub).await.unwrap();

        assert_eq!(plan_mode_state(sid).await, PlanModeState::PreInitEditing);
        assert!(is_pre_init_editing(sid).await);
    })
    .await;
}

#[tokio::test]
async fn pre_init_refused_when_plan_is_live() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        let mut plan = PlanDocument::new(sid, "Live".to_string());
        plan.add_task(task(1, "t1"));
        plan.status = PlanStatus::Active;
        save_plan(&plan).await.unwrap();

        assert!(set_pre_init_editing(sid).await.is_err());
        assert_eq!(plan_mode_state(sid).await, PlanModeState::Active);
    })
    .await;
}

#[tokio::test]
async fn post_init_editing_requires_md() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        let plan = PlanDocument::new(sid, "Design".to_string());
        save_plan(&plan).await.unwrap();
        create_design_md(sid, "Design").await.unwrap();

        assert_eq!(plan_mode_state(sid).await, PlanModeState::PostInitEditing);
        assert!(!is_pre_init_editing(sid).await);
    })
    .await;
}

#[tokio::test]
async fn save_writes_canonical_status_strings() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        let mut plan = PlanDocument::new(sid, "Canonical".to_string());
        save_plan(&plan).await.unwrap();
        let raw = std::fs::read_to_string(plan_json_path(sid).await).unwrap();
        assert!(raw.contains("\"Editing\""), "got: {raw}");

        plan.status = PlanStatus::Active;
        save_plan(&plan).await.unwrap();
        let raw = std::fs::read_to_string(plan_json_path(sid).await).unwrap();
        assert!(raw.contains("\"Active\""), "got: {raw}");
    })
    .await;
}

#[tokio::test]
async fn legacy_draft_checklist_normalizes_to_active() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        // Old-world Draft with tasks: executable then, must stay executable
        // now (there is no .md to approve, so Editing would trap it).
        write_raw_plan(sid, "Draft", 2).await;
        let plan = load_plan(sid).await.unwrap();
        assert_eq!(plan.status, PlanStatus::Active);
        assert_eq!(plan_mode_state(sid).await, PlanModeState::Active);
    })
    .await;
}

#[tokio::test]
async fn legacy_approved_and_in_progress_map_to_active() {
    in_temp_home(async {
        for legacy in ["Approved", "InProgress"] {
            let sid = Uuid::new_v4();
            write_raw_plan(sid, legacy, 1).await;
            let plan = load_plan(sid).await.unwrap();
            assert_eq!(plan.status, PlanStatus::Active, "legacy {legacy}");
            assert_eq!(plan_mode_state(sid).await, PlanModeState::Active);
        }
    })
    .await;
}

#[tokio::test]
async fn legacy_completed_archives_silently() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        write_raw_plan(sid, "Completed", 1).await;

        assert!(
            load_plan(sid).await.is_none(),
            "completed plan must resolve to NoPlan"
        );
        assert!(
            !plan_json_path(sid).await.exists(),
            "live JSON must be gone"
        );
        assert_eq!(plan_mode_state(sid).await, PlanModeState::NoPlan);

        // The plan retired into the archive dir instead of being lost.
        let archived: Vec<_> = std::fs::read_dir(archive_dir(sid).await)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(&sid.to_string()) && n.ends_with(".json"))
            .collect();
        assert_eq!(
            archived.len(),
            1,
            "expected one archived JSON, got {archived:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn legacy_cancelled_deletes() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        write_raw_plan(sid, "Cancelled", 1).await;
        assert!(load_plan(sid).await.is_none());
        assert!(!plan_json_path(sid).await.exists());
        assert_eq!(plan_mode_state(sid).await, PlanModeState::NoPlan);
    })
    .await;
}

#[tokio::test]
async fn idle_active_plan_with_empty_tasks_survives_load() {
    in_temp_home(async {
        // A seed-failed Active plan (no tasks yet) must stay intact so the
        // idle retry path can pick it up: loading is never destructive for
        // live statuses.
        let sid = Uuid::new_v4();
        let mut plan = PlanDocument::new(sid, "Seed failed".to_string());
        plan.status = PlanStatus::Active;
        save_plan(&plan).await.unwrap();

        let loaded = load_plan(sid).await.unwrap();
        assert_eq!(loaded.status, PlanStatus::Active);
        assert!(loaded.tasks.is_empty());
        assert!(plan_json_path(sid).await.exists());
        assert_eq!(plan_mode_state(sid).await, PlanModeState::Active);
    })
    .await;
}

#[tokio::test]
async fn design_md_scaffold_and_mirror() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        let plan = PlanDocument::new(sid, "Design doc".to_string());
        save_plan(&plan).await.unwrap();
        let md_path = create_design_md(sid, "Design doc").await.unwrap();
        let scaffold = std::fs::read_to_string(&md_path).unwrap();
        assert!(scaffold.starts_with("# Design doc"));
        assert!(scaffold.contains("## Context"));
        assert!(scaffold.contains("## Implementation steps"));
        assert!(
            scaffold.contains("1. \n   - Done when: "),
            "scaffold must model the per-step Done when convention"
        );

        // Edit the .md, then mirror: the JSON description follows the body
        // and tasks stay empty (Editing cannot persist a checklist).
        let body = "# Design doc\n\n## Context\n- **Problem:** X is broken\n\
                    - **Target state:** X works\n- **Intent:** user asked\n\n\
                    ## Implementation steps\n1. Fix X in module Y\n";
        std::fs::write(&md_path, body).unwrap();
        sync_md_to_json(sid).await.unwrap();

        let mirrored = load_plan(sid).await.unwrap();
        assert_eq!(mirrored.description, body);
        assert!(mirrored.tasks.is_empty());
        assert_eq!(mirrored.status, PlanStatus::Editing);
    })
    .await;
}

#[tokio::test]
async fn sync_refuses_malformed_body_and_restores_previous_mirror() {
    in_temp_home(async {
        let sid = Uuid::new_v4();
        let plan = PlanDocument::new(sid, "Guarded design".to_string());
        save_plan(&plan).await.unwrap();
        let md_path = create_design_md(sid, "Guarded design").await.unwrap();
        let valid = "# Guarded design\n\n## Context\n- **Problem:** old\n- **Target state:** fixed\n- **Intent:** test\n\n## Implementation steps\n1. Keep old\n";
        std::fs::write(&md_path, valid).unwrap();
        sync_md_to_json(sid).await.unwrap();

        let malformed = "# Guarded design\n\n## Context\n- **Problem:**\n  text moved to the next line\n- **Target state:** fixed\n- **Intent:** test\n\n## Implementation steps\n1. Keep old\n";
        std::fs::write(&md_path, malformed).unwrap();
        let error = sync_md_to_json(sid).await.unwrap_err();
        assert!(error.contains("`**Problem:**` needs non-empty text after the label"));
        assert!(error.contains("each `**Label:**` must be a single line: label + space + text"));
        assert_eq!(std::fs::read_to_string(&md_path).unwrap(), valid);
        assert_eq!(load_plan(sid).await.unwrap().description, valid);

        let empty_label = "# Guarded design\n\n## Context\n- **Problem:** \n- **Target state:** fixed\n- **Intent:** test\n\n## Implementation steps\n1. Keep old\n";
        std::fs::write(&md_path, empty_label).unwrap();
        let error = sync_md_to_json(sid).await.unwrap_err();
        assert!(error.contains("`**Problem:**` needs non-empty text after the label"));
        assert_eq!(std::fs::read_to_string(&md_path).unwrap(), valid);
        assert_eq!(load_plan(sid).await.unwrap().description, valid);
    })
    .await;
}

#[test]
fn template_warnings_flag_missing_sections() {
    let empty = template_section_warnings("just prose, no structure");
    assert!(empty.iter().any(|w| w.contains("## Context")));
    assert!(empty.iter().any(|w| w.contains("## Implementation steps")));

    // Labels present but unfilled still warn.
    let unfilled = "## Context\n- **Problem:** \n- **Target state:** \n- **Intent:** \n\
                    \n## Implementation steps\n1. \n";
    let w = template_section_warnings(unfilled);
    assert!(w.iter().any(|x| x.contains("**Problem:**")));
    assert!(w.iter().any(|x| x.contains("numbered step")));

    // A filled template is quiet.
    let filled = "## Context\n- **Problem:** broken\n- **Target state:** fixed\n\
                  - **Intent:** asked\n\n## Implementation steps\n1. do the thing\n";
    assert!(template_section_warnings(filled).is_empty());
}

#[tokio::test]
async fn plan_autonomy_is_a_durable_session_policy() {
    // Self-approval autonomy is granted/revoked per session, survives across
    // plans (not cleared by discard), and defaults off (#581).
    in_temp_home(async {
        let sid = Uuid::new_v4();
        assert!(!is_plan_autonomy(sid).await, "default off");

        set_plan_autonomy(sid, true).await.unwrap();
        assert!(is_plan_autonomy(sid).await);

        // A plan lifecycle does not touch the session policy.
        set_pre_init_editing(sid).await.unwrap();
        discard_plan(sid).await;
        assert!(
            is_plan_autonomy(sid).await,
            "autonomy is a session policy, not cleared by plan discard"
        );

        set_plan_autonomy(sid, false).await.unwrap();
        assert!(!is_plan_autonomy(sid).await);
    })
    .await;
}

#[tokio::test]
async fn archive_roundtrip_latest_archived_plan_finds_writer_output() {
    // #16 round 2 regression: `archive_plan_files` trims the leading dot
    // off `.opencrabs_plan_<sid>` when renaming into `archive/`, and
    // `latest_archived_plan_from_path` must prefix-match exactly those
    // dot-less names via the shared `plan_archive_stem`. Between c57de25c
    // and this fix the reader computed its own prefix WITH the dot,
    // matched nothing on disk, and every completed-plan card finalize hit
    // its no-doc branch — zero completion cards posted.
    in_temp_home(async {
        let sid = Uuid::new_v4();
        let mut plan = PlanDocument::new(sid, "Round trip".to_string());
        plan.add_task(task(1, "t1"));
        save_plan(&plan).await.unwrap();

        // The live file is dotted…
        let live = plan_json_path(sid).await;
        assert!(
            live.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.')),
            "live plan file must be dotted, got {}",
            live.display()
        );

        // …the archive name is dot-less…
        crate::utils::plan_files::archive_plan(sid).await.unwrap();
        let names: Vec<String> = std::fs::read_dir(archive_dir(sid).await)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("opencrabs_plan_") && n.ends_with(".json")),
            "archive writer must produce dot-less names, got {names:?}"
        );

        // …and the reader finds it again — the round trip itself.
        let doc = crate::utils::plan_files::latest_archived_plan(sid)
            .await
            .expect(
                "latest_archived_plan must match the writer's dot-less \
                 archive names (#16 round 2)",
            );
        assert_eq!(doc.title, "Round trip");
        assert_eq!(doc.tasks.len(), 1);
    })
    .await;
}
