//! Claude CLI Provider — direct subprocess integration
//!
//! Spawns the `claude` CLI binary as a text completion backend and reads
//! its NDJSON stream output, converting it to standard `StreamEvent`s.
//! OpenCrabs handles all tools, memory, and context locally.

use super::error::{ProviderError, Result};
use super::r#trait::{Provider, ProviderStream};
use super::types::*;
use async_trait::async_trait;
use futures::stream::StreamExt;
use serde::Deserialize;
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;

/// Claude CLI provider — talks directly to the `claude` binary.
#[derive(Clone)]
pub struct ClaudeCliProvider {
    claude_path: String,
    default_model: String,
}

impl ClaudeCliProvider {
    /// Create a new provider, auto-detecting the claude binary.
    pub fn new() -> Result<Self> {
        let path = resolve_claude_path()?;
        Ok(Self {
            claude_path: path,
            default_model: "sonnet-4-6".to_string(),
        })
    }

    /// Override the default model (e.g. "opus", "haiku", "sonnet").
    /// Normalizes to display form (e.g. "opus" → "opus-4-6").
    pub fn with_default_model(mut self, model: String) -> Self {
        self.default_model = Self::normalize_model(&model);
        self
    }

    /// Map full Anthropic model name to CLI shorthand.
    fn map_model(model: &str) -> &str {
        if model.contains("opus") {
            "opus"
        } else if model.contains("haiku") {
            "haiku"
        } else {
            "sonnet"
        }
    }

    /// Normalize a model name for pricing/usage tracking.
    /// CLI shorthands like "opus" → "opus-4-6". Strips "claude-" prefix.
    fn normalize_model(model: &str) -> String {
        // Strip "claude-" prefix first (API returns "claude-opus-4-6")
        let stripped = model.strip_prefix("claude-").unwrap_or(model);
        match stripped {
            "opus" => "opus-4-6".to_string(),
            "sonnet" => "sonnet-4-6".to_string(),
            "haiku" => "haiku-4-5".to_string(),
            other => other.to_string(),
        }
    }

    /// Build a plain-text prompt from LLMRequest messages.
    fn build_prompt(request: &LLMRequest) -> String {
        let mut parts = Vec::new();

        if let Some(ref system) = request.system
            && !system.is_empty()
        {
            parts.push(system.clone());
        }

        for msg in &request.messages {
            let role = match msg.role {
                Role::User => "Human",
                Role::Assistant => "Assistant",
                Role::System => "System",
            };
            let content: String = msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => Some(format!("[tool_result for {}]: {}", tool_use_id, content)),
                    ContentBlock::ToolUse { id, name, input } => {
                        Some(format!("[tool_use {} ({}): {}]", name, id, input))
                    }
                    ContentBlock::Thinking { thinking, .. } => {
                        if thinking.is_empty() {
                            None
                        } else {
                            Some(format!("<thinking>{}</thinking>", thinking))
                        }
                    }
                    ContentBlock::Image { source } => {
                        // CLI -p mode cannot process images inline.
                        // Keep the file path reference so the agent can use
                        // the analyze_image tool to describe it.
                        Some(match source {
                            ImageSource::Base64 { media_type, data } => {
                                // Save to temp file so analyze_image can read it
                                let ext = match media_type.as_str() {
                                    "image/png" => "png",
                                    "image/jpeg" => "jpeg",
                                    "image/gif" => "gif",
                                    "image/webp" => "webp",
                                    _ => "png",
                                };
                                let tmp = std::env::temp_dir().join(format!(
                                    "opencrabs_cli_img_{}.{}",
                                    uuid::Uuid::new_v4(),
                                    ext
                                ));
                                use base64::Engine;
                                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data)
                                    && std::fs::write(&tmp, &bytes).is_ok()
                                {
                                    format!(
                                        "[User attached an image at {}. Use the analyze_image tool to view it.]",
                                        tmp.display()
                                    )
                                } else {
                                    "[User attached an image but it could not be decoded.]".to_string()
                                }
                            }
                            ImageSource::Url { url } => {
                                format!(
                                    "[User attached an image: {}. Use the analyze_image tool to view it.]",
                                    url
                                )
                            }
                        })
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            if content.trim().is_empty() {
                continue;
            }
            parts.push(format!("{}: {}", role, content));
        }

        parts.join("\n\n")
    }
}

