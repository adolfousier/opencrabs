//! Brave Search Tool
//!
//! Perform real-time internet searches using the Brave Search API.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Brave search tool (requires BRAVE_API_KEY)
pub struct BraveSearchTool {
    api_key: String,
}

impl BraveSearchTool {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct BraveSearchInput {
    /// Search query
    query: String,

    /// Maximum number of results to return
    #[serde(default = "default_max_results")]
    max_results: usize,
}

fn default_max_results() -> usize {
    5
}

#[derive(Debug, Deserialize)]
struct BraveResponse {
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Vec<BraveResult>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    description: Option<String>,
}

#[async_trait]
impl Tool for BraveSearchTool {
    fn name(&self) -> &str {
        "brave_search"
    }

    fn description(&self) -> &str {
        "Search the internet using Brave Search for real-time web results with privacy focus."
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
        let input: BraveSearchInput = serde_json::from_value(input.clone())
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
        let input: BraveSearchInput = serde_json::from_value(input)?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| ToolError::Execution(format!("Failed to create HTTP client: {}", e)))?;

        let url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
            urlencoding::encode(&input.query),
            input.max_results
        );

        let response = client
            .get(&url)
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("Brave search request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Ok(ToolResult::error(format!(
                "Brave search failed with status {}: {}",
                status, body
            )));
        }

        let brave_response: BraveResponse = response
            .json()
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to parse Brave response: {}", e)))?;

        let mut output = format!("Search results for: \"{}\"\n\n", input.query);

        let results = brave_response
            .web
            .map(|w| w.results)
            .unwrap_or_default();

        if results.is_empty() {
            output.push_str("No results found. Try rephrasing your query.\n");
        } else {
            for (i, result) in results.iter().enumerate() {
                output.push_str(&format!("{}. {}\n", i + 1, result.title));
                output.push_str(&format!("   URL: {}\n", result.url));
                if let Some(desc) = &result.description {
                    output.push_str(&format!("   {}\n", desc));
                }
                output.push('\n');
            }
        }

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> BraveSearchTool {
        BraveSearchTool::new("test-key".to_string())
    }

    #[test]
    fn test_tool_name() {
        let tool = make_tool();
        assert_eq!(tool.name(), "brave_search");
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
        let input = serde_json::json!({ "query": "" });
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
    fn test_default_deserialization() {
        let input: BraveSearchInput =
            serde_json::from_value(serde_json::json!({ "query": "hello" })).unwrap();
        assert_eq!(input.query, "hello");
        assert_eq!(input.max_results, 5);
    }

    #[test]
    fn test_brave_response_parsing() {
        let json = serde_json::json!({
            "web": {
                "results": [
                    {
                        "title": "Test Result",
                        "url": "https://example.com",
                        "description": "A test result"
                    }
                ]
            }
        });
        let response: BraveResponse = serde_json::from_value(json).unwrap();
        let results = response.web.unwrap().results;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Result");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].description, Some("A test result".to_string()));
    }

    #[test]
    fn test_brave_response_no_web() {
        let json = serde_json::json!({});
        let response: BraveResponse = serde_json::from_value(json).unwrap();
        assert!(response.web.is_none());
    }
}
