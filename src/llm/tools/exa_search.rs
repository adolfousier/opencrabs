//! EXA Search Tool
//!
//! Perform real-time internet searches using the EXA AI search API.
//! Supports two modes:
//! - **MCP mode (default):** Free, no API key — uses hosted MCP endpoint at `mcp.exa.ai`
//! - **Direct API mode:** When `EXA_API_KEY` is set — higher rate limits

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

const MCP_ENDPOINT: &str = "https://mcp.exa.ai/mcp";
const MCP_PROTOCOL_VERSION: &str = "2025-03-26";

/// EXA search tool — works out of the box via free MCP endpoint.
/// Set `EXA_API_KEY` for direct API access with higher rate limits.
pub struct ExaSearchTool {
    api_key: Option<String>,
    mcp_session_id: Arc<RwLock<Option<String>>>,
}

impl ExaSearchTool {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            mcp_session_id: Arc::new(RwLock::new(None)),
        }
    }

    fn use_mcp(&self) -> bool {
        self.api_key.is_none()
    }

    /// Initialize an MCP session and return the session ID.
    async fn init_mcp_session(&self, client: &reqwest::Client) -> Result<String> {
        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "opencrabs",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });

        let response = client
            .post(MCP_ENDPOINT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&init_request)
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("MCP initialize failed: {}", e)))?;

        // Capture session ID from response header
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                ToolError::Execution("MCP server did not return session ID".to_string())
            })?;

        // Consume the response body (we don't need the init result)
        let _body = response.text().await.unwrap_or_default();

        // Send initialized notification (required by MCP protocol)
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });

        let _notif_resp = client
            .post(MCP_ENDPOINT)
            .header("Content-Type", "application/json")
            .header("Mcp-Session-Id", &session_id)
            .json(&notification)
            .send()
            .await
            .map_err(|e| {
                ToolError::Execution(format!("MCP initialized notification failed: {}", e))
            })?;

        // Store session ID
        *self.mcp_session_id.write().await = Some(session_id.clone());

        Ok(session_id)
    }

    /// Get or create an MCP session.
    async fn ensure_mcp_session(&self, client: &reqwest::Client) -> Result<String> {
        // Check existing session
        if let Some(ref id) = *self.mcp_session_id.read().await {
            return Ok(id.clone());
        }
        self.init_mcp_session(client).await
    }

    /// Execute search via free hosted MCP endpoint.
    async fn execute_via_mcp(&self, query: &str, num_results: usize) -> Result<ToolResult> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ToolError::Execution(format!("Failed to create HTTP client: {}", e)))?;

        // Try with existing session, re-init on 404
        let result = self
            .try_mcp_tool_call(&client, query, num_results)
            .await;

        match result {
            Ok(tool_result) => Ok(tool_result),
            Err(ToolError::Execution(msg)) if msg.contains("404") || msg.contains("session") => {
                // Session expired — re-initialize
                tracing::info!("MCP session expired, re-initializing");
                *self.mcp_session_id.write().await = None;
                self.try_mcp_tool_call(&client, query, num_results).await
            }
            Err(e) => Err(e),
        }
    }

    /// Perform a single MCP tools/call request.
    async fn try_mcp_tool_call(
        &self,
        client: &reqwest::Client,
        query: &str,
        num_results: usize,
    ) -> Result<ToolResult> {
        let session_id = self.ensure_mcp_session(client).await?;

        let tool_call = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "web_search_exa",
                "arguments": {
                    "query": query,
                    "numResults": num_results
                }
            }
        });

        let response = client
            .post(MCP_ENDPOINT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Session-Id", &session_id)
            .json(&tool_call)
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("MCP tool call failed: {}", e)))?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Err(ToolError::Execution(
                "MCP session expired (404)".to_string(),
            ));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Ok(ToolResult::error(format!(
                "EXA MCP search failed with status {}: {}",
                status, body
            )));
        }

        // Parse response — handle both JSON and SSE
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body_text = response.text().await.map_err(|e| {
            ToolError::Execution(format!("Failed to read MCP response body: {}", e))
        })?;

        let json_body = if content_type.contains("text/event-stream") {
            // Parse SSE: extract last "data: " line with a JSON-RPC response
            Self::parse_sse_response(&body_text)?
        } else {
            serde_json::from_str::<Value>(&body_text).map_err(|e| {
                ToolError::Execution(format!("Failed to parse MCP JSON response: {}", e))
            })?
        };

        // Extract result text from JSON-RPC response
        Self::extract_mcp_result(&json_body, query)
    }

    /// Parse SSE response body into the last JSON-RPC message.
    fn parse_sse_response(body: &str) -> Result<Value> {
        let mut last_json = None;
        for line in body.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data: ")
                && let Ok(parsed) = serde_json::from_str::<Value>(data)
                && parsed.get("id").is_some()
            {
                last_json = Some(parsed);
            }
        }
        last_json
            .ok_or_else(|| ToolError::Execution("No JSON-RPC response found in SSE stream".to_string()))
    }

    /// Extract the text result from a JSON-RPC tools/call response.
    fn extract_mcp_result(json: &Value, query: &str) -> Result<ToolResult> {
        // Check for JSON-RPC error
        if let Some(error) = json.get("error") {
            let msg = error
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown MCP error");
            return Ok(ToolResult::error(format!("EXA MCP error: {}", msg)));
        }

        // Check for tool execution error
        let result = json.get("result").ok_or_else(|| {
            ToolError::Execution("MCP response missing 'result' field".to_string())
        })?;

        if result.get("isError") == Some(&Value::Bool(true)) {
            let error_text = result
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("Unknown error");
            return Ok(ToolResult::error(format!("EXA search error: {}", error_text)));
        }

        // Extract content text
        let text = result
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("No results returned");

        let mut output = format!("Search results for: \"{}\"\n\n{}", query, text);
        if output.ends_with('\n') {
            // Already has trailing newline
        } else {
            output.push('\n');
        }

        Ok(ToolResult::success(output))
    }

    /// Execute search via direct EXA API (requires API key).
    async fn execute_via_api(&self, input: &ExaSearchInput) -> Result<ToolResult> {
        let api_key = self.api_key.as_deref().ok_or_else(|| {
            ToolError::Execution("Direct API mode requires EXA_API_KEY".to_string())
        })?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| ToolError::Execution(format!("Failed to create HTTP client: {}", e)))?;

        let body = serde_json::json!({
            "query": input.query,
            "num_results": input.max_results,
            "type": input.search_type,
            "contents": {
                "text": true
            }
        });

        let response = client
            .post("https://api.exa.ai/search")
            .header("x-api-key", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("EXA search request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Ok(ToolResult::error(format!(
                "EXA search failed with status {}: {}",
                status, body
            )));
        }

        let exa_response: ExaResponse = response
            .json()
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to parse EXA response: {}", e)))?;

        let mut output = format!("Search results for: \"{}\"\n\n", input.query);

        if exa_response.results.is_empty() {
            output.push_str("No results found. Try rephrasing your query.\n");
        } else {
            for (i, result) in exa_response.results.iter().enumerate() {
                let title = result.title.as_deref().unwrap_or("Untitled");
                output.push_str(&format!("{}. {}\n", i + 1, title));
                output.push_str(&format!("   URL: {}\n", result.url));
                if let Some(text) = &result.text {
                    let snippet: String = text.chars().take(300).collect();
                    output.push_str(&format!("   {}\n", snippet));
                }
                output.push('\n');
            }
        }

        Ok(ToolResult::success(output))
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ExaSearchInput {
    /// Search query
    query: String,

    /// Maximum number of results to return
    #[serde(default = "default_max_results")]
    max_results: usize,

    /// Search type: "auto", "neural", or "keyword"
    #[serde(default = "default_search_type")]
    search_type: String,
}