/// Resolve the claude CLI binary path.
fn resolve_claude_path() -> Result<String> {
    if let Ok(path) = std::env::var("CLAUDE_PATH") {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
        return Err(ProviderError::Internal(format!(
            "CLAUDE_PATH set but not found: {}",
            path
        )));
    }

    for candidate in &["/opt/homebrew/bin/claude", "/usr/local/bin/claude"] {
        if std::path::Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }

    // Try PATH via `which`
    if let Ok(output) = std::process::Command::new("which").arg("claude").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }

    Err(ProviderError::Internal(
        "claude CLI not found — install it or set CLAUDE_PATH".to_string(),
    ))
}

// ── CLI NDJSON types ──

/// A parsed NDJSON message from claude CLI stdout.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CliMessage {
    System {
        #[allow(dead_code)]
        model: Option<String>,
    },
    Assistant {
        message: CliAssistantMessage,
    },
    /// Real-time SSE event — contains a complete Anthropic SSE payload.
    StreamEvent {
        event: serde_json::Value,
    },
    /// User turn during multi-turn tool loops (tool_result).
    User {
        #[allow(dead_code)]
        #[serde(default)]
        message: serde_json::Value,
    },
    RateLimitEvent {},
    Result {
        stop_reason: Option<String>,
        usage: Option<CliUsage>,
        #[serde(default)]
        is_error: bool,
        result: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct CliAssistantMessage {
    pub id: Option<String>,
    pub model: Option<String>,
    pub usage: Option<CliUsage>,
    #[serde(default)]
    pub content: Vec<CliContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CliContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    Thinking {
        thinking: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
struct CliUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    /// Cache tokens are the CLI's system prompt — tracked for billing but not context.
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

impl CliUsage {
    /// Non-cached input tokens only.
    /// The CLI's cache_creation and cache_read tokens are its own system
    /// prompt being cached by Anthropic — not conversation content we control.
    /// Including them inflates our context tracking and misleads the TUI display.
    fn total_input(&self) -> u32 {
        self.input_tokens
    }

    /// Full token count including cache (for cost/billing and debug logging).
    fn total_with_cache(&self) -> u32 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }
}

#[async_trait]
impl Provider for ClaudeCliProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        // Collect streaming response into a single response
        let mut stream = self.stream(request).await?;

        let mut id = String::new();
        let mut model = String::new();
        let mut content = Vec::new();
        let mut stop_reason = None;
        let mut usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
        };

        // Simple accumulator — text blocks only
        let mut text_buf = String::new();

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::MessageStart { message } => {
                    id = message.id;
                    model = message.model;
                    usage.input_tokens = message.usage.input_tokens;
                }
                StreamEvent::ContentBlockDelta {
                    delta: ContentDelta::TextDelta { text },
                    ..
                } => {
                    text_buf.push_str(&text);
                }
                StreamEvent::MessageDelta { delta: d, usage: u } => {
                    stop_reason = d.stop_reason;
                    usage.output_tokens = u.output_tokens;
                }
                StreamEvent::MessageStop => break,
                _ => {}
            }
        }

        if !text_buf.is_empty() {
            content.push(ContentBlock::Text { text: text_buf });
        }

        Ok(LLMResponse {
            id,
            model,
            content,
            stop_reason,
            usage,
        })
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        let prompt = Self::build_prompt(&request);
        let original_model = Self::normalize_model(&request.model);
        let model = Self::map_model(&request.model).to_string();

        let cwd = request
            .working_directory
            .as_deref()
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")));

        tracing::info!(
            "Spawning claude CLI: model={}, prompt_len={}, cwd={}",
            model,
            prompt.len(),
            cwd.display()
        );

        // Each CLI spawn gets a fresh session ID. We manage conversation context
        // ourselves (context.rs), so we don't need the CLI to maintain session state.
        // Reusing OpenCrabs session IDs caused "Session ID already in use" errors
        // when concurrent requests (TUI + Telegram/Slack) shared the same session.
        let session_id_str = uuid::Uuid::new_v4().to_string();

        let mut child = tokio::process::Command::new(&self.claude_path)
            .env_remove("CLAUDECODE")
            .env_remove("CLAUDE_CODE_ENTRYPOINT")
            .arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--session-id")
            .arg(&session_id_str)
            .arg("--dangerously-skip-permissions")
            .arg("--model")
            .arg(&model)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ProviderError::Internal(format!("failed to spawn claude CLI: {}", e)))?;

        // Write prompt via stdin to avoid leaking in `ps aux`
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProviderError::Internal("failed to capture stdin".to_string()))?;
        let prompt_bytes = prompt.into_bytes();
        tokio::spawn(async move {
            let _ = stdin.write_all(&prompt_bytes).await;
            let _ = stdin.shutdown().await;
        });

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::Internal("failed to capture stdout".to_string()))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ProviderError::Internal("failed to capture stderr".to_string()))?;

        // Spawn stderr reader — log everything the CLI writes to stderr
        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if !line.is_empty() {
                    tracing::warn!("claude CLI stderr: {}", line);
                }
            }
        });

        // Channel-based stream: parse NDJSON lines → StreamEvent
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamEvent>>(64);

        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();

            // Translation state — mirrors the proxy's TranslateState
            let mut started = false;
            let mut streaming_via_events = false;
            let mut completed_blocks: usize = 0;
            let mut current_block_started = false;
            let mut current_block_chars: usize = 0;
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut line_count = 0u32;
            // Track block index offset across tool rounds — each CLI turn
            // restarts content block indices at 0, so we offset them to prevent
            // collision in stream_complete()'s block_states array.
            let mut block_index_offset: usize = 0;
            let mut max_block_index_this_round: usize = 0;

            loop {
                let line_result = lines.next_line().await;
                let line = match line_result {
                    Ok(Some(line)) => line,
                    Ok(None) => {
                        tracing::info!(
                            "CLI stdout EOF after {} lines (started={})",
                            line_count,
                            started
                        );
                        break;
                    }
                    Err(e) => {
                        tracing::error!("CLI stdout read error after {} lines: {}", line_count, e);
                        break;
                    }
                };
                line_count += 1;
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                tracing::debug!("CLI stdout raw: {}", &line[..line.floor_char_boundary(300)]);

                let msg: CliMessage = match serde_json::from_str(&line) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(
                            "Skipping unparseable CLI line: {} — {}",
                            e,
                            &line[..line.floor_char_boundary(200)]
                        );
                        continue;
                    }
                };

                match msg {
                    CliMessage::System { .. } => {
                        tracing::debug!("CLI → system");
                    }

                    CliMessage::StreamEvent { event } => {
                        // Real-time SSE events from CLI — forward directly.
                        // Suppress per-turn lifecycle (message_start after first, message_stop, message_delta).
                        let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        streaming_via_events = true;

                        match event_type {
                            "message_start" => {
                                if !started {
                                    started = true;
                                    // Capture cache tokens from message_start usage
                                    if let Some(msg) = event.get("message")
                                        && let Some(u) = msg.get("usage")
                                        && let Ok(cli_u) =
                                            serde_json::from_value::<CliUsage>(u.clone())
                                    {
                                        input_tokens = cli_u.total_input();
                                    }
                                    match serde_json::from_value::<StreamEvent>(event) {
                                        Ok(mut se) => {
                                            // Patch input_tokens to include cache tokens
                                            if let StreamEvent::MessageStart { ref mut message } =
                                                se
                                            {
                                                message.usage.input_tokens = input_tokens;
                                            }
                                            if tx.send(Ok(se)).await.is_err() {
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Failed to parse message_start: {}", e);
                                        }
                                    }
                                }
                            }
                            "message_delta" => {
                                // Suppress event — Result handles final close.
                                // But capture usage (cache tokens) and track tool rounds.
                                let is_tool_round = event
                                    .get("delta")
                                    .and_then(|d| d.get("stop_reason"))
                                    .and_then(|r| r.as_str())
                                    == Some("tool_use");
                                if is_tool_round {
                                    block_index_offset += max_block_index_this_round;
                                    max_block_index_this_round = 0;
                                    tracing::debug!(
                                        "CLI tool round ended, block_index_offset={}",
                                        block_index_offset
                                    );
                                }
                                // Accumulate token usage from each round's message_delta.
                                // CLI reports cache_read/creation tokens here, not in Result.
                                if let Some(u) = event.get("usage") {
                                    let round_output = u
                                        .get("output_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                        as u32;
                                    output_tokens += round_output;
                                    // Take the max input (includes cache) — each round reports
                                    // the running total of cache_read_input_tokens.
                                    if let Ok(round_usage) =
                                        serde_json::from_value::<CliUsage>(u.clone())
                                    {
                                        let round_input = round_usage.total_input();
                                        if round_input > input_tokens {
                                            input_tokens = round_input;
                                        }
                                    }
                                }
                                // Send Ping to prevent idle timeout during tool execution
                                if tx.send(Ok(StreamEvent::Ping)).await.is_err() {
                                    break;
                                }
                            }
                            "message_stop" => {
                                // Suppress — Result handles final close.
                                // Send Ping to prevent idle timeout.
                                if tx.send(Ok(StreamEvent::Ping)).await.is_err() {
                                    break;
                                }
                            }
                            // Forward tool_use blocks as real stream events — the tool_loop
                            // will see cli_handles_tools() and emit ProgressEvents for TUI
                            // display without re-executing them.
                            "content_block_start"
                                if event
                                    .get("content_block")
                                    .and_then(|b| b.get("type"))
                                    .and_then(|t| t.as_str())
                                    == Some("tool_use") =>
                            {
                                match serde_json::from_value::<StreamEvent>(event.clone()) {
                                    Ok(se) => {
                                        // Track max block index for offset calculation
                                        if let StreamEvent::ContentBlockStart { index, .. } = &se {
                                            max_block_index_this_round =
                                                max_block_index_this_round.max(index + 1);
                                        }
                                        let se = offset_block_index(se, block_index_offset);
                                        if tx.send(Ok(se)).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::debug!(
                                            "Skipping tool_use content_block_start: {}",
                                            e
                                        );
                                    }
                                }
                            }
                            // Forward input_json_delta for tool_use blocks
                            "content_block_delta"
                                if event
                                    .get("delta")
                                    .and_then(|d| d.get("type"))
                                    .and_then(|t| t.as_str())
                                    == Some("input_json_delta") =>
                            {
                                match serde_json::from_value::<StreamEvent>(event.clone()) {
                                    Ok(se) => {
                                        let se = offset_block_index(se, block_index_offset);
                                        if tx.send(Ok(se)).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::debug!("Skipping input_json_delta: {}", e);
                                    }
                                }
                            }
                            _ => match serde_json::from_value::<StreamEvent>(event.clone()) {
                                Ok(se) => {
                                    // Track max block index for offset calculation
                                    match &se {
                                        StreamEvent::ContentBlockStart { index, .. }
                                        | StreamEvent::ContentBlockDelta { index, .. }
                                        | StreamEvent::ContentBlockStop { index } => {
                                            max_block_index_this_round =
                                                max_block_index_this_round.max(index + 1);
                                        }
                                        _ => {}
                                    }
                                    let se = offset_block_index(se, block_index_offset);
                                    let se = normalize_stream_event(se);
                                    if tx.send(Ok(se)).await.is_err() {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(
                                        "Skipping stream event '{}': {}",
                                        event_type,
                                        e
                                    );
                                }
                            },
                        }
                    }

                    CliMessage::Assistant { message } => {
                        // With --include-partial-messages, each assistant line has the FULL
                        // accumulated content so far. If streaming_via_events, these are
                        // redundant — just grab usage.
                        if streaming_via_events {
                            if let Some(u) = &message.usage {
                                output_tokens = u.output_tokens;
                            }
                            continue;
                        }

                        // Fallback path: no stream_events, use assistant messages for content.
                        // Emit MessageStart once.
                        if !started {
                            started = true;
                            let msg_id = message.id.unwrap_or_else(|| {
                                format!("msg_{}", uuid::Uuid::new_v4().simple())
                            });
                            let msg_model = message
                                .model
                                .map(|m| Self::normalize_model(&m))
                                .unwrap_or_else(|| original_model.clone());
                            let input_tokens =
                                message.usage.as_ref().map(|u| u.total_input()).unwrap_or(0);

                            let _ = tx
                                .send(Ok(StreamEvent::MessageStart {
                                    message: StreamMessage {
                                        id: msg_id,
                                        model: msg_model,
                                        role: Role::Assistant,
                                        usage: TokenUsage {
                                            input_tokens,
                                            output_tokens: 0,
                                        },
                                    },
                                }))
                                .await;
                        }

                        // Diff content blocks against what we already emitted.
                        let num_blocks = message.content.len();
                        for (i, block) in message.content.iter().enumerate() {
                            let is_last = i == num_blocks - 1;

                            // Already fully emitted — skip
                            if i < completed_blocks {
                                continue;
                            }

                            // New block appeared after current — close the previous one
                            if i > completed_blocks {
                                if current_block_started {
                                    let _ = tx
                                        .send(Ok(StreamEvent::ContentBlockStop {
                                            index: completed_blocks,
                                        }))
                                        .await;
                                    completed_blocks += 1;
                                    current_block_chars = 0;
                                    current_block_started = false;
                                }

                                // Emit any intermediate blocks fully
                                while completed_blocks < i {
                                    emit_full_block(
                                        &tx,
                                        &message.content[completed_blocks],
                                        completed_blocks,
                                    )
                                    .await;
                                    completed_blocks += 1;
                                }
                            }

                            let full_text = cli_block_text(block);

                            // Start block if needed
                            if !current_block_started {
                                let empty = cli_empty_block(block);
                                let _ = tx
                                    .send(Ok(StreamEvent::ContentBlockStart {
                                        index: i,
                                        content_block: empty,
                                    }))
                                    .await;
                                current_block_started = true;
                                current_block_chars = 0;
                            }

                            // Emit delta for NEW characters only
                            if full_text.len() > current_block_chars {
                                let new_text = &full_text[current_block_chars..];
                                if !new_text.is_empty() {
                                    let delta = cli_block_delta(block, new_text);
                                    let _ = tx
                                        .send(Ok(StreamEvent::ContentBlockDelta {
                                            index: i,
                                            delta,
                                        }))
                                        .await;
                                    current_block_chars = full_text.len();
                                }
                            }

                            // If not the last block, it's complete — close it
                            if !is_last {
                                let _ = tx
                                    .send(Ok(StreamEvent::ContentBlockStop { index: i }))
                                    .await;
                                completed_blocks += 1;
                                current_block_chars = 0;
                                current_block_started = false;
                            }
                        }

                        if let Some(u) = &message.usage {
                            output_tokens = u.output_tokens;
                        }
                    }

                    CliMessage::Result {
                        stop_reason,
                        usage,
                        is_error,
                        result,
                    } => {
                        // CLI returned an error (API failure, image processing, etc.)
                        // Surface it as a text block so the user sees what happened
                        // instead of silently dropping the response.
                        if is_error {
                            let error_text =
                                result.unwrap_or_else(|| "CLI returned an error".to_string());
                            tracing::error!("CLI result is_error=true: {}", error_text);

                            // Ensure message_start was sent
                            if !started {
                                started = true;
                                let _ = tx
                                    .send(Ok(StreamEvent::MessageStart {
                                        message: StreamMessage {
                                            id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
                                            model: original_model.clone(),
                                            role: Role::Assistant,
                                            usage: TokenUsage {
                                                input_tokens: 0,
                                                output_tokens: 0,
                                            },
                                        },
                                    }))
                                    .await;
                            }

                            // Close any open content block from partial streaming
                            if current_block_started {
                                let _ = tx
                                    .send(Ok(StreamEvent::ContentBlockStop {
                                        index: completed_blocks,
                                    }))
                                    .await;
                                completed_blocks += 1;
                            }

                            // Emit the error as a visible text block
                            let error_idx = completed_blocks + block_index_offset;
                            let _ = tx
                                .send(Ok(StreamEvent::ContentBlockStart {
                                    index: error_idx,
                                    content_block: ContentBlock::Text {
                                        text: String::new(),
                                    },
                                }))
                                .await;
                            let _ = tx
                                .send(Ok(StreamEvent::ContentBlockDelta {
                                    index: error_idx,
                                    delta: ContentDelta::TextDelta {
                                        text: format!("\n\n⚠️ CLI error: {}", error_text),
                                    },
                                }))
                                .await;
                            let _ = tx
                                .send(Ok(StreamEvent::ContentBlockStop { index: error_idx }))
                                .await;
                        } else {
                            // Close any open content block
                            if current_block_started {
                                let _ = tx
                                    .send(Ok(StreamEvent::ContentBlockStop {
                                        index: completed_blocks,
                                    }))
                                    .await;
                            }
                        }

                        let reason = stop_reason.map(|r| match r.as_str() {
                            "end_turn" => StopReason::EndTurn,
                            "tool_use" => StopReason::ToolUse,
                            "max_tokens" => StopReason::MaxTokens,
                            _ => StopReason::EndTurn,
                        });

                        let final_output = usage
                            .as_ref()
                            .map(|u| u.output_tokens)
                            .unwrap_or(output_tokens);
                        let final_input = usage
                            .as_ref()
                            .map(|u| u.total_input())
                            .unwrap_or(input_tokens);
                        if let Some(ref u) = usage {
                            tracing::debug!(
                                "CLI session complete: input={}, cache_create={}, cache_read={}, total_with_cache={}",
                                u.input_tokens,
                                u.cache_creation_input_tokens,
                                u.cache_read_input_tokens,
                                u.total_with_cache()
                            );
                        }

                        let _ = tx
                            .send(Ok(StreamEvent::MessageDelta {
                                delta: MessageDelta {
                                    stop_reason: reason,
                                    stop_sequence: None,
                                },
                                usage: TokenUsage {
                                    input_tokens: final_input,
                                    output_tokens: final_output,
                                },
                            }))
                            .await;

                        let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
                        break;
                    }

                    CliMessage::User { .. } => {
                        tracing::debug!("CLI → user turn (tool_result)");
                        // Keep the stream alive — tool results arrive during internal
                        // tool execution which can take >60s for complex tasks.
                        if tx.send(Ok(StreamEvent::Ping)).await.is_err() {
                            break;
                        }
                    }

                    CliMessage::RateLimitEvent {} => {
                        tracing::warn!("CLI → rate_limit_event");
                        // Keep the stream alive during rate-limit pauses
                        if tx.send(Ok(StreamEvent::Ping)).await.is_err() {
                            break;
                        }
                    }
                }
            }

            // Wait for process exit
            let exit_status = child.wait().await;
            match &exit_status {
                Ok(status) if !status.success() => {
                    tracing::warn!("claude CLI exited with status: {}", status);
                    if !started {
                        let _ = tx
                            .send(Err(ProviderError::Internal(format!(
                                "claude CLI exited with {} before producing any output",
                                status
                            ))))
                            .await;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to wait on claude CLI: {}", e);
                }
                Ok(_) => {
                    if !started {
                        tracing::warn!(
                            "claude CLI exited successfully but produced no stream events"
                        );
                    }
                }
            }
        });

        // Convert mpsc receiver to a Stream via unfold
        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        "claude-cli"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "sonnet".to_string(),
            "opus".to_string(),
            "haiku".to_string(),
            "sonnet-4-6".to_string(),
            "opus-4-6".to_string(),
            "haiku-4-5".to_string(),
        ]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(200_000) // Claude models support 200k context
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        crate::pricing::PricingConfig::load().calculate_cost(model, input_tokens, output_tokens)
    }

    fn supports_tools(&self) -> bool {
        true // CLI handles tools internally
    }

    fn supports_vision(&self) -> bool {
        false // CLI -p mode cannot process images — use analyze_image fallback
    }

    fn cli_handles_tools(&self) -> bool {
        true // CLI executes tools internally — display only, don't re-execute
    }
}

