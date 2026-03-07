//! A2A Send Tool
//!
//! Agent-callable tool for sending tasks to remote A2A agents.
//! Supports discovery, message/send, tasks/get, and tasks/cancel.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Tool for communicating with remote A2A agents.
#[derive(Default)]
pub struct A2aSendTool;

impl A2aSendTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for A2aSendTool {
    fn name(&self) -> &str {
        "a2a_send"
    }

    fn description(&self) -> &str {
        "Communicate with remote A2A (Agent-to-Agent) agents. \
         Actions: 'discover' to fetch an agent's capabilities, \
         'send' to send a task message, 'get' to check task status, \
         'cancel' to cancel a running task, 'stream' to send and stream response. \
         Requires the remote agent's base URL (e.g. http://192.168.1.10:18790)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["discover", "send", "get", "cancel"],
                    "description": "'discover' to fetch agent card, 'send' to send a task, 'get' to check task status, 'cancel' to cancel a task"
                },
                "url": {
                    "type": "string",
                    "description": "Base URL of the remote A2A agent (e.g. http://127.0.0.1:18790)"
                },
                "message": {
                    "type": "string",
                    "description": "Text message to send to the remote agent (required for 'send')"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (required for 'get' and 'cancel')"
                },
                "context_id": {
                    "type": "string",
                    "description": "Optional context ID for continuing a conversation"
                },
                "api_key": {
                    "type": "string",
                    "description": "Optional Bearer token for authenticated A2A endpoints"
                }
            },
            "required": ["action", "url"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        true
    }

    fn requires_approval_for_input(&self, input: &Value) -> bool {
        // Discovery is read-only, no approval needed
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");
        action != "discover"
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let base_url = match input.get("url").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u.trim_end_matches('/'),
            _ => {
                return Ok(ToolResult::error(
                    "Missing required parameter 'url'.".to_string(),
                ));
            }
        };
        let api_key = input.get("api_key").and_then(|v| v.as_str());

        match action {
            "discover" => discover(base_url, api_key).await,
            "send" => {
                let message = match input.get("message").and_then(|v| v.as_str()) {
                    Some(m) if !m.is_empty() => m,
                    _ => {
                        return Ok(ToolResult::error(
                            "Missing required parameter 'message' for 'send' action.".to_string(),
                        ));
                    }
                };
                let context_id = input.get("context_id").and_then(|v| v.as_str());
                send_message(base_url, api_key, message, context_id).await
            }
            "get" => {
                let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "Missing required parameter 'task_id' for 'get' action.".to_string(),
                        ));
                    }
                };
                get_task(base_url, api_key, task_id).await
            }
            "cancel" => {
                let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "Missing required parameter 'task_id' for 'cancel' action.".to_string(),
                        ));
                    }
                };
                cancel_task(base_url, api_key, task_id).await
            }
            other => Ok(ToolResult::error(format!(
                "Unknown action '{other}'. Valid: discover, send, get, cancel"
            ))),
        }
    }
}

fn auth_headers(api_key: Option<&str>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(key) = api_key
        && let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key))
    {
        headers.insert(reqwest::header::AUTHORIZATION, val);
    }
    headers
}

/// Discover a remote agent's capabilities via their Agent Card.
async fn discover(base_url: &str, api_key: Option<&str>) -> Result<ToolResult> {
    let url = format!("{}/.well-known/agent.json", base_url);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .headers(auth_headers(api_key))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| super::error::ToolError::Execution(format!("HTTP error: {}", e)))?;

    if !resp.status().is_success() {
        return Ok(ToolResult::error(format!(
            "Agent discovery failed: HTTP {}",
            resp.status()
        )));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| super::error::ToolError::Execution(format!("JSON parse error: {}", e)))?;

    let pretty = serde_json::to_string_pretty(&body).unwrap_or_default();
    Ok(ToolResult::success(format!(
        "Agent Card from {}:\n{}",
        base_url, pretty
    )))
}

