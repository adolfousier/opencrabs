//! Tasks List Tool (#1160)
//!
//! One agent-facing roster of BOTH background systems: spawned sub-agents
//! (previously visible only through wait_agent's error path) and detached
//! bash commands (previously invisible to the model entirely). Both managers
//! already ride in `ToolExecutionContext`, so this tool only reads.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolHints, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Tool listing in-flight sub-agents and detached commands.
#[derive(Default)]
pub struct TasksListTool;

impl TasksListTool {
    pub fn new() -> Self {
        Self
    }
}

/// One sub-agent roster row, render-ready.
pub(crate) struct SubagentRow {
    pub id: String,
    pub label: String,
    pub state: String,
    /// Path of the agent's JSON status file, as built by
    /// [`subagent_status_file`], so the model can read live progress directly.
    pub status_file: Option<String>,
}

/// Advertised status-file path for a sub-agent — the same path writers use.
///
/// Resolved through [`crate::brain::agent::service::work_status`], which is
/// where every writer persists (`subagent/spawn.rs` → `work_status::status_path`).
/// The pre-#26 `subagent::status` helper resolves to `<home>/tmp/subagents`,
/// a directory nothing creates, so advertising a path through it handed the
/// model a file that always read ENOENT — and an empty read from a wrong path
/// is indistinguishable from "this sub-agent never existed" (#165).
///
/// Extracted from `execute()` so the advertised path is unit-testable; while
/// it was computed inline, no test could pin it and the wrong-path defect
/// survived (#165).
pub(crate) fn subagent_status_file(id: &str) -> String {
    crate::brain::agent::service::work_status::status_path(id)
        .display()
        .to_string()
}

/// One detached-command roster row.
pub(crate) struct DetachedRow {
    pub label: String,
    pub elapsed_secs: u64,
}

/// Pure renderer so tests pin output shape without live managers.
///
/// Empty everything renders the explicit "No background tasks." line — the
/// model must never have to infer emptiness from absent sections.
pub(crate) fn render_tasks(subagents: &[SubagentRow], detached: &[DetachedRow]) -> String {
    if subagents.is_empty() && detached.is_empty() {
        return "No background tasks.".to_string();
    }
    let mut out = String::from("Background tasks:");
    if !subagents.is_empty() {
        out.push_str(&format!("\n\nSub-agents ({}):", subagents.len()));
        for a in subagents {
            out.push_str(&format!("\n- {} [{}] {}", a.id, a.label, a.state));
            if let Some(sf) = &a.status_file {
                out.push_str(&format!("\n  status file: {sf}"));
            }
        }
    }
    if !detached.is_empty() {
        out.push_str(&format!("\n\nDetached commands ({}):", detached.len()));
        for d in detached {
            out.push_str(&format!("\n- {} (elapsed {}s)", d.label, d.elapsed_secs));
        }
    }
    out
}

fn state_label(state: &super::subagent::manager::SubAgentState) -> &'static str {
    use super::subagent::manager::SubAgentState as S;
    match state {
        S::Running => "running",
        S::AwaitingInput => "awaiting input",
        S::Completed => "completed",
        S::Failed(_) => "failed",
        S::Cancelled => "cancelled",
    }
}

#[async_trait]
impl Tool for TasksListTool {
    fn name(&self) -> &str {
        "tasks_list"
    }

    fn description(&self) -> &str {
        "List in-flight background work: spawned sub-agents (id, label, \
         state, status-file path) and detached shell commands (label, \
         elapsed). Read-only. Use it to check what is running instead of \
         re-spawning or busy-waiting; results are pushed to you on \
         completion either way."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
        }
    }

    async fn execute(&self, _input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let mut subagents = Vec::new();
        if let Some(mgr) = context.subagent_manager.as_ref() {
            // Scoped to the CALLER's children (#191). The manager is
            // process-global (one instance per channel factory), so an
            // unfiltered `list()` reported other sessions' sub-agents as this
            // caller's in-flight work — and this tool's framing ("do not
            // spawn duplicates") makes that a silent suppressor, not just
            // noise. The detached half below is already scoped the same way.
            for (id, label, state) in mgr.list_for_parent(context.session_id) {
                let status_file = subagent_status_file(&id);
                subagents.push(SubagentRow {
                    id,
                    label,
                    state: state_label(&state).to_string(),
                    status_file: Some(status_file),
                });
            }
        }

        let mut detached = Vec::new();
        if let Some(bm) = context.background_manager.as_ref() {
            for t in bm.running_tasks(context.session_id) {
                detached.push(DetachedRow {
                    label: t.label,
                    elapsed_secs: t.started.elapsed().as_secs(),
                });
            }
        }

        Ok(ToolResult::success(render_tasks(&subagents, &detached)))
    }
}