// ── Helper functions for assistant message → StreamEvent translation ──

/// Extract text content from a CLI content block.
fn cli_block_text(block: &CliContentBlock) -> &str {
    match block {
        CliContentBlock::Text { text } => text.as_str(),
        CliContentBlock::Thinking { thinking } => thinking.as_str(),
        _ => "",
    }
}

/// Create an empty ContentBlock for content_block_start.
fn cli_empty_block(block: &CliContentBlock) -> ContentBlock {
    match block {
        CliContentBlock::Text { .. } => ContentBlock::Text {
            text: String::new(),
        },
        CliContentBlock::Thinking { .. } => ContentBlock::Thinking {
            thinking: String::new(),
            signature: None,
        },
        CliContentBlock::ToolUse { id, name, .. } => ContentBlock::ToolUse {
            id: id.clone(),
            name: normalize_cli_tool_name(name),
            input: serde_json::json!({}),
        },
        CliContentBlock::Unknown => ContentBlock::Text {
            text: String::new(),
        },
    }
}

/// Create the appropriate ContentDelta for a block type.
fn cli_block_delta(block: &CliContentBlock, new_text: &str) -> ContentDelta {
    match block {
        CliContentBlock::Thinking { .. } => ContentDelta::ThinkingDelta {
            thinking: new_text.to_string(),
        },
        _ => ContentDelta::TextDelta {
            text: new_text.to_string(),
        },
    }
}

