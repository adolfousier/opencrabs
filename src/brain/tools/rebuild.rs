//! Rebuild Tool
//!
//! Lets the agent build OpenCrabs from source and exec() restart automatically.
//! The build runs via `SelfUpdater::build_streaming` — progress lines are forwarded
//! through the ProgressCallback so the TUI shows them live.  On success, a
//! `ProgressEvent::RestartReady` is emitted which triggers an automatic exec() restart
//! (no user prompt needed).

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::brain::SelfUpdater;
use crate::brain::agent::{ProgressCallback, ProgressEvent};
use async_trait::async_trait;
use serde_json::Value;

/// Agent-callable tool that builds the project and auto-restarts via exec().
pub struct RebuildTool {
    progress: Option<ProgressCallback>,
}

impl RebuildTool {
    pub fn new(progress: Option<ProgressCallback>) -> Self {
        Self { progress }
    }
}

#[async_trait]
impl Tool for RebuildTool {
    fn name(&self) -> &str {
        "rebuild"
    }

    fn description(&self) -> &str {
        "Build OpenCrabs from source (cargo build --release) and signal the TUI to hot-restart. \
         Call this after editing source code to apply your changes. On success the binary is \
         exec()-replaced automatically (no prompt). On failure the compiler output is returned."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    async fn execute(&self, _input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let updater = match SelfUpdater::auto_detect() {
            Ok(u) => u,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Cannot detect project root: {}",
                    e
                )));
            }
        };

        let cb = self.progress.clone();

        // Stream build progress lines through the progress callback
        let result = updater
            .build_streaming(move |line| {
                let trimmed = line.trim();
                // Forward meaningful cargo lines as intermediate text
                if (trimmed.starts_with("Compiling")
                    || trimmed.starts_with("Finished")
                    || trimmed.starts_with("error")
                    || trimmed.starts_with("warning[")
                    || trimmed.starts_with("-->"))
                    && let Some(ref cb) = cb
                {
                    cb(ProgressEvent::IntermediateText {
                        text: line,
                        reasoning: None,
                    });
                }
            })
            .await;

        match result {
            Ok(path) => {
                // Signal auto-restart — TuiEvent::RestartReady triggers exec() with no prompt
                if let Some(ref cb) = self.progress {
                    cb(ProgressEvent::RestartReady {
                        status: format!("Build successful: {}", path.display()),
                    });
                }
                Ok(ToolResult::success(format!(
                    "Build successful: {}. Restarting now.",
                    path.display()
                )))
            }
            Err(output) => Ok(ToolResult::error(format!("Build failed:\n{}", output))),
        }
    }
}
