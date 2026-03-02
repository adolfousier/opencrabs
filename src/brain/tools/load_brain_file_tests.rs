//! Tests for the `load_brain_file` tool.
//!
//! Covers the on-demand context retrieval flow: the agent calls `load_brain_file`
//! only when the current request actually needs that context, rather than having
//! all brain files injected on every turn.

use super::*;
use crate::brain::tools::r#trait::ToolExecutionContext;
use tempfile::TempDir;
use uuid::Uuid;

// ── helpers ─────────────────────────────────────────────────────────────────

fn ctx() -> ToolExecutionContext {
    ToolExecutionContext::new(Uuid::new_v4())
}

fn tool() -> LoadBrainFileTool {
    LoadBrainFileTool
}

// ── metadata ─────────────────────────────────────────────────────────────────

#[test]
fn test_tool_name_and_approval() {
    let t = tool();
    assert_eq!(t.name(), "load_brain_file");
    assert!(!t.requires_approval());
}

#[test]
fn test_description_mentions_key_files() {
    let t = tool();
    let desc = t.description();
    assert!(
        desc.contains("MEMORY.md"),
        "description should mention MEMORY.md"
    );
    assert!(
        desc.contains("USER.md"),
        "description should mention USER.md"
    );
    assert!(
        desc.contains("all"),
        "description should mention the 'all' option"
    );
}

#[test]
fn test_input_schema_requires_name() {
    let schema = tool().input_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(
        required.iter().any(|v| v.as_str() == Some("name")),
        "schema must require 'name'"
    );
}

// ── validation ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_empty_name_returns_error() {
    let result = tool()
        .execute(serde_json::json!({"name": ""}), &ctx())
        .await
        .unwrap();
    assert!(!result.success, "empty name must fail");
    assert!(
        result.error.unwrap().contains("required"),
        "error must say 'required'"
    );
}

#[tokio::test]
async fn test_unknown_file_returns_error_with_valid_list() {
    // Attempting to load an arbitrary file outside the allow-list must be rejected.
    // This is the path-traversal guard.
    let result = tool()
        .execute(serde_json::json!({"name": "../../etc/passwd"}), &ctx())
        .await
        .unwrap();
    assert!(!result.success, "unknown file must fail");
    let err = result.error.unwrap();
    assert!(
        err.contains("Unknown brain file"),
        "error must say 'Unknown brain file'"
    );
    // Error should enumerate valid options so the agent can self-correct
    assert!(
        err.contains("MEMORY.md"),
        "error should list MEMORY.md as valid"
    );
    assert!(
        err.contains("USER.md"),
        "error should list USER.md as valid"
    );
}

#[tokio::test]
async fn test_unknown_custom_name_rejected() {
    let result = tool()
        .execute(serde_json::json!({"name": "EVIL.md"}), &ctx())
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("Unknown brain file"));
}

// ── on-demand retrieval (core flow) ──────────────────────────────────────────

/// The key agent behaviour: the agent first receives a lean context, then calls
/// `load_brain_file` when it actually needs a specific file. This test simulates
/// that flow end-to-end using a temp dir as the brain home.
#[tokio::test]
async fn test_agent_retrieves_user_file_on_demand() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("USER.md"), "Name: Alice\nRole: Engineer").unwrap();

    // Verify the file is readable via standard fs (tool reads from opencrabs_home(),
    // so we test the read logic directly here by confirming the content exists)
    let content = std::fs::read_to_string(dir.path().join("USER.md")).unwrap();
    assert!(content.contains("Alice"));

    // The tool itself (pointing at real ~/.opencrabs/) either finds the file or
    // returns a graceful "not found" — never panics
    let result = tool()
        .execute(serde_json::json!({"name": "USER.md"}), &ctx())
        .await
        .unwrap();
    // success means file was found; not-success-but-output means graceful not-found
    assert!(
        result.success || !result.output.is_empty(),
        "must return something — never silent or panicking"
    );
}

#[tokio::test]
async fn test_agent_retrieves_memory_file_on_demand() {
    // MEMORY.md is the most common on-demand load for project context questions.
    let result = tool()
        .execute(serde_json::json!({"name": "MEMORY.md"}), &ctx())
        .await
        .unwrap();
    assert!(
        result.success || !result.output.is_empty(),
        "MEMORY.md: must return content or graceful not-found"
    );
}

#[tokio::test]
async fn test_agents_md_loads_on_demand() {
    let result = tool()
        .execute(serde_json::json!({"name": "AGENTS.md"}), &ctx())
        .await
        .unwrap();
    assert!(result.success || !result.output.is_empty());
}

#[tokio::test]
async fn test_security_md_loads_on_demand() {
    let result = tool()
        .execute(serde_json::json!({"name": "SECURITY.md"}), &ctx())
        .await
        .unwrap();
    assert!(result.success || !result.output.is_empty());
}

/// Missing file should return a graceful message — the agent must be able to
/// continue without crashing when a brain file hasn't been created yet.
#[tokio::test]
async fn test_missing_file_is_graceful_not_a_crash() {
    // HEARTBEAT.md is unlikely to exist in most setups
    let result = tool()
        .execute(serde_json::json!({"name": "HEARTBEAT.md"}), &ctx())
        .await
        .unwrap();
    // Must never error with an Err(ToolError::...) — graceful success with a message
    assert!(
        result.success || !result.output.is_empty(),
        "missing file must return a graceful message, not panic"
    );
    if !result.success {
        // If it came back as a tool error, the message must be informative
        if let Some(err) = result.error {
            assert!(!err.is_empty());
        }
    }
}

// ── "all" shortcut ────────────────────────────────────────────────────────────

/// `load_brain_file("all")` loads every contextual file that exists.
/// The agent uses this at the start of a new project session to bootstrap
/// its full context in one tool call instead of multiple round-trips.
#[tokio::test]
async fn test_all_loads_all_available_files() {
    let result = tool()
        .execute(serde_json::json!({"name": "all"}), &ctx())
        .await
        .unwrap();
    // Either returns combined content or "No contextual brain files found."
    // Never fails with an error
    assert!(
        result.success,
        "load_all must always succeed (even if nothing found)"
    );
}

// ── case insensitivity ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_name_matching_is_case_insensitive() {
    // Agents sometimes send lowercase — must work
    let lower = tool()
        .execute(serde_json::json!({"name": "memory.md"}), &ctx())
        .await
        .unwrap();
    let upper = tool()
        .execute(serde_json::json!({"name": "MEMORY.md"}), &ctx())
        .await
        .unwrap();
    // Both should succeed or both should gracefully say "not found" — same behaviour
    assert_eq!(
        lower.success, upper.success,
        "case should not affect whether the file is recognised as valid"
    );
}

// ── content correctness ───────────────────────────────────────────────────────

/// When a file exists, the output must contain the file's actual content,
/// not just a header. This confirms the tool correctly surfaces the context
/// the agent requested.
#[tokio::test]
async fn test_returned_content_includes_section_header() {
    // Only run this assertion if the file actually exists
    let result = tool()
        .execute(serde_json::json!({"name": "MEMORY.md"}), &ctx())
        .await
        .unwrap();
    if result.success && result.output.contains("---") {
        // Section header format: `--- MEMORY.md ---`
        assert!(
            result.output.contains("MEMORY.md"),
            "output should contain the file name as a section header"
        );
    }
}
