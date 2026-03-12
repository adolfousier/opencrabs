//! Tests for tool name normalization.
//!
//! Some providers (e.g. MiniMax) hallucinate tool names like "Plan: complete_task"
//! instead of sending tool="plan" with operation="complete_task". The normalizer
//! recovers the intended call so it doesn't fail with "Tool not found".

use super::*;

#[test]
fn plan_complete_task_normalized() {
    let input = serde_json::json!({"task_order": 4, "output": "done", "success": true});
    let (name, result) = AgentService::normalize_tool_call("Plan: complete_task".into(), input);
    assert_eq!(name, "plan");
    assert_eq!(result["operation"], "complete_task");
    assert_eq!(result["task_order"], 4);
}

#[test]
fn plan_summary_normalized() {
    let input = serde_json::json!({});
    let (name, result) = AgentService::normalize_tool_call("Plan: summary".into(), input);
    assert_eq!(name, "plan");
    assert_eq!(result["operation"], "summary");
}

#[test]
fn plan_start_task_normalized() {
    let input = serde_json::json!({"task_order": 1});
    let (name, result) = AgentService::normalize_tool_call("Plan: start_task".into(), input);
    assert_eq!(name, "plan");
    assert_eq!(result["operation"], "start_task");
}

#[test]
fn lowercase_plan_prefix_normalized() {
    let input = serde_json::json!({});
    let (name, _) = AgentService::normalize_tool_call("plan: complete_task".into(), input);
    assert_eq!(name, "plan");
}

#[test]
fn plan_no_space_after_colon() {
    let input = serde_json::json!({});
    let (name, result) = AgentService::normalize_tool_call("Plan:summary".into(), input);
    assert_eq!(name, "plan");
    assert_eq!(result["operation"], "summary");
}

#[test]
fn existing_operation_not_overwritten() {
    // If input already has "operation", normalization should not overwrite it
    let input = serde_json::json!({"operation": "add_task", "title": "New task"});
    let (name, result) = AgentService::normalize_tool_call("Plan: complete_task".into(), input);
    assert_eq!(name, "plan");
    assert_eq!(
        result["operation"], "add_task",
        "existing operation must not be overwritten"
    );
}

#[test]
fn normal_tool_name_unchanged() {
    let input = serde_json::json!({"command": "ls"});
    let (name, result) = AgentService::normalize_tool_call("bash".into(), input.clone());
    assert_eq!(name, "bash");
    assert_eq!(result, input);
}

#[test]
fn plan_tool_unchanged() {
    let input = serde_json::json!({"operation": "complete_task", "task_order": 1});
    let (name, result) = AgentService::normalize_tool_call("plan".into(), input.clone());
    assert_eq!(name, "plan");
    assert_eq!(result, input);
}

#[test]
fn generic_colon_prefix_normalized() {
    // Handles other tools that might get hallucinated the same way
    let input = serde_json::json!({"query": "test"});
    let (name, result) = AgentService::normalize_tool_call("Memory: search".into(), input);
    assert_eq!(name, "memory");
    assert_eq!(result["operation"], "search");
}

#[test]
fn spaces_in_operation_become_underscores() {
    let input = serde_json::json!({});
    let (name, result) = AgentService::normalize_tool_call("Plan: complete task".into(), input);
    assert_eq!(name, "plan");
    assert_eq!(result["operation"], "complete_task");
}