/// Emit a complete block (start + delta + stop) in one shot.
async fn emit_full_block(
    tx: &tokio::sync::mpsc::Sender<super::error::Result<StreamEvent>>,
    block: &CliContentBlock,
    index: usize,
) {
    let empty = cli_empty_block(block);
    let _ = tx
        .send(Ok(StreamEvent::ContentBlockStart {
            index,
            content_block: empty,
        }))
        .await;

    match block {
        CliContentBlock::Text { text } => {
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockDelta {
                    index,
                    delta: ContentDelta::TextDelta { text: text.clone() },
                }))
                .await;
        }
        CliContentBlock::Thinking { thinking } => {
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockDelta {
                    index,
                    delta: ContentDelta::ThinkingDelta {
                        thinking: thinking.clone(),
                    },
                }))
                .await;
        }
        CliContentBlock::ToolUse { input, .. } => {
            let input_str = serde_json::to_string(input).unwrap_or_default();
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockDelta {
                    index,
                    delta: ContentDelta::InputJsonDelta {
                        partial_json: input_str,
                    },
                }))
                .await;
        }
        CliContentBlock::Unknown => {}
    }

    let _ = tx.send(Ok(StreamEvent::ContentBlockStop { index })).await;
}

/// Normalize Claude Code CLI tool names to OpenCrabs format.
fn normalize_cli_tool_name(name: &str) -> String {
    match name {
        "Bash" => "bash".to_string(),
        "Read" => "read_file".to_string(),
        "Write" => "write_file".to_string(),
        "Edit" => "edit_file".to_string(),
        "Grep" => "grep".to_string(),
        "Glob" => "glob".to_string(),
        "LSP" => "lsp".to_string(),
        "WebSearch" => "web_search".to_string(),
        "WebFetch" => "http_request".to_string(),
        "Agent" => "agent".to_string(),
        "NotebookEdit" => "notebook_edit".to_string(),
        other => other.to_string(),
    }
}

/// Offset content block indices in a StreamEvent by the given amount.
/// This prevents collisions when multiple CLI tool rounds produce blocks
/// with indices restarting at 0.
fn offset_block_index(event: StreamEvent, offset: usize) -> StreamEvent {
    if offset == 0 {
        return event;
    }
    match event {
        StreamEvent::ContentBlockStart {
            index,
            content_block,
        } => StreamEvent::ContentBlockStart {
            index: index + offset,
            content_block,
        },
        StreamEvent::ContentBlockDelta { index, delta } => StreamEvent::ContentBlockDelta {
            index: index + offset,
            delta,
        },
        StreamEvent::ContentBlockStop { index } => StreamEvent::ContentBlockStop {
            index: index + offset,
        },
        other => other,
    }
}

/// Normalize tool names in stream events from CLI to OpenCrabs format.
fn normalize_stream_event(event: StreamEvent) -> StreamEvent {
    match event {
        StreamEvent::ContentBlockStart {
            index,
            content_block: ContentBlock::ToolUse { id, name, input },
        } => StreamEvent::ContentBlockStart {
            index,
            content_block: ContentBlock::ToolUse {
                id,
                name: normalize_cli_tool_name(&name),
                input,
            },
        },
        other => other,
    }
}
