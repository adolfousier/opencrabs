//! GitHub Issue Reporter — RSI-only tool
//!
//! Opens issues on a configured GitHub repo when the RSI agent detects
//! problems that cannot be fixed at runtime via brain files.
//! Gated by a `<!-- rsi:github-issues repo:OWNER/REPO -->` marker in
//! ~/.opencrabs/AGENTS.md — without it, this tool is never registered.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Parse the repo from AGENTS.md marker: `<!-- rsi:github-issues repo:OWNER/REPO -->`
pub fn parse_repo_from_agents_md() -> Option<String> {
    let agents_path = crate::config::opencrabs_home().join("AGENTS.md");
    let content = std::fs::read_to_string(agents_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("rsi:github-issues")
            && let Some(repo_start) = trimmed.find("repo:")
        {
            let after = &trimmed[repo_start + 5..];
            let repo = after
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches("-->");
            if repo.contains('/') && !repo.is_empty() {
                return Some(repo.to_string());
            }
        }
    }
    None
}

pub struct GithubIssueTool {
    repo: String,
}

impl GithubIssueTool {
    pub fn new(repo: String) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl Tool for GithubIssueTool {
    fn name(&self) -> &str {
        "github_report_issue"
    }

    fn description(&self) -> &str {
        "Report a GitHub issue for problems that CANNOT be fixed at runtime via brain files. \
         Use ONLY for code-level bugs, architectural limitations, or missing features \
         that require a code change. The RSI agent must provide evidence from feedback data. \
         This tool can ONLY list and open issues — it cannot close, edit, or comment."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "What to do:\n\
                        - 'list': List open issues with the rsi-detected label to check for duplicates.\n\
                        - 'create': Open a new issue with structured evidence.",
                    "enum": ["list", "create"]
                },
                "title": {
                    "type": "string",
                    "description": "For 'create': short issue title (under 80 chars). Prefix with [RSI] automatically."
                },
                "body": {
                    "type": "string",
                    "description": "For 'create': issue body with structured evidence — what was detected, \
                         frequency, affected component, why it can't be fixed via brain files, \
                         and suggested fix if any."
                },
                "labels": {
                    "type": "string",
                    "description": "For 'create': comma-separated labels (e.g. 'bug,provider'). \
                         'rsi-detected' is always added automatically."
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    fn requires_approval_for_input(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");

        match action {
            "list" => {
                let output = tokio::process::Command::new("gh")
                    .args([
                        "issue",
                        "list",
                        "--repo",
                        &self.repo,
                        "--label",
                        "rsi-detected",
                        "--state",
                        "open",
                        "--limit",
                        "20",
                        "--json",
                        "number,title,url,createdAt",
                    ])
                    .output()
                    .await;

                match output {
                    Ok(out) if out.status.success() => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        if stdout.trim() == "[]" {
                            Ok(ToolResult::success(
                                "No open rsi-detected issues found.".to_string(),
                            ))
                        } else {
                            Ok(ToolResult::success(stdout.to_string()))
                        }
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        Ok(ToolResult::error(format!("gh issue list failed: {stderr}")))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to run gh command: {e}"))),
                }
            }

            "create" => {
                let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let body = input.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let extra_labels = input.get("labels").and_then(|v| v.as_str()).unwrap_or("");

                if title.is_empty() || body.is_empty() {
                    return Ok(ToolResult::error(
                        "title and body are required for 'create'".to_string(),
                    ));
                }

                let prefixed_title = if title.starts_with("[RSI]") {
                    title.to_string()
                } else {
                    format!("[RSI] {title}")
                };

                // Dedup: check if an open issue with same title already exists
                let search_output = tokio::process::Command::new("gh")
                    .args([
                        "issue",
                        "list",
                        "--repo",
                        &self.repo,
                        "--label",
                        "rsi-detected",
                        "--state",
                        "open",
                        "--search",
                        title,
                        "--limit",
                        "5",
                        "--json",
                        "title,number,url",
                    ])
                    .output()
                    .await;

                if let Ok(out) = &search_output
                    && out.status.success()
                {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    if let Ok(issues) = serde_json::from_str::<Vec<Value>>(&stdout) {
                        let title_core = prefixed_title.trim_start_matches("[RSI] ").to_lowercase();
                        for issue in &issues {
                            if let Some(existing_title) =
                                issue.get("title").and_then(|t| t.as_str())
                            {
                                let existing_core =
                                    existing_title.trim_start_matches("[RSI] ").to_lowercase();
                                if existing_core == title_core {
                                    let url = issue
                                        .get("url")
                                        .and_then(|u| u.as_str())
                                        .unwrap_or("(unknown)");
                                    return Ok(ToolResult::success(format!(
                                        "Duplicate — open issue already exists: {url}"
                                    )));
                                }
                            }
                        }
                    }
                }

                // Build labels
                let mut labels = vec!["rsi-detected".to_string()];
                for l in extra_labels.split(',') {
                    let l = l.trim();
                    if !l.is_empty() && l != "rsi-detected" {
                        labels.push(l.to_string());
                    }
                }
                let labels_str = labels.join(",");

                // Create the issue
                let create_output = tokio::process::Command::new("gh")
                    .args([
                        "issue",
                        "create",
                        "--repo",
                        &self.repo,
                        "--title",
                        &prefixed_title,
                        "--body",
                        body,
                        "--label",
                        &labels_str,
                    ])
                    .output()
                    .await;

                match create_output {
                    Ok(out) if out.status.success() => {
                        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        tracing::info!("RSI created GitHub issue: {url}");
                        Ok(ToolResult::success(format!("Issue created: {url}")))
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        Ok(ToolResult::error(format!(
                            "gh issue create failed: {stderr}"
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to run gh command: {e}"))),
                }
            }

            other => Ok(ToolResult::error(format!(
                "Unknown action: '{other}'. Use 'list' or 'create'."
            ))),
        }
    }
}
