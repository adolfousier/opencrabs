//! Tests for `BrainLoader::build_core_brain()`.
//!
//! Verifies the lean-injection model: SOUL.md + USER.md are baked into
//! every request; all other files appear only as an index so the agent can
//! retrieve them on demand via `load_brain_file`.

use super::*;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn loader(dir: &TempDir) -> BrainLoader {
    BrainLoader::new(dir.path().to_path_buf())
}

fn write(dir: &TempDir, name: &str, content: &str) {
    std::fs::write(dir.path().join(name), content).unwrap();
}

// ── core files are injected ───────────────────────────────────────────────────

#[test]
fn test_core_brain_contains_preamble() {
    let dir = TempDir::new().unwrap();
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        brain.contains("You are OpenCrabs"),
        "preamble must always be present"
    );
}

#[test]
fn test_soul_md_is_injected_in_core() {
    let dir = TempDir::new().unwrap();
    write(&dir, "SOUL.md", "Be helpful and precise.");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        brain.contains("Be helpful and precise."),
        "SOUL.md must be injected in core brain"
    );
}

#[test]
fn test_user_md_is_injected_in_core() {
    let dir = TempDir::new().unwrap();
    write(&dir, "USER.md", "Name: Alice\nRole: Engineer");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        brain.contains("Name: Alice"),
        "USER.md must be injected in core brain"
    );
}

// ── contextual files are NOT injected inline ──────────────────────────────────

/// The whole point of the optimisation: contextual files must NOT be baked into
/// every request. The agent should retrieve them via `load_brain_file` only when
/// the request actually needs them.
#[test]
fn test_identity_md_not_injected_in_core_brain() {
    let dir = TempDir::new().unwrap();
    write(&dir, "IDENTITY.md", "I am OpenCrabs the crab.");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        !brain.contains("I am OpenCrabs the crab."),
        "IDENTITY.md content must NOT be injected inline — used only for cron/social sessions"
    );
}

#[test]
fn test_memory_md_not_injected_in_core_brain() {
    let dir = TempDir::new().unwrap();
    write(&dir, "MEMORY.md", "SECRET_PROJECT_NOTES: do not leak.");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        !brain.contains("SECRET_PROJECT_NOTES"),
        "MEMORY.md content must NOT be injected inline — only listed in the memory index"
    );
}

#[test]
fn test_agents_md_not_injected_in_core_brain() {
    let dir = TempDir::new().unwrap();
    write(&dir, "AGENTS.md", "WORKSPACE_RULE: always commit");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        !brain.contains("WORKSPACE_RULE"),
        "AGENTS.md content must NOT be injected inline"
    );
}

#[test]
fn test_tools_md_not_injected_in_core_brain() {
    let dir = TempDir::new().unwrap();
    write(&dir, "TOOLS.md", "TOOL_NOTE: use cargo test");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        !brain.contains("TOOL_NOTE"),
        "TOOLS.md content must NOT be injected inline"
    );
}

#[test]
fn test_security_md_not_injected_in_core_brain() {
    let dir = TempDir::new().unwrap();
    write(&dir, "SECURITY.md", "POLICY: never expose keys");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        !brain.contains("POLICY: never expose keys"),
        "SECURITY.md content must NOT be injected inline"
    );
}

#[test]
fn test_code_md_not_injected_in_core_brain() {
    let dir = TempDir::new().unwrap();
    write(&dir, "CODE.md", "RULE: use snake_case");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        !brain.contains("RULE: use snake_case"),
        "CODE.md content must NOT be injected inline"
    );
}

// ── memory index is present when contextual files exist ──────────────────────

/// The memory index tells the agent WHAT is available to retrieve. Without it,
/// the agent would not know to call `load_brain_file`.
#[test]
fn test_memory_index_present_when_contextual_files_exist() {
    let dir = TempDir::new().unwrap();
    write(&dir, "MEMORY.md", "some notes");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        brain.contains("Available Context Files"),
        "memory index section must appear when contextual files exist"
    );
    assert!(
        brain.contains("load_brain_file"),
        "memory index must mention the load_brain_file tool"
    );
}

