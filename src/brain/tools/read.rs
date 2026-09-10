//! Read File Tool
//!
//! Allows reading file contents from the filesystem.

use super::error::{Result, ToolError, validate_file_path};
use super::hashline::hash::{format_hashline, hash_line};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Maximum file size to read without warning (10MB)
const LARGE_FILE_THRESHOLD: u64 = 10 * 1024 * 1024;

/// Maximum file size to read at all (100MB)
const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Maximum number of lines to read in a single request
const MAX_LINES: usize = 100_000;

/// Per-line character ceiling (#986). A minified JS/CSS bundle or sourcemap
/// is one line; without a clamp it lands in context whole. 2,000 matches the
/// battle-tested default from the Command Code read_file audit.
const MAX_LINE_CHARS: usize = 2_000;

/// Output byte budget for default (non-ranged) reads (#986). Once emitted
/// output exceeds this, reading stops with an announced truncation and an
/// exact resume offset. Explicit start_line/line_count requests bypass the
/// budget (user-driven window) but never the per-line clamp.
const OUTPUT_BUDGET: usize = 128 * 1024;

/// Binary media that must NOT be read as text — reading the raw bytes yields
/// garbage. Returns the tool the agent should call instead, or `None` for a
/// normal text file. Keyed on extension so it's cheap and needs no file read.
fn media_tool_redirect(path: &std::path::Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())?;
    let p = path.display();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "heic" | "heif" | "tiff" => {
            Some(format!(
                "'{p}' is an image — reading it as text would be garbage. Call \
                 analyze_image(image='{p}', question='...') to view it with a vision model."
            ))
        }
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "3gp" | "flv" => Some(format!(
            "'{p}' is a video. Call analyze_video(path='{p}', question='...') to view it."
        )),
        "pdf" | "docx" | "doc" | "pptx" | "xlsx" | "xlsb" | "xlsm" | "ods" | "epub" => {
            Some(format!(
                "'{p}' is a document. Call parse_document(path='{p}') to read its text\
             {}.",
                if ext == "pdf" {
                    ", or pdf_to_images then analyze_image for scanned/figure pages"
                } else {
                    ""
                }
            ))
        }
        _ => None,
    }
}

/// Read file tool
pub struct ReadTool;

#[derive(Debug, Deserialize, Serialize)]
struct ReadInput {
    /// Path to the file to read
    path: String,

    /// Optional: Start line (0-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    start_line: Option<usize>,

    /// Optional: Number of lines to read
    #[serde(skip_serializing_if = "Option::is_none")]
    line_count: Option<usize>,