fn default_max_results() -> usize {
    5
}

fn default_search_type() -> String {
    "auto".to_string()
}

// Direct API response structures (used only in API mode)
#[derive(Debug, Deserialize)]
struct ExaResponse {
    results: Vec<ExaResult>,
}

#[derive(Debug, Deserialize)]
struct ExaResult {
    title: Option<String>,
    url: String,
    text: Option<String>,
}

#[async_trait]
impl Tool for ExaSearchTool {
    fn name(&self) -> &str {
        "exa_search"
    }

    fn description(&self) -> &str {
        "Search the internet using EXA AI for high-quality, neural-powered web search results."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 5)",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 10
                },
                "search_type": {
                    "type": "string",
                    "description": "Search type: 'auto', 'neural', or 'keyword' (default: 'auto')",
                    "enum": ["auto", "neural", "keyword"],
                    "default": "auto"
                }
            },
            "required": ["query"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input: ExaSearchInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;

        if input.query.trim().is_empty() {
            return Err(ToolError::InvalidInput("Query cannot be empty".to_string()));
        }

        if input.max_results == 0 || input.max_results > 10 {
            return Err(ToolError::InvalidInput(
                "max_results must be between 1 and 10".to_string(),
            ));
        }

        Ok(())
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let parsed: ExaSearchInput = serde_json::from_value(input)?;

        if self.use_mcp() {
            self.execute_via_mcp(&parsed.query, parsed.max_results)
                .await
        } else {
            self.execute_via_api(&parsed).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> ExaSearchTool {
        ExaSearchTool::new(None)
    }

    fn make_tool_with_key() -> ExaSearchTool {
        ExaSearchTool::new(Some("test-key".to_string()))
    }

    #[test]
    fn test_tool_name() {
        let tool = make_tool();
        assert_eq!(tool.name(), "exa_search");
    }

    #[test]
    fn test_tool_capabilities() {
        let tool = make_tool();
        let caps = tool.capabilities();
        assert_eq!(caps.len(), 1);
        assert!(matches!(caps[0], ToolCapability::Network));
    }

    #[test]
    fn test_tool_no_approval_required() {
        let tool = make_tool();
        assert!(!tool.requires_approval());
    }

    #[test]
    fn test_input_schema_has_query() {
        let tool = make_tool();
        let schema = tool.input_schema();
        let required = schema.get("required").and_then(|v| v.as_array());
        assert!(required.is_some());
        let required = required.unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
    }

    #[test]
    fn test_validate_valid_input() {
        let tool = make_tool();
        let input = serde_json::json!({ "query": "rust programming" });
        assert!(tool.validate_input(&input).is_ok());
    }

    #[test]
    fn test_validate_empty_query() {
        let tool = make_tool();
        let input = serde_json::json!({ "query": "  " });
        assert!(tool.validate_input(&input).is_err());
    }

    #[test]
    fn test_validate_missing_query() {
        let tool = make_tool();
        let input = serde_json::json!({ "max_results": 5 });
        assert!(tool.validate_input(&input).is_err());
    }

    #[test]
    fn test_validate_max_results_zero() {
        let tool = make_tool();
        let input = serde_json::json!({ "query": "test", "max_results": 0 });
        assert!(tool.validate_input(&input).is_err());
    }

    #[test]
    fn test_validate_max_results_too_high() {
        let tool = make_tool();
        let input = serde_json::json!({ "query": "test", "max_results": 11 });
        assert!(tool.validate_input(&input).is_err());
    }

    #[test]
    fn test_validate_with_search_type() {
        let tool = make_tool();
        let input = serde_json::json!({
            "query": "test",
            "max_results": 3,
            "search_type": "neural"
        });
        assert!(tool.validate_input(&input).is_ok());
    }

    #[test]
    fn test_default_deserialization() {
        let input: ExaSearchInput =
            serde_json::from_value(serde_json::json!({ "query": "hello" })).unwrap();
        assert_eq!(input.query, "hello");
        assert_eq!(input.max_results, 5);
        assert_eq!(input.search_type, "auto");
    }

    #[test]
    fn test_mcp_mode_default() {
        let tool = make_tool();
        assert!(tool.use_mcp());
        assert!(tool.api_key.is_none());
    }

    #[test]
    fn test_direct_api_mode_with_key() {
        let tool = make_tool_with_key();
        assert!(!tool.use_mcp());
        assert!(tool.api_key.is_some());
    }

    #[test]
    fn test_parse_sse_response() {
        let sse_body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"Search results here\"}],\"isError\":false}}\n\n";
        let json = ExaSearchTool::parse_sse_response(sse_body).unwrap();
        assert_eq!(json["id"], 2);
        assert_eq!(json["result"]["content"][0]["text"], "Search results here");
    }

    #[test]
    fn test_parse_sse_response_no_data() {
        let sse_body = "event: ping\n\n";
        assert!(ExaSearchTool::parse_sse_response(sse_body).is_err());
    }

    #[test]
    fn test_extract_mcp_result_success() {
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "content": [{ "type": "text", "text": "1. Result Title\n   URL: https://example.com\n" }],
                "isError": false
            }
        });
        let result = ExaSearchTool::extract_mcp_result(&json, "test query").unwrap();
        assert!(result.success);
    }

    #[test]
    fn test_extract_mcp_result_error() {
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "error": { "code": -32602, "message": "Unknown tool" }
        });
        let result = ExaSearchTool::extract_mcp_result(&json, "test").unwrap();
        assert!(!result.success);
    }

    #[test]
    fn test_extract_mcp_result_tool_error() {
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "content": [{ "type": "text", "text": "Rate limit exceeded" }],
                "isError": true
            }
        });
        let result = ExaSearchTool::extract_mcp_result(&json, "test").unwrap();
        assert!(!result.success);
    }
}
