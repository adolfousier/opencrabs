//! Memory Search Tool
//!
//! Searches past conversation compaction logs using the `qmd` crate's FTS5 engine.
//! Always available — no external dependencies required.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Memory search tool backed by the `qmd` crate's FTS5 engine.
pub struct MemorySearchTool;

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search past conversation memory logs for relevant context. \
         Use this when you need to recall decisions, files, errors, or context \
         from previous sessions. Returns matching excerpts from daily memory logs."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query for past memories"
                },
                "n": {
                    "type": "integer",
                    "description": "Number of results to return (default: 5)",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if query.is_empty() {
            return Ok(ToolResult::error("query parameter is required".to_string()));
        }

        let n = input.get("n").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        // Get memory qmd store
        let store = match crate::memory::get_store() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Memory store init failed: {}", e);
                return Ok(ToolResult::error(format!(
                    "Memory search unavailable: {e}. \
                     Daily memory logs are still saved to ~/.opencrabs/memory/ as markdown files \
                     that you can read directly with the read_file tool."
                )));
            }
        };

        match crate::memory::search(store, &query, n).await {
            Ok(results) if results.is_empty() => Ok(ToolResult::success(
                "No matching memories found.".to_string(),
            )),
            Ok(results) => {
                let mut output = String::new();
                for (i, r) in results.iter().enumerate() {
                    output.push_str(&format!("{}. **{}**\n   {}\n\n", i + 1, r.path, r.snippet));
                }
                Ok(ToolResult::success(output))
            }
            Err(e) => Ok(ToolResult::error(format!("Memory search failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_metadata() {
        let tool = MemorySearchTool;
        assert_eq!(tool.name(), "memory_search");
        assert!(!tool.requires_approval());
    }

    #[tokio::test]
    async fn test_empty_query() {
        let tool = MemorySearchTool;
        let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
        let result = tool
            .execute(serde_json::json!({"query": ""}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
    }
}