    /// Optional: Output with hashline tags (HASH|content format, where HASH is a 4-char content hash)
    #[serde(default)]
    hashline: Option<bool>,
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read contents of a file from the filesystem. Can optionally read specific line ranges."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (absolute or relative to working directory)"
                },
                "start_line": {
                    "type": "integer",
                    "description": "Optional: Starting line number (0-indexed)",
                    "minimum": 0
                },
                "line_count": {
                    "type": "integer",
                    "description": "Optional: Number of lines to read from start_line",
                    "minimum": 1
                },
                "hashline": {
                    "type": "boolean",
                    "description": "Optional: Output lines in HASH|content format (4-char content hash) for use with hashline_edit tool. Default: false."
                }
            },
            "required": ["path"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Reading files is generally safe
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: ReadInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: ReadInput = serde_json::from_value(input)?;

        // Validate path: safety check, existence, and file type
        let path = match validate_file_path(&input.path, &context.working_dir()) {
            Ok(p) => p,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };

        // Confidential-path deny (OC-08): keys.toml, SSH private keys, .env,
        // .pem, /etc/shadow and friends are refused at the harness, not left to
        // a brain-file instruction the model may ignore. Reading them would put
        // raw secrets into the conversation, the DB, and any channel.
        if let Some(reason) = super::confidential::is_confidential(&path) {
            return Ok(ToolResult::error(format!(
                "read_file refused: {} is confidential ({reason}) and is not readable through \
                 tools (OC-08). Handle it outside the agent if you truly need to.",
                path.display()
            )));
        }

        // Bounce binary media (images/video/docs) to the right tool — reading
        // their bytes as text is meaningless, and the model otherwise loops on
        // read_file for a dropped screenshot instead of calling analyze_image.
        if let Some(redirect) = media_tool_redirect(&path) {
            return Ok(ToolResult::error(redirect));
        }

        // Check file size to prevent memory exhaustion
        let metadata = fs::metadata(&path).await.map_err(ToolError::Io)?;
        let file_size = metadata.len();

        if file_size > MAX_FILE_SIZE {
            return Ok(ToolResult::error(format!(
                "File too large: {} MB exceeds maximum {} MB. Use start_line and line_count to read portions.",
                file_size / (1024 * 1024),
                MAX_FILE_SIZE / (1024 * 1024)
            )));
        }

        let is_large_file = file_size > LARGE_FILE_THRESHOLD;

        let is_hashline = input.hashline.unwrap_or(false);

        // For large files or line-range requests, use buffered streaming
        let (output, total_lines, warning, clamped_lines) = if input.start_line.is_some()
            || input.line_count.is_some()
            || is_large_file
        {
            self.read_with_buffer(&path, input.start_line, input.line_count, is_large_file)
                .await?
        } else {
            // Small file: read entire contents directly
            let contents = fs::read_to_string(&path).await.map_err(ToolError::Io)?;
            let line_count = contents.lines().count();
            // Remember what this session saw, so a later whole-file write
            // can tell its own output from another agent's change (#954).
            // Only whole-file reads qualify: a partial read is not a basis
            // for replacing the file.
            super::file_versions::record(context.session_id, &path, &contents);
            // Whole-file reads unlock later overwrites (#1168); windowed
            // reads deliberately do not.
            super::read_state::mark_fully_read(context.session_id, &path);
            // A whole-file read of a skill definition counts as consuming
            // that skill (issue #131): the post-compaction stamp lists it
            // even though no slash command was ever issued.
            if let Some(slug) = super::seen_skills::skill_slug_from_path(&path) {
                super::seen_skills::mark_seen(context.session_id, &slug);
            }
            if contents.len() > OUTPUT_BUDGET {
                // Budget path (#986): emit lines until the 128 KB budget is
                // exhausted, then stop with an announced truncation and the
                // exact resume offset.
                let mut out = String::new();
                let mut emitted = 0usize;
                let mut clamped = 0usize;
                for line in contents.lines() {
                    let (cl, was_clamped) = clamp_line(line);
                    if was_clamped.is_some() {
                        clamped += 1;
                    }
                    let add_len = cl.len() + usize::from(!out.is_empty());
                    if out.len() + add_len > OUTPUT_BUDGET {
                        break;
                    }
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&cl);
                    emitted += 1;
                }
                let warning = format!(
                    "Output truncated at the {} KB output budget. File has {} total lines. Resume with start_line={} (0-indexed). Use start_line and line_count for pagination.",
                    OUTPUT_BUDGET / 1024,
                    line_count,
                    emitted
                );
                (out, line_count, Some(warning), clamped)
            } else {
                let mut clamped = 0usize;
                let mut out = String::new();
                for line in contents.lines() {
                    let (cl, was_clamped) = clamp_line(line);
                    if was_clamped.is_some() {
                        clamped += 1;
                    }
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&cl);
                }
                (out, line_count, None, clamped)
            }
        };

        // An empty file must announce itself. Silence is indistinguishable
        // from a failed read or a wrong path, and the model burns turns
        // re-reading and guessing path variants. Say it plainly (#987, from
        // the Command Code read_file audit: silence is the most expensive
        // thing a tool can return).
        let output = if output.is_empty() && total_lines == 0 {
            "(file exists and is empty, 0 bytes)".to_string()
        } else {
            output
        };

        // Apply hashline formatting if requested
        let output = if is_hashline {
            let file_start_line = input.start_line.unwrap_or(0) + 1; // convert 0-indexed to 1-indexed

            // First pass: compute all hashes and detect collisions
            let lines_with_hashes: Vec<(usize, String, &str)> = output
                .lines()
                .enumerate()
                .map(|(i, line)| {
                    let line_num = file_start_line + i;
                    let hash = hash_line(line);
                    (line_num, hash, line)
                })
                .collect();

            // Build reverse lookup to detect collisions
            let mut hash_to_lines: std::collections::HashMap<&str, Vec<usize>> =
                std::collections::HashMap::new();
            for (line_num, hash, _) in &lines_with_hashes {
                hash_to_lines
                    .entry(hash.as_str())
                    .or_default()
                    .push(*line_num);
            }

            // Identify collision hashes (appear on multiple lines) - own the strings
            let collision_hashes: std::collections::HashSet<String> = hash_to_lines
                .iter()
                .filter(|(_, lines)| lines.len() > 1)
                .map(|(hash, _)| hash.to_string())
                .collect();

            // Second pass: format output, marking collision lines
            let mut formatted_lines = Vec::new();
            for (_line_num, hash, line) in lines_with_hashes {
                if collision_hashes.contains(&hash) {
                    // Collision: don't show hash, add instruction
                    formatted_lines.push(format!("COLLISION|{}", line));
                } else {
                    formatted_lines.push(format_hashline(0, &hash, line));
                }
            }

            // Add collision warning at the end if any collisions detected
            if !collision_hashes.is_empty() {
                let collision_count = collision_hashes.len();
                formatted_lines.push(String::new());
                formatted_lines.push(format!(
                    "[WARNING: {} line(s) have hash collisions and cannot be edited with hashline_edit. Use the conventional edit_file tool with search/replace instead.]",
                    collision_count
                ));
            }

            formatted_lines.join("\n")
        } else {
            output
        };

        let output_len = output.len();
        let mut result = ToolResult::success(output)
            .with_metadata("path".to_string(), path.display().to_string())
            .with_metadata("bytes".to_string(), output_len.to_string())
            .with_metadata("total_lines".to_string(), total_lines.to_string());

        // Announce clamped lines, then attach the combined warning (#986).
        let warning = if clamped_lines > 0 {
            let clamp_note = if is_hashline {
                format!(
                    "{} line(s) exceeded {} chars and were truncated; their hashes cover only the visible prefix, so do not hashline_edit those lines.",
                    clamped_lines, MAX_LINE_CHARS
                )
            } else {
                format!(
                    "{} line(s) exceeded {} chars and were truncated.",
                    clamped_lines, MAX_LINE_CHARS
                )
            };
            match warning {
                Some(w) => Some(format!("{} {}", w, clamp_note)),
                None => Some(clamp_note),
            }
        } else {
            warning
        };

        // Add warning for large files
        if let Some(warn_msg) = warning {
            result = result.with_metadata("warning".to_string(), warn_msg);
        }

        Ok(result)
    }
}