/// Send a message/task to a remote A2A agent.
async fn send_message(
    base_url: &str,
    api_key: Option<&str>,
    message: &str,
    context_id: Option<&str>,
) -> Result<ToolResult> {
    let url = format!("{}/a2a/v1", base_url);

    let mut msg = serde_json::json!({
        "role": "user",
        "parts": [{"text": message}]
    });
    if let Some(ctx) = context_id {
        msg["contextId"] = serde_json::json!(ctx);
    }

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "message/send",
        "params": {
            "message": msg
        },
        "id": 1
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .headers(auth_headers(api_key))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| super::error::ToolError::Execution(format!("HTTP error: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Ok(ToolResult::error(format!(
            "A2A send failed: HTTP {} — {}",
            status, text
        )));
    }

    let rpc_response: Value = resp
        .json()
        .await
        .map_err(|e| super::error::ToolError::Execution(format!("JSON parse error: {}", e)))?;

    // Check for JSON-RPC error
    if let Some(err) = rpc_response.get("error") {
        let msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Ok(ToolResult::error(format!("A2A error: {}", msg)));
    }

    // Extract task result
    let result = &rpc_response["result"];
    let task_id = result["id"].as_str().unwrap_or("unknown");
    let state = result["status"]["state"].as_str().unwrap_or("unknown");

    // Extract response text from artifacts or status message
    let response_text = extract_response_text(result);

    let mut output = format!("Task: {} (state: {})", task_id, state);
    if let Some(ctx) = result["contextId"].as_str() {
        output.push_str(&format!("\nContext: {}", ctx));
    }
    if !response_text.is_empty() {
        output.push_str(&format!("\n\nResponse:\n{}", response_text));
    }

    Ok(ToolResult::success(output))
}

/// Get the status of a task on a remote A2A agent.
async fn get_task(base_url: &str, api_key: Option<&str>, task_id: &str) -> Result<ToolResult> {
    let url = format!("{}/a2a/v1", base_url);
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tasks/get",
        "params": {
            "id": task_id
        },
        "id": 1
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .headers(auth_headers(api_key))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| super::error::ToolError::Execution(format!("HTTP error: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Ok(ToolResult::error(format!(
            "A2A get failed: HTTP {} — {}",
            status, text
        )));
    }

    let rpc_response: Value = resp
        .json()
        .await
        .map_err(|e| super::error::ToolError::Execution(format!("JSON parse error: {}", e)))?;

    if let Some(err) = rpc_response.get("error") {
        let msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Ok(ToolResult::error(format!("A2A error: {}", msg)));
    }

    let result = &rpc_response["result"];
    let state = result["status"]["state"].as_str().unwrap_or("unknown");
    let response_text = extract_response_text(result);

    let mut output = format!("Task {} — state: {}", task_id, state);
    if !response_text.is_empty() {
        output.push_str(&format!("\n\nResponse:\n{}", response_text));
    }

    Ok(ToolResult::success(output))
}

/// Cancel a task on a remote A2A agent.
async fn cancel_task(base_url: &str, api_key: Option<&str>, task_id: &str) -> Result<ToolResult> {
    let url = format!("{}/a2a/v1", base_url);
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tasks/cancel",
        "params": {
            "id": task_id
        },
        "id": 1
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .headers(auth_headers(api_key))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| super::error::ToolError::Execution(format!("HTTP error: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Ok(ToolResult::error(format!(
            "A2A cancel failed: HTTP {} — {}",
            status, text
        )));
    }

    let rpc_response: Value = resp
        .json()
        .await
        .map_err(|e| super::error::ToolError::Execution(format!("JSON parse error: {}", e)))?;

    if let Some(err) = rpc_response.get("error") {
        let msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Ok(ToolResult::error(format!("A2A error: {}", msg)));
    }

    let state = rpc_response["result"]["status"]["state"]
        .as_str()
        .unwrap_or("unknown");
    Ok(ToolResult::success(format!(
        "Task {} — cancelled (state: {})",
        task_id, state
    )))
}

