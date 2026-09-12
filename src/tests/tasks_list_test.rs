//! #1160 tasks_list + detached status files + prompt rot-guard.

use crate::brain::agent::service::work_status::{self, test_override};
use crate::brain::tools::Tool;
use crate::brain::tools::tasks_list::{DetachedRow, SubagentRow, TasksListTool, render_tasks};
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn schema_has_no_params_and_tool_is_read_only() {
    let tool = TasksListTool::new();
    assert_eq!(tool.name(), "tasks_list");
    assert_eq!(
        tool.input_schema()["properties"].as_object().unwrap().len(),
        0
    );
    assert!(tool.hints().read_only);
}

#[test]
fn render_empty_roster_says_so_explicitly() {
    let out = render_tasks(&[], &[]);
    assert_eq!(out, "No background tasks.");
}

#[test]
fn render_lists_both_systems_with_states_and_pointers() {
    let subs = vec![SubagentRow {
        id: "agt-1".into(),
        label: "research".into(),
        state: "running".into(),
        status_file: Some("/tmp/detached/agt-1.json".into()),
    }];
    let det = vec![DetachedRow {
        label: "cargo test".into(),
        elapsed_secs: 42,
    }];
    let out = render_tasks(&subs, &det);
    assert!(out.contains("Sub-agents (1)"), "was: {out}");
    assert!(out.contains("- agt-1 [research] running"), "was: {out}");
    assert!(out.contains("status file: /tmp/detached/agt-1.json"));
    assert!(out.contains("Detached commands (1)"), "was: {out}");
    assert!(out.contains("- cargo test (elapsed 42s)"), "was: {out}");
}

/// Gap 2: a detached command's status file exists mid-run with spawn data,
/// and gains exit info on completion. Since #26 the record lives in the
/// unified `work_status` schema (`kind: command`, state machine, `finish`).
#[test]
fn detached_status_file_written_then_finished() {
    let dir = TempDir::new().unwrap();
    test_override::set(dir.path().join("detached"));

    let task_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    work_status::WorkStatus::new_command(
        &task_id.to_string(),
        &session_id.to_string(),
        "cargo test",
        "cargo test --lib",
    )
    .unwrap();
    let path = dir.path().join("detached").join(format!("{task_id}.json"));
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"cargo test\""), "was: {raw}");
    assert!(raw.contains(&session_id.to_string()), "was: {raw}");
    assert!(raw.contains("\"kind\": \"command\""), "was: {raw}");
    assert!(!raw.contains("\"finish\""), "mid-run must be unfinished");

    work_status::WorkStatus::finish_command(
        &task_id.to_string(),
        &session_id.to_string(),
        "cargo test",
        "cargo test --lib",
        work_status::CommandExit {
            success: true,
            code: 0,
            elapsed_secs: 99.5,
            output_bytes: 2048,
        },
    )
    .unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"success\": true"), "was: {raw}");
    assert!(raw.contains("\"output_bytes\": 2048"), "was: {raw}");
    assert!(raw.contains("\"state\": \"Completed\""), "was: {raw}");
}

/// Gap 3 rot-guard: the LONG TASKS paragraph must keep covering sub-agents,
/// not only bash — it regressed to bash-only once already (#762).
#[test]
fn prompt_builder_keeps_subagent_background_contract() {
    let dir = TempDir::new().unwrap();
    let prompt = crate::brain::prompt_builder::BrainLoader::new(dir.path().to_path_buf())
        .build_system_brain(None);
    assert!(
        prompt.contains("spawned agents run in the background"),
        "subagent background contract missing from system prompt"
    );
}

/// #191 regression pin: `execute` reports only the CALLER's sub-agents.
///
/// The manager is process-global (one instance per channel factory), so
/// pre-fix this iterated every session's children. That is not merely noise:
/// the tool's framing tells the model these are *its* in-flight sub-agents
/// ("do not spawn duplicates"), so a foreign row makes a lane silently skip
/// work it believes is already running.
///
/// Pins the call site rather than the renderer — `render_tasks` takes rows the
/// caller built, so it cannot see a scope regression.
#[tokio::test]
async fn execute_lists_only_the_callers_subagents() {
    use crate::brain::tools::ToolExecutionContext;
    use crate::brain::tools::subagent::{SubAgent, SubAgentManager};
    use std::sync::Arc;

    fn child(id: &str, label: &str, parent: Uuid) -> SubAgent {
        SubAgent::new(id.to_string(), label.to_string(), Uuid::new_v4(), parent)
    }

    let me = Uuid::from_u128(0x191);
    let other = Uuid::from_u128(0x192);

    let mgr = Arc::new(SubAgentManager::new());
    mgr.insert(child("mine0001", "my-research", me));
    mgr.insert(child("theirs01", "their-research", other));

    let mut ctx = ToolExecutionContext::new(me);
    ctx.subagent_manager = Some(mgr);

    let out = TasksListTool::new()
        .execute(serde_json::json!({}), &ctx)
        .await
        .unwrap();
    assert!(out.success, "tasks_list failed: {:?}", out.error);
    assert!(
        out.output.contains("mine0001"),
        "own child missing: {}",
        out.output
    );
    assert!(
        !out.output.contains("theirs01"),
        "another session's child leaked into the roster: {}",
        out.output
    );
}
