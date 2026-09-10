//! Tests for sub-agent brain context boundary (#145).
//!
//! Sub-agents run lean by default (no pre-injected brain files), but can
//! opt in to full workspace brain via `include_brain: true`.

use crate::brain::tools::Tool;
use crate::brain::tools::subagent::brain::{
    LEAN_BRAIN_CONTEXT_NOTE, READ_ONLY_CAPABILITY_NOTE, brain_status_label, child_prompt,
    child_system_brain,
};
use crate::brain::tools::subagent::{SpawnAgentTool, SubAgentManager, TeamCreateTool, TeamManager};
use crate::config::profile::with_home_override;
use std::fs;
use std::sync::Arc;

#[test]
fn prompt_stacking_lean_full_access() {
    let prompt = child_prompt(false, false, "Fix the bug");
    assert!(
        prompt.contains(LEAN_BRAIN_CONTEXT_NOTE.trim()),
        "lean full-access child must carry the lean context note"
    );
    assert!(
        !prompt.contains(READ_ONLY_CAPABILITY_NOTE.trim()),
        "full-access child must not carry read-only capability note"
    );
    assert!(
        prompt.ends_with("Fix the bug"),
        "task prompt must be at the end"
    );
}

#[test]
fn prompt_stacking_lean_read_only() {
    let prompt = child_prompt(true, false, "Review the PR");
    assert!(
        prompt.contains(READ_ONLY_CAPABILITY_NOTE.trim()),
        "read-only child must carry capability note"
    );
    assert!(
        prompt.contains(LEAN_BRAIN_CONTEXT_NOTE.trim()),
        "lean read-only child must carry lean context note"
    );
    assert!(
        prompt.find(READ_ONLY_CAPABILITY_NOTE.trim()).unwrap()
            < prompt.find(LEAN_BRAIN_CONTEXT_NOTE.trim()).unwrap(),
        "capability note must precede context note"
    );
    assert!(
        prompt.ends_with("Review the PR"),
        "task prompt must be at the end"
    );
}

#[test]
fn prompt_stacking_included_brain() {
    let prompt = child_prompt(false, true, "Implement feature");
    assert!(
        !prompt.contains(LEAN_BRAIN_CONTEXT_NOTE.trim()),
        "child with include_brain=true must NOT carry lean context note"
    );
    assert!(
        !prompt.contains(READ_ONLY_CAPABILITY_NOTE.trim()),
        "full-access child must not carry capability note"
    );
    assert!(prompt.ends_with("Implement feature"));
}

#[test]
fn brain_status_label_reporting() {
    assert_eq!(
        brain_status_label(false),
        "lean (none pre-injected; use load_brain_file)"
    );
    assert_eq!(brain_status_label(true), "core attached (SOUL/USER/AGENTS)");
}

#[test]
fn child_system_brain_none_when_false() {
    let tmp = tempfile::tempdir().unwrap();
    let brain = child_system_brain(false, tmp.path(), None, None);
    assert!(
        brain.is_none(),
        "child_system_brain must return None when include_brain is false"
    );
}

#[test]
fn child_system_brain_loads_core_files_when_true() {
    let home_tmp = tempfile::tempdir().unwrap();
    let work_tmp = tempfile::tempdir().unwrap();

    with_home_override(home_tmp.path().to_path_buf(), || {
        // Seed home brain files
        fs::write(home_tmp.path().join("SOUL.md"), "# SOUL\nBeep boop").unwrap();
        fs::write(home_tmp.path().join("USER.md"), "# USER\nAlexey").unwrap();
        fs::write(home_tmp.path().join("AGENTS.md"), "# AGENTS\nRunbook rules").unwrap();

        // Seed project directive in child workdir
        fs::write(work_tmp.path().join("CLAUDE.md"), "# CLAUDE\nProject rules").unwrap();

        let brain = child_system_brain(
            true,
            work_tmp.path(),
            Some("test-model"),
            Some("test-provider"),
        );
        assert!(
            brain.is_some(),
            "child_system_brain must produce a brain when include_brain is true"
        );
        let brain_str = brain.unwrap();
        assert!(brain_str.contains("Beep boop"), "must contain SOUL.md");
        assert!(brain_str.contains("Alexey"), "must contain USER.md");
        assert!(
            brain_str.contains("Runbook rules"),
            "must contain AGENTS.md"
        );
        assert!(
            brain_str.contains("CLAUDE.md"),
            "must discover project directive (CLAUDE.md) in child working directory"
        );
    });
}

#[test]
fn spawn_agent_schema_exposes_include_brain() {
    let tool = SpawnAgentTool::new(
        Arc::new(SubAgentManager::new()),
        Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let schema = tool.input_schema();
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema must have properties");
    assert!(
        props.contains_key("include_brain"),
        "spawn_agent must expose optional `include_brain` parameter (#145)"
    );
    let include_brain_prop = props.get("include_brain").unwrap();
    assert_eq!(
        include_brain_prop.get("type").and_then(|v| v.as_str()),
        Some("boolean")
    );
}

#[test]
fn team_create_schema_exposes_include_brain_on_agent_items() {
    let tool = TeamCreateTool::new(
        Arc::new(SubAgentManager::new()),
        Arc::new(TeamManager::new()),
        Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let schema = tool.input_schema();
    let agent_props = schema
        .pointer("/properties/agents/items/properties")
        .and_then(|v| v.as_object())
        .expect("agents items must have properties");
    assert!(
        agent_props.contains_key("include_brain"),
        "team_create per-agent item schema must expose `include_brain` (#145)"
    );
}