#[test]
fn test_memory_index_lists_existing_files_only() {
    let dir = TempDir::new().unwrap();
    write(&dir, "IDENTITY.md", "I am a crab.");
    // MEMORY.md does NOT exist
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        brain.contains("IDENTITY.md"),
        "index must list IDENTITY.md (exists)"
    );
    assert!(
        !brain.contains("MEMORY.md"),
        "index must NOT list MEMORY.md (does not exist)"
    );
}

#[test]
fn test_memory_index_absent_when_no_contextual_files_exist() {
    let dir = TempDir::new().unwrap();
    // Only SOUL.md and USER.md — no contextual files
    write(&dir, "SOUL.md", "I am a crab.");
    write(&dir, "USER.md", "Name: Alice");
    let brain = loader(&dir).build_core_brain(None, None);
    assert!(
        !brain.contains("Available Context Files"),
        "memory index must be absent when no contextual files exist"
    );
}

// ── on-demand guidance is present ────────────────────────────────────────────

#[test]
fn test_load_guidance_tells_agent_when_to_retrieve() {
    let dir = TempDir::new().unwrap();
    write(&dir, "MEMORY.md", "notes");
    write(&dir, "IDENTITY.md", "identity");
    let brain = loader(&dir).build_core_brain(None, None);
    // Agent must know WHEN to call load_brain_file
    assert!(
        brain.contains("Load proactively when"),
        "brain must contain guidance on when to load contextual files"
    );
}

// ── runtime info ─────────────────────────────────────────────────────────────

#[test]
fn test_runtime_info_included_in_core_brain() {
    let dir = TempDir::new().unwrap();
    let info = RuntimeInfo {
        model: Some("claude-sonnet-4-6".to_string()),
        provider: Some("anthropic".to_string()),
        working_directory: Some("/home/user/project".to_string()),
    };
    let brain = loader(&dir).build_core_brain(Some(&info), None);
    assert!(brain.contains("claude-sonnet-4-6"));
    assert!(brain.contains("anthropic"));
    assert!(brain.contains("/home/user/project"));
}

// ── slash commands ────────────────────────────────────────────────────────────

#[test]
fn test_slash_commands_included_in_core_brain() {
    let dir = TempDir::new().unwrap();
    let commands = "/help - show help\n/clear - clear screen\n";
    let brain = loader(&dir).build_core_brain(None, Some(commands));
    assert!(
        brain.contains("/help"),
        "slash commands must be present in core brain"
    );
}

// ── full brain still works (backwards compat) ─────────────────────────────────

#[test]
fn test_full_brain_still_injects_all_files() {
    let dir = TempDir::new().unwrap();
    write(&dir, "SOUL.md", "core soul");
    write(&dir, "USER.md", "Name: Alice");
    write(&dir, "MEMORY.md", "long term memory content");
    let brain = loader(&dir).build_system_brain(None, None);
    assert!(
        brain.contains("Name: Alice"),
        "full brain must include USER.md"
    );
    assert!(
        brain.contains("long term memory content"),
        "full brain must include MEMORY.md"
    );
}

// ── core vs full comparison ───────────────────────────────────────────────────

/// Demonstrates the token saving: the core brain must be strictly smaller
/// than the full brain when contextual files are populated.
#[test]
fn test_core_brain_is_smaller_than_full_brain_when_contextual_files_exist() {
    let dir = TempDir::new().unwrap();
    write(&dir, "SOUL.md", "I am a crab.");
    write(&dir, "USER.md", "Name: Alice\n".repeat(100).as_str()); // 1 200 chars
    write(&dir, "MEMORY.md", "project notes\n".repeat(200).as_str()); // 2 800 chars
    write(&dir, "AGENTS.md", "workspace rules\n".repeat(50).as_str());

    let core_len = loader(&dir).build_core_brain(None, None).len();
    let full_len = loader(&dir).build_system_brain(None, None).len();

    assert!(
        core_len < full_len,
        "core brain ({} bytes) must be smaller than full brain ({} bytes)",
        core_len,
        full_len
    );
}
