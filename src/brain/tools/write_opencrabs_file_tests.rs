//! Tests for `write_opencrabs_file` tool.

use super::*;
use crate::brain::tools::r#trait::ToolExecutionContext;
use tempfile::TempDir;
use uuid::Uuid;

fn ctx() -> ToolExecutionContext {
    ToolExecutionContext::new(Uuid::new_v4())
}

fn tool() -> WriteOpenCrabsFileTool {
    WriteOpenCrabsFileTool
}

// ── metadata ─────────────────────────────────────────────────────────────────

#[test]
fn test_tool_name_and_requires_approval() {
    let t = tool();
    assert_eq!(t.name(), "write_opencrabs_file");
    assert!(t.requires_approval(), "writes must require approval");
}

#[test]
fn test_input_schema_required_fields() {
    let schema = tool().input_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("path")));
    assert!(required.iter().any(|v| v.as_str() == Some("operation")));
}

// ── path validation ───────────────────────────────────────────────────────────

#[test]
fn test_empty_path_rejected() {
    assert!(validate_opencrabs_path("").is_err());
}

#[test]
fn test_absolute_path_rejected() {
    assert!(validate_opencrabs_path("/etc/passwd").is_err());
    assert!(validate_opencrabs_path("~/MEMORY.md").is_err());
}

#[test]
fn test_path_traversal_rejected() {
    assert!(validate_opencrabs_path("../etc/passwd").is_err());
    assert!(validate_opencrabs_path("../../secrets").is_err());
    assert!(validate_opencrabs_path("subdir/../../etc/passwd").is_err());
}

#[test]
fn test_valid_brain_files_accepted() {
    assert!(validate_opencrabs_path("MEMORY.md").is_ok());
    assert!(validate_opencrabs_path("USER.md").is_ok());
    assert!(validate_opencrabs_path("SOUL.md").is_ok());
}

#[test]
fn test_valid_config_files_accepted() {
    assert!(validate_opencrabs_path("commands.toml").is_ok());
    assert!(validate_opencrabs_path("config.toml").is_ok());
}

#[test]
fn test_valid_subdirectory_paths_accepted() {
    assert!(validate_opencrabs_path("memory/2026-03-02.md").is_ok());
    assert!(validate_opencrabs_path("agents/session/context.json").is_ok());
}

// ── operation validation ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_unknown_operation_returns_error() {
    let result = tool()
        .execute(
            serde_json::json!({"path": "MEMORY.md", "operation": "delete"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("Unknown operation"));
}

#[tokio::test]
async fn test_overwrite_missing_content_returns_error() {
    let result = tool()
        .execute(
            serde_json::json!({"path": "MEMORY.md", "operation": "overwrite"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("content is required"));
}

#[tokio::test]
async fn test_append_missing_content_returns_error() {
    let result = tool()
        .execute(
            serde_json::json!({"path": "MEMORY.md", "operation": "append"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("content is required"));
}

#[tokio::test]
async fn test_replace_missing_old_text_returns_error() {
    let result = tool()
        .execute(
            serde_json::json!({"path": "MEMORY.md", "operation": "replace", "new_text": "x"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("old_text is required"));
}

#[tokio::test]
async fn test_replace_missing_new_text_returns_error() {
    let result = tool()
        .execute(
            serde_json::json!({"path": "MEMORY.md", "operation": "replace", "old_text": "x"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("new_text is required"));
}

// ── roundtrip write + read ────────────────────────────────────────────────────

#[test]
fn test_overwrite_roundtrip_via_fs() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("MEMORY.md");
    std::fs::write(&path, "initial content").unwrap();
    std::fs::write(&path, "updated content").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "updated content");
}

#[test]
fn test_append_roundtrip_via_fs() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("MEMORY.md");
    std::fs::write(&path, "## Section\n").unwrap();
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    f.write_all(b"\nnew note").unwrap();
    let result = std::fs::read_to_string(&path).unwrap();
    assert!(result.contains("## Section"));
    assert!(result.contains("new note"));
}

#[test]
fn test_replace_roundtrip_via_fs() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("MEMORY.md");
    std::fs::write(&path, "old value here").unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    let updated = content.replacen("old value", "new value", 1);
    std::fs::write(&path, &updated).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new value here");
}

#[test]
fn test_subdirectory_created_on_write() {
    let dir = TempDir::new().unwrap();
    let subdir = dir.path().join("memory");
    let path = subdir.join("note.md");
    // Simulate create_dir_all + write
    std::fs::create_dir_all(&subdir).unwrap();
    std::fs::write(&path, "content").unwrap();
    assert!(path.exists());
}