impl ReadTool {
    /// Read file using buffered I/O for memory efficiency
    async fn read_with_buffer(
        &self,
        path: &std::path::Path,
        start_line: Option<usize>,
        line_count: Option<usize>,
        is_large_file: bool,
    ) -> Result<(String, usize, Option<String>, usize)> {
        let file = fs::File::open(path).await.map_err(ToolError::Io)?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let start = start_line.unwrap_or(0);
        let max_lines = line_count.unwrap_or(MAX_LINES).min(MAX_LINES);

        let mut output = String::new();
        let mut current_line = 0;
        let mut lines_read = 0;
        let mut total_lines = 0;
        let mut truncated = false;
        let mut clamped_lines = 0usize;
        let budgeted = start_line.is_none() && line_count.is_none();
        let mut budget_exceeded = false;

        // Skip lines before start
        while current_line < start {
            match lines.next_line().await.map_err(ToolError::Io)? {
                Some(_) => {
                    current_line += 1;
                    total_lines += 1;
                }
                None => {
                    return Err(ToolError::InvalidInput(format!(
                        "Start line {} exceeds file length {}",
                        start, current_line
                    )));
                }
            }
        }

        // Read requested lines
        while lines_read < max_lines {
            match lines.next_line().await.map_err(ToolError::Io)? {
                Some(line) => {
                    let (clamped_line, was_clamped) = clamp_line(&line);
                    if was_clamped.is_some() {
                        clamped_lines += 1;
                    }
                    let add_len = clamped_line.len() + usize::from(!output.is_empty());
                    if budgeted && output.len() + add_len > OUTPUT_BUDGET {
                        // The line was already consumed from the reader above;
                        // count it so the reported total stays exact, and stop
                        // before emitting it so the resume offset stays exact
                        // (#986/#988).
                        budget_exceeded = true;
                        total_lines += 1;
                        break;
                    }
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&clamped_line);
                    lines_read += 1;
                    total_lines += 1;
                }
                None => break,
            }
        }

        // Count remaining lines if we haven't read the whole file
        if budget_exceeded || (line_count.is_none() && lines_read >= MAX_LINES) {
            truncated = true;
            // Count remaining lines without loading them into memory
            while lines.next_line().await.map_err(ToolError::Io)?.is_some() {
                total_lines += 1;
            }
        } else {
            // Count any remaining lines
            while lines.next_line().await.map_err(ToolError::Io)?.is_some() {
                total_lines += 1;
            }
        }

        let warning = if truncated {
            let reason = if budget_exceeded {
                format!(
                    "Output truncated at the {} KB output budget.",
                    OUTPUT_BUDGET / 1024
                )
            } else {
                format!("Output truncated at {} lines.", MAX_LINES)
            };
            Some(format!(
                "{} File has {} total lines. Resume with start_line={} (0-indexed). Use start_line and line_count for pagination.",
                reason,
                total_lines,
                start + lines_read
            ))
        } else if is_large_file && line_count.is_none() {
            Some(format!(
                "Large file ({} lines). Consider using start_line and line_count for better performance.",
                total_lines
            ))
        } else {
            None
        };

        Ok((output, total_lines, warning, clamped_lines))
    }
}

/// Clamp one line to [`MAX_LINE_CHARS`] characters (#986). Returns the
/// (possibly truncated) line plus, when clamped, the original character count
/// so callers can announce what the model is not seeing.
fn clamp_line(line: &str) -> (std::borrow::Cow<'_, str>, Option<usize>) {
    if line.len() <= MAX_LINE_CHARS {
        // bytes <= cap => chars <= cap, no need to walk the string
        return (std::borrow::Cow::Borrowed(line), None);
    }
    let total_chars = line.chars().count();
    if total_chars <= MAX_LINE_CHARS {
        return (std::borrow::Cow::Borrowed(line), None);
    }
    let byte_cut = line
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    (
        std::borrow::Cow::Owned(format!(
            "{}... [line truncated: {} chars total, showing first {}]",
            &line[..byte_cut],
            total_chars,
            MAX_LINE_CHARS
        )),
        Some(total_chars),
    )
}
