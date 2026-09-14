//! Write File Tool
//!
//! Allows writing content to files on the filesystem.

use super::error::{Result, ToolError, validate_path_safety};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;

/// Write file tool
pub struct WriteTool;

#[derive(Debug, Deserialize, Serialize)]
struct WriteInput {
    /// Path to the file to write
    path: String,

    /// Content to write to the file
    content: String,

    /// Whether to create parent directories if they don't exist
    #[serde(default)]
    create_dirs: bool,

    /// Confirm overwriting a file this session has not fully read (#1168)
    #[serde(default)]
    overwrite_read_confirm: bool,
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file on the filesystem. Creates the file if it doesn't exist, overwrites if it does. Keep one call under ~300 lines / ~12 KB: the content is generated as a single tool-call argument, and a larger one streams for minutes and gets cut. Split bigger files by concern (see the LARGE FILES rule) and add parts with edit_file."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write (absolute or relative to working directory)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "create_dirs": {
                    "type": "boolean",
                    "description": "Whether to create parent directories if they don't exist (default: false)",
                    "default": false
                },
                "overwrite_read_confirm": {
                    "type": "boolean",
                    "description": "Required to overwrite an existing file that this session has not fully read (a windowed start_line/line_count read does not count). Confirms you intend a blind replace.",
                    "default": false
                }
            },
            "required": ["path", "content"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![
            ToolCapability::WriteFiles,
            ToolCapability::SystemModification,
        ]
    }

    fn requires_approval(&self) -> bool {
        true // Writing files requires approval
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: WriteInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: WriteInput = serde_json::from_value(input)?;

        // Resolve path (tilde expansion + absolute/relative resolution).
        let path = super::error::resolve_tool_path(&input.path, &context.working_dir());

        // Brain-file guardrail (issue #91): protected brain files
        // (SOUL.md, MEMORY.md, USER.md, etc.) must go through
        // `write_opencrabs_file`, which enforces append-only,
        // dedup-aware shrink, and `.bak` snapshots. Generic write_file
        // does none of those, so reject the call before any bytes hit
        // disk and tell the agent to switch tools.
        if super::brain_file_safety::is_protected_path(&path) {
            return Ok(ToolResult::error(format!(
                "Refusing to write protected brain file '{}' with generic write_file. \
                 Use the `write_opencrabs_file` tool instead. It enforces append-only \
                 writes, dedup-aware shrinking, and saves a `.bak` snapshot before every \
                 change.",
                path.display()
            )));
        }

        // Create parent directories if requested (before path validation)
        if input.create_dirs
            && let Some(parent) = path.parent()
        {
            // Validate parent path is within working directory
            let canonical_wd = context.working_dir().canonicalize().map_err(|e| {
                ToolError::Internal(format!("Failed to canonicalize working directory: {}", e))
            })?;

            // If parent exists, check it's within bounds
            if parent.exists() {
                let canonical_parent = parent.canonicalize().map_err(|e| {
                    ToolError::InvalidInput(format!("Failed to resolve parent path: {}", e))
                })?;

                if !canonical_parent.starts_with(&canonical_wd) {
                    return Ok(ToolResult::error(format!(
                        "Access denied: Path '{}' is outside the working directory",
                        input.path
                    )));
                }
            }

            fs::create_dir_all(parent).await.map_err(ToolError::Io)?;
        }

        // Resolve path (relative paths resolve against working directory)
        let path = match validate_path_safety(&input.path, &context.working_dir()) {
            Ok(p) => p,
            Err(ToolError::InvalidInput(msg))
                if msg.contains("Parent directory does not exist") =>
            {
                // For write operations, give a helpful error about create_dirs
                let resolved = std::path::PathBuf::from(&input.path);
                if let Some(parent) = resolved.parent() {
                    return Ok(ToolResult::error(format!(
                        "Parent directory does not exist: {}. Use create_dirs: true to create it.",
                        parent.display()
                    )));
                }
                return Ok(ToolResult::error(msg));
            }
            Err(ToolError::InvalidInput(msg)) => {
                return Ok(ToolResult::error(format!("Invalid path: {}", msg)));
            }
            Err(e) => return Err(e),
        };

        // Check if parent directory exists (safety check after validation)
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            return Ok(ToolResult::error(format!(
                "Parent directory does not exist: {}. Use create_dirs: true to create it.",
                parent.display()
            )));
        }

        // Partial-view overwrite guard (#1168): a windowed read is not a
        // basis for replacing a file wholesale — lines outside the window
        // would be destroyed unseen. Require either a prior full read in
        // this session or an explicit confirm.
        if path.exists()
            && !input.overwrite_read_confirm
            && !super::read_state::was_fully_read(context.session_id, &path)
        {
            let size = fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
            return Ok(ToolResult::error(format!(
                "Refusing to overwrite '{}': file exists ({} bytes) but was not fully read \
                 in this session — a windowed read doesn't count. Read the whole file first, \
                 or pass \"overwrite_read_confirm\": true to confirm.",
                path.display(),
                size
            )));
        }

        // Never let a raw write corrupt the OpenCrabs config.toml/keys.toml (#713):
        // a broken write there takes down all API keys and the bot token. Deny it,
        // tell the agent why, and leave the file untouched.
        if let Err(msg) = crate::config::guard::deny_if_would_break(&path, &input.content) {
            return Ok(ToolResult::error(msg));
        }

        // Several agents share this working directory by design, and this tool
        // replaces the file wholesale with content composed from a read in an
        // EARLIER call. If the file moved since then, writing it destroys the
        // other agent's change with nothing reported. Refuse and let the agent
        // re-read — it can do that; it cannot detect a silent clobber (#954).
        let on_disk = fs::read_to_string(&path).await.ok();
        if super::file_versions::is_stale_write(context.session_id, &path, on_disk.as_deref()) {
            tracing::warn!(
                "write_file refused for {}: changed since this session read it — \
                 concurrent agents in one working directory",
                path.display()
            );
            return Ok(ToolResult::error(super::file_versions::refusal_message(
                &path,
            )));
        }

        // One writer at a time on this path (#1153). Advisory, held only
        // across the write; a contended write proceeds and says so rather
        // than refusing, since a blocked save is worse than an interleave.
        let write_lock = super::path_lock::acquire(&path);
        let contended = write_lock.as_ref().is_some_and(|l| !l.is_held());

        // Write the file
        super::fs_util::atomic_write_file(&path, input.content.as_bytes()).await?;
        drop(write_lock);

        // The session now knows the file as what it just wrote, so its next
        // write is not mistaken for a stale one.
        super::file_versions::record(context.session_id, &path, &input.content);

        // A file written by this session is known to it in full (#159), so a
        // subsequent write in the same session does not trigger the partial-view
        // overwrite guard (#1168) as if the file were pre-existing and unread.
        super::read_state::mark_fully_read(context.session_id, &path);

        // Track file in session (fire and forget, path-only)
        if let Some(ref sc) = context.service_context {
            let fs = crate::services::FileService::new(sc.clone());
            let _ = fs
                .get_or_create_file(context.session_id, path.clone(), None)
                .await;
        }

        let mut message = format!(
            "Successfully wrote {} bytes to {}",
            input.content.len(),
            path.display()
        );
        // An overlapping write is reported rather than swallowed.
        if contended {
            message.push_str(&super::path_lock::contention_notice(&path));
        }

        Ok(ToolResult::success(message)
            .with_metadata("path".to_string(), path.display().to_string())
            .with_metadata("bytes".to_string(), input.content.len().to_string()))
    }
}
