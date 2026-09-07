use crate::brain::tools::Tool;
use crate::brain::tools::ToolExecutionContext;
use crate::brain::tools::config_tool::*;
use tokio;

#[test]
fn test_tool_metadata() {
    let tool = ConfigTool;
    assert_eq!(tool.name(), "config_manager");
    assert!(tool.requires_approval());
}

#[tokio::test]
async fn test_unknown_operation() {
    let tool = ConfigTool;
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    let result = tool
        .execute(serde_json::json!({"operation": "nope"}), &ctx)
        .await
        .unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("Unknown operation"));
}

#[tokio::test]
async fn test_read_config() {
    let tool = ConfigTool;
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    let result = tool
        .execute(
            serde_json::json!({"operation": "read_config", "section": "agent"}),
            &ctx,
        )
        .await
        .unwrap();
    // Should succeed even with default config
    assert!(result.success);
    assert!(result.output.contains("approval_policy"));
}

/// Every section the resolver resolves must render (#86).
///
/// The render match once carried only 8 arms, so 8 known sections
/// (memory, brain, browser, cron, daemon, doctor, image, a2a — plus tui
/// later) fell into the catch-all error arm: the error named the section
/// unknown while listing it among the valid ones. This pins the contract:
/// every name in CONFIG_SECTIONS renders successfully, never errors.
#[tokio::test]
async fn test_read_config_renders_every_known_section() {
    let tool = ConfigTool;
    for section in crate::config::sections::CONFIG_SECTIONS {
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(
                serde_json::json!({"operation": "read_config", "section": section}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            result.success,
            "read_config section '{section}' must render, got error: {:?}",
            result.error
        );
        assert!(
            !result.output.is_empty(),
            "read_config section '{section}' rendered empty output"
        );
    }
}

/// Shorthand children resolve to their parent and render (#86 contract).
#[tokio::test]
async fn test_read_config_shorthand_children_render() {
    let tool = ConfigTool;
    for (child, parent) in crate::config::sections::SECTION_PARENTS {
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(
                serde_json::json!({"operation": "read_config", "section": child}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            result.success,
            "read_config shorthand '{child}' (parent {parent}) must render, got: {:?}",
            result.error
        );
    }
}

#[tokio::test]
async fn test_read_commands_empty() {
    let tool = ConfigTool;
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    let result = tool
        .execute(serde_json::json!({"operation": "read_commands"}), &ctx)
        .await
        .unwrap();
    assert!(result.success);
}

#[tokio::test]
async fn test_write_config_missing_fields() {
    let tool = ConfigTool;
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    let result = tool
        .execute(serde_json::json!({"operation": "write_config"}), &ctx)
        .await
        .unwrap();
    assert!(!result.success);
}