/// Extract readable text from a task's artifacts and status message.
fn extract_response_text(task: &Value) -> String {
    let mut texts = Vec::new();

    // Check status message
    if let Some(msg) = task.get("status").and_then(|s| s.get("message"))
        && let Some(parts) = msg.get("parts").and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                texts.push(text.to_string());
            }
        }
    }

    // Check artifacts
    if let Some(artifacts) = task.get("artifacts").and_then(|a| a.as_array()) {
        for artifact in artifacts {
            if let Some(parts) = artifact.get("parts").and_then(|p| p.as_array()) {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        texts.push(text.to_string());
                    }
                }
            }
        }
    }

    texts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::tools::r#trait::{Tool, ToolExecutionContext};

    fn ctx() -> ToolExecutionContext {
        ToolExecutionContext::new(uuid::Uuid::new_v4())
    }

    #[test]
    fn test_tool_name_and_schema() {
        let tool = A2aSendTool::new();
        assert_eq!(tool.name(), "a2a_send");
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("action"));
        assert!(props.contains_key("url"));
        assert!(props.contains_key("message"));
        assert!(props.contains_key("task_id"));
        assert!(props.contains_key("api_key"));
    }

    #[test]
    fn test_discover_does_not_require_approval() {
        let tool = A2aSendTool::new();
        let input = serde_json::json!({"action": "discover", "url": "http://localhost:18790"});
        assert!(!tool.requires_approval_for_input(&input));
    }

    #[test]
    fn test_send_requires_approval() {
        let tool = A2aSendTool::new();
        let input =
            serde_json::json!({"action": "send", "url": "http://localhost:18790", "message": "hi"});
        assert!(tool.requires_approval_for_input(&input));
    }

    #[test]
    fn test_cancel_requires_approval() {
        let tool = A2aSendTool::new();
        let input = serde_json::json!({"action": "cancel", "url": "http://localhost:18790", "task_id": "abc"});
        assert!(tool.requires_approval_for_input(&input));
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = A2aSendTool::new();
        let input = serde_json::json!({"action": "discover"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("url"));
    }

    #[tokio::test]
    async fn test_missing_message_for_send() {
        let tool = A2aSendTool::new();
        let input = serde_json::json!({"action": "send", "url": "http://localhost:18790"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("message"));
    }

    #[tokio::test]
    async fn test_missing_task_id_for_get() {
        let tool = A2aSendTool::new();
        let input = serde_json::json!({"action": "get", "url": "http://localhost:18790"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("task_id"));
    }

    #[tokio::test]
    async fn test_missing_task_id_for_cancel() {
        let tool = A2aSendTool::new();
        let input = serde_json::json!({"action": "cancel", "url": "http://localhost:18790"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("task_id"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = A2aSendTool::new();
        let input = serde_json::json!({"action": "invalid", "url": "http://localhost:18790"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("Unknown action")
        );
    }

    #[test]
    fn test_extract_response_text_from_status_message() {
        let task = serde_json::json!({
            "status": {
                "state": "completed",
                "message": {
                    "role": "agent",
                    "parts": [{"text": "Hello from agent"}]
                }
            }
        });
        assert_eq!(extract_response_text(&task), "Hello from agent");
    }

    #[test]
    fn test_extract_response_text_from_artifacts() {
        let task = serde_json::json!({
            "status": {"state": "completed"},
            "artifacts": [{
                "parts": [{"text": "Result A"}, {"text": "Result B"}]
            }]
        });
        assert_eq!(extract_response_text(&task), "Result A\nResult B");
    }

    #[test]
    fn test_extract_response_text_combined() {
        let task = serde_json::json!({
            "status": {
                "state": "completed",
                "message": {"role": "agent", "parts": [{"text": "Status msg"}]}
            },
            "artifacts": [{"parts": [{"text": "Artifact text"}]}]
        });
        assert_eq!(extract_response_text(&task), "Status msg\nArtifact text");
    }

    #[test]
    fn test_extract_response_text_empty() {
        let task = serde_json::json!({"status": {"state": "working"}});
        assert_eq!(extract_response_text(&task), "");
    }

    #[test]
    fn test_auth_headers_with_key() {
        let headers = auth_headers(Some("my-secret"));
        let auth = headers.get(reqwest::header::AUTHORIZATION).unwrap();
        assert_eq!(auth.to_str().unwrap(), "Bearer my-secret");
    }

    #[test]
    fn test_auth_headers_without_key() {
        let headers = auth_headers(None);
        assert!(headers.get(reqwest::header::AUTHORIZATION).is_none());
    }

    #[test]
    fn test_default_impl() {
        let _tool = A2aSendTool;
        assert_eq!(_tool.name(), "a2a_send");
    }
}
