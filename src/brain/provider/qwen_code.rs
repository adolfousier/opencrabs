//! Qwen Code CLI Provider — direct subprocess integration
//!
//! Spawns the `qwen` CLI binary as a text completion backend and reads
//! its NDJSON stream output, converting it to standard `StreamEvent`s.
//! OpenCrabs handles all tools, memory, and context locally.
//!
//! Binary: `qwen` (npm: @qwen-code/qwen-code)
//! Free tier: 1,000 req/day via Qwen OAuth (hard-coded, no config needed).
//! Also supports OpenAI/Anthropic/Gemini-compatible APIs via settings.json.

use super::error::{ProviderError, Result};
use super::r#trait::{Provider, ProviderStream};
use super::types::*;
use async_trait::async_trait;
use futures::stream::StreamExt;
use serde::Deserialize;
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;

/// Qwen Code CLI provider — talks directly to the `qwen` binary.
#[derive(Clone)]
pub struct QwenCodeCliProvider {
    qwen_path: String,
    default_model: String,
}

impl QwenCodeCliProvider {
    /// Create a new provider, auto-detecting the qwen binary.
    pub fn new() -> Result<Self> {
        let path = resolve_qwen_path()?;
        Ok(Self {
            qwen_path: path,
            default_model: "qwen3-coder-plus".to_string(),
        })
    }

    /// Override the default model.
    pub fn with_default_model(mut self, model: String) -> Self {
        self.default_model = model;
        self
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
                        Some(match source {
                            ImageSource::Base64 { media_type, data } => {
                                let ext = match media_type.as_str() {
                                    "image/png" => "png",
                                    "image/jpeg" => "jpeg",
                                    "image/gif" => "gif",
                                    "image/webp" => "webp",
                                    _ => "png",
                                };
                                let tmp = std::env::temp_dir().join(format!(
                                    "opencrabs_qwen_img_{}.{}",
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

/// Resolve the qwen CLI binary path.
fn resolve_qwen_path() -> Result<String> {
    if let Ok(path) = std::env::var("QWEN_CODE_PATH") {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
        return Err(ProviderError::Internal(format!(
            "QWEN_CODE_PATH set but not found: {}",
            path
        )));
    }

    // Try common install locations (npm global, brew, home install script)
    for candidate in &["/opt/homebrew/bin/qwen", "/usr/local/bin/qwen"] {
        if std::path::Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }

    // Try PATH via `which`
    if let Ok(output) = std::process::Command::new("which").arg("qwen").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }

    // Also try `qwen-code` as the binary name (some installs use this)
    if let Ok(output) = std::process::Command::new("which")
        .arg("qwen-code")
        .output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }

    Err(ProviderError::Internal(
        "qwen CLI not found — install with `npm install -g @qwen-code/qwen-code` or set QWEN_CODE_PATH".to_string(),
    ))
}

// ── CLI NDJSON types — Qwen uses the same Gemini/Anthropic protocol ──

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
    StreamEvent {
        event: serde_json::Value,
    },
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
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

impl CliUsage {
    fn total_input(&self) -> u32 {
        self.input_tokens
    }
}

#[async_trait]
impl Provider for QwenCodeCliProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        let mut stream = self.stream(request).await?;

        let mut id = String::new();
        let mut model = String::new();
        let mut content = Vec::new();
        let mut stop_reason = None;
        let mut usage = TokenUsage::default();
        let mut text_buf = String::new();

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::MessageStart { message } => {
                    id = message.id;
                    model = message.model;
                    usage = message.usage;
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
                    if u.cache_creation_tokens > 0 {
                        usage.cache_creation_tokens = u.cache_creation_tokens;
                    }
                    if u.cache_read_tokens > 0 {
                        usage.cache_read_tokens = u.cache_read_tokens;
                    }
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
        let model = request.model.clone();

        let cwd = request
            .working_directory
            .as_deref()
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")));

        tracing::info!(
            "Spawning qwen CLI: model={}, prompt_len={}, cwd={}",
            model,
            prompt.len(),
            cwd.display()
        );

        let session_id_str = uuid::Uuid::new_v4().to_string();

        let mut child = tokio::process::Command::new(&self.qwen_path)
            .env_remove("QWEN_CODE")
            .arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--include-partial-messages")
            .arg("--session-id")
            .arg(&session_id_str)
            .arg("--yolo")
            // Qwen Code CLI has no `--settings` flag and no env var to
            // disable the `Co-authored-by: Qwen-Coder` git-commit trailer
            // (the `general.gitCoAuthor` setting is only configurable via
            // ~/.qwen/settings.json). Force it off at the system-prompt
            // level so every spawn — including brand-new installs — never
            // ships an attribution trailer we didn't authorize.
            .arg("--append-system-prompt")
            .arg(
                "HARD RULE: When creating git commits, NEVER append any \
                 `Co-authored-by:` trailer (Qwen-Coder, Qwen, or otherwise) \
                 and NEVER add `Generated with Qwen Code` footers. Commit \
                 messages must contain only the user-authored content.",
            )
            .arg("--model")
            .arg(&model)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ProviderError::Internal(format!("failed to spawn qwen CLI: {}", e)))?;

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

        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if !line.is_empty() {
                    tracing::warn!("qwen CLI stderr: {}", line);
                }
            }
        });

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamEvent>>(64);

        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();

            let mut started = false;
            let mut streaming_via_events = false;
            let mut completed_blocks: usize = 0;
            let mut current_block_started = false;
            let mut current_block_chars: usize = 0;
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut cache_creation_tokens_last: u32 = 0;
            let mut cache_read_tokens_last: u32 = 0;
            let mut cache_creation_tokens_billing: u32 = 0;
            let mut cache_read_tokens_billing: u32 = 0;
            let mut result_received = false;
            let mut line_count = 0u32;
            let mut block_index_offset: usize = 0;
            let mut max_block_index_this_round: usize = 0;
            // Tracks whether any tool_use block was seen during the entire stream,
            // logged at EOF for diagnostics. Qwen CLI never emits a `result`
            // envelope, so MessageStop is synthesized on EOF.
            let mut saw_tool_use_in_message = false;

            loop {
                let line_result = tokio::select! {
                    biased;
                    _ = tx.closed() => {
                        tracing::info!("Qwen CLI stream cancelled — killing subprocess");
                        let _ = child.kill().await;
                        break;
                    }
                    result = lines.next_line() => result,
                };
                let line = match line_result {
                    Ok(Some(line)) => line,
                    Ok(None) => {
                        tracing::info!(
                            "Qwen CLI stdout EOF after {} lines (started={})",
                            line_count,
                            started
                        );
                        break;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Qwen CLI stdout read error after {} lines: {}",
                            line_count,
                            e
                        );
                        break;
                    }
                };
                line_count += 1;
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                tracing::debug!(
                    "Qwen CLI stdout raw: {}",
                    &line[..line.floor_char_boundary(2000)]
                );

                let msg: CliMessage = match serde_json::from_str(&line) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(
                            "Skipping unparseable Qwen CLI line: {} — {}",
                            e,
                            &line[..line.floor_char_boundary(200)]
                        );
                        continue;
                    }
                };

                match msg {
                    CliMessage::System { .. } => {
                        tracing::debug!("Qwen CLI → system");
                    }

                    CliMessage::StreamEvent { event } => {
                        let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        streaming_via_events = true;

                        match event_type {
                            "message_start" => {
                                if !started {
                                    started = true;
                                    // Qwen's `message_start` often ships WITHOUT a
                                    // `usage` field, which makes the strict
                                    // `StreamEvent::MessageStart` deserializer fail.
                                    // Build the event manually so missing/loose
                                    // fields can't break the stream header — without
                                    // it, the agent loop never sees a stream begin
                                    // and treats the whole turn as dropped.
                                    if let Some(msg) = event.get("message")
                                        && let Some(u) = msg.get("usage")
                                        && let Ok(cli_u) =
                                            serde_json::from_value::<CliUsage>(u.clone())
                                    {
                                        input_tokens = cli_u.total_input();
                                    }
                                    let id = event
                                        .get("message")
                                        .and_then(|m| m.get("id"))
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string)
                                        .unwrap_or_else(|| {
                                            format!("msg_{}", uuid::Uuid::new_v4().simple())
                                        });
                                    let model_name = event
                                        .get("message")
                                        .and_then(|m| m.get("model"))
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string)
                                        .unwrap_or_else(|| request.model.clone());
                                    let se = StreamEvent::MessageStart {
                                        message: StreamMessage {
                                            id,
                                            model: model_name,
                                            role: Role::Assistant,
                                            usage: TokenUsage {
                                                input_tokens,
                                                output_tokens: 0,
                                                ..Default::default()
                                            },
                                        },
                                    };
                                    if tx.send(Ok(se)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            "message_delta" => {
                                let is_tool_round = event
                                    .get("delta")
                                    .and_then(|d| d.get("stop_reason"))
                                    .and_then(|r| r.as_str())
                                    == Some("tool_use");
                                if is_tool_round {
                                    block_index_offset += max_block_index_this_round;
                                    max_block_index_this_round = 0;
                                }
                                if let Some(u) = event.get("usage") {
                                    let round_output = u
                                        .get("output_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                        as u32;
                                    output_tokens += round_output;
                                    if let Ok(round_usage) =
                                        serde_json::from_value::<CliUsage>(u.clone())
                                    {
                                        let round_input = round_usage.total_input();
                                        if round_input > input_tokens {
                                            input_tokens = round_input;
                                        }
                                        cache_creation_tokens_last =
                                            round_usage.cache_creation_input_tokens;
                                        cache_read_tokens_last =
                                            round_usage.cache_read_input_tokens;
                                        cache_creation_tokens_billing +=
                                            round_usage.cache_creation_input_tokens;
                                        cache_read_tokens_billing +=
                                            round_usage.cache_read_input_tokens;
                                    }
                                }
                                if tx.send(Ok(StreamEvent::Ping)).await.is_err() {
                                    break;
                                }
                            }
                            "message_stop" => {
                                // NOT the end of conversation — qwen CLI in --yolo
                                // mode runs its own tools and continues with more
                                // assistant messages. Each `message_stop` just
                                // marks the end of one assistant turn. The real
                                // termination is process EOF (handled below).
                                tracing::debug!(
                                    "Qwen CLI → message_stop (saw_tool_use={})",
                                    saw_tool_use_in_message
                                );
                                if tx.send(Ok(StreamEvent::Ping)).await.is_err() {
                                    break;
                                }
                            }
                            "content_block_start"
                                if event
                                    .get("content_block")
                                    .and_then(|b| b.get("type"))
                                    .and_then(|t| t.as_str())
                                    == Some("tool_use") =>
                            {
                                // Track that we saw a tool_use, but DO forward
                                // the block downstream so it gets persisted to
                                // the DB and rendered in the TUI. The agent
                                // loop short-circuits actual execution via
                                // `cli_handles_tools()`.
                                saw_tool_use_in_message = true;
                                match serde_json::from_value::<StreamEvent>(event.clone()) {
                                    Ok(se) => {
                                        if let StreamEvent::ContentBlockStart { index, .. } = &se {
                                            max_block_index_this_round =
                                                max_block_index_this_round.max(index + 1);
                                        }
                                        let se = offset_block_index(se, block_index_offset);
                                        let se = normalize_stream_event(se);
                                        if tx.send(Ok(se)).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::debug!("tool_use content_block_start: {}", e);
                                    }
                                }
                            }
                            _ => match serde_json::from_value::<StreamEvent>(event.clone()) {
                                Ok(se) => {
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
                        if streaming_via_events {
                            if let Some(u) = &message.usage {
                                // Each `assistant` envelope reports the usage
                                // for ONE agentic round (one model call). Track
                                // per-round LAST values for context-window
                                // display, and accumulate billing separately.
                                // Without this, the per-round counters stay 0
                                // for qwen (it never emits `message_delta`),
                                // and the calibration falls back to the
                                // cumulative `result.usage` which inflates
                                // ctx % by num_turns (e.g. 2700% on 19 turns).
                                output_tokens = u.output_tokens;
                                input_tokens = u.input_tokens;
                                cache_creation_tokens_last = u.cache_creation_input_tokens;
                                cache_read_tokens_last = u.cache_read_input_tokens;
                                cache_creation_tokens_billing += u.cache_creation_input_tokens;
                                cache_read_tokens_billing += u.cache_read_input_tokens;
                                tracing::info!(
                                    "Qwen per-round usage: in={}, out={}, cc={}, cr={}",
                                    u.input_tokens,
                                    u.output_tokens,
                                    u.cache_creation_input_tokens,
                                    u.cache_read_input_tokens,
                                );
                            }
                            continue;
                        }

                        if !started {
                            started = true;
                            let msg_id = message.id.unwrap_or_else(|| {
                                format!("msg_{}", uuid::Uuid::new_v4().simple())
                            });
                            let msg_model = message
                                .model
                                .clone()
                                .unwrap_or_else(|| request.model.clone());
                            let (input_tokens, cc, cr) = message
                                .usage
                                .as_ref()
                                .map(|u| {
                                    (
                                        u.total_input(),
                                        u.cache_creation_input_tokens,
                                        u.cache_read_input_tokens,
                                    )
                                })
                                .unwrap_or((0, 0, 0));

                            let _ = tx
                                .send(Ok(StreamEvent::MessageStart {
                                    message: StreamMessage {
                                        id: msg_id,
                                        model: msg_model,
                                        role: Role::Assistant,
                                        usage: TokenUsage {
                                            input_tokens,
                                            output_tokens: 0,
                                            cache_creation_tokens: cc,
                                            cache_read_tokens: cr,
                                            ..Default::default()
                                        },
                                    },
                                }))
                                .await;
                        }

                        let num_blocks = message.content.len();
                        for (i, block) in message.content.iter().enumerate() {
                            let is_last = i == num_blocks - 1;

                            if i < completed_blocks {
                                continue;
                            }

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
                        if is_error {
                            let error_text =
                                result.unwrap_or_else(|| "CLI returned an error".to_string());
                            tracing::error!("Qwen CLI result is_error=true: {}", error_text);

                            let error_lower = error_text.to_lowercase();

                            if error_lower.contains("prompt is too long")
                                || error_lower.contains("too many tokens")
                                || error_lower.contains("context length")
                            {
                                let _ = tx.send(Err(ProviderError::ContextLengthExceeded(0))).await;
                                break;
                            }

                            if error_lower.contains("rate limit")
                                || error_lower.contains("hit your limit")
                                || error_lower.contains("overloaded")
                                || error_lower.contains("too many requests")
                                || error_lower.contains("capacity")
                                || error_lower.contains("429")
                            {
                                tracing::warn!(
                                    "Qwen CLI rate/account limit — returning RateLimitExceeded"
                                );
                                let _ = tx
                                    .send(Err(ProviderError::RateLimitExceeded(error_text)))
                                    .await;
                                break;
                            }

                            if !started {
                                started = true;
                                let _ = tx
                                    .send(Ok(StreamEvent::MessageStart {
                                        message: StreamMessage {
                                            id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
                                            model: request.model.clone(),
                                            role: Role::Assistant,
                                            usage: TokenUsage {
                                                input_tokens: 0,
                                                output_tokens: 0,
                                                ..Default::default()
                                            },
                                        },
                                    }))
                                    .await;
                            }

                            if current_block_started {
                                let _ = tx
                                    .send(Ok(StreamEvent::ContentBlockStop {
                                        index: completed_blocks,
                                    }))
                                    .await;
                                completed_blocks += 1;
                            }

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
                            if current_block_started {
                                let _ = tx
                                    .send(Ok(StreamEvent::ContentBlockStop {
                                        index: completed_blocks,
                                    }))
                                    .await;
                            }
                        }

                        // Qwen CLI's `result` envelope often omits `stop_reason`
                        // entirely on success. Default to EndTurn so the agent
                        // loop doesn't treat the stream as dropped and retry,
                        // which manifested as the same response looping 5–6 times.
                        let reason = Some(
                            stop_reason
                                .map(|r| match r.as_str() {
                                    "end_turn" => StopReason::EndTurn,
                                    "tool_use" => StopReason::ToolUse,
                                    "max_tokens" => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                })
                                .unwrap_or(StopReason::EndTurn),
                        );

                        // Output tokens from result.usage are cumulative across
                        // all agentic rounds — correct for billing/display of
                        // "tokens generated this turn".
                        let final_output = usage
                            .as_ref()
                            .map(|u| u.output_tokens)
                            .unwrap_or(output_tokens);

                        // Pull cumulative billing values from result.usage.
                        // These represent the TOTAL tokens consumed across all
                        // sub-rounds — the right thing for cost calculation.
                        let (result_input, result_cc, result_cr) = usage
                            .as_ref()
                            .map(|u| {
                                (
                                    u.input_tokens,
                                    u.cache_creation_input_tokens,
                                    u.cache_read_input_tokens,
                                )
                            })
                            .unwrap_or((0, 0, 0));

                        // CRITICAL: result.usage is CUMULATIVE across rounds
                        // (verified from logs: num_turns=1 → 136K input, but
                        // num_turns=19 → 4.8M input — ~36× because each round
                        // re-includes the growing context AND tool/file content
                        // which is also pulled in cumulatively). It is the
                        // WRONG value for context-window display, which needs
                        // the size of the LAST round's prompt only.
                        //
                        // The per-round trackers (input_tokens, *_last) are
                        // populated by the Assistant envelope handler when
                        // qwen-cli ships per-round usage on its assistant
                        // messages. If those are present, use them. Otherwise,
                        // emit ZERO for context fields so tool_loop's
                        // calibration is skipped entirely and the local
                        // tiktoken estimate of context.token_count is kept
                        // (which matches what the user sees after restart).
                        let ctx_input = input_tokens;
                        let ctx_cache_creation = cache_creation_tokens_last;
                        let ctx_cache_read = cache_read_tokens_last;

                        // Billing accumulates across rounds for cost tracking.
                        // Prefer the cumulative tracker; if we never received
                        // per-round assistant usage, fall back to result.usage
                        // (which is also cumulative).
                        let billing_cache_creation = if cache_creation_tokens_billing > 0 {
                            cache_creation_tokens_billing
                        } else {
                            result_cc
                        };
                        let billing_cache_read = if cache_read_tokens_billing > 0 {
                            cache_read_tokens_billing
                        } else {
                            result_cr
                        };

                        tracing::info!(
                            "Qwen result envelope: cumulative result in={} cc={} cr={} \
                             out={} num_turns_seen | per-round LAST in={} cc={} cr={} \
                             (ctx: {})",
                            result_input,
                            result_cc,
                            result_cr,
                            final_output,
                            input_tokens,
                            cache_creation_tokens_last,
                            cache_read_tokens_last,
                            if ctx_input + ctx_cache_creation + ctx_cache_read > 0 {
                                "using per-round LAST"
                            } else {
                                "skipped — falling back to local tiktoken estimate"
                            },
                        );

                        let _ = tx
                            .send(Ok(StreamEvent::MessageDelta {
                                delta: MessageDelta {
                                    stop_reason: reason,
                                    stop_sequence: None,
                                },
                                usage: TokenUsage {
                                    input_tokens: ctx_input,
                                    output_tokens: final_output,
                                    cache_creation_tokens: ctx_cache_creation,
                                    cache_read_tokens: ctx_cache_read,
                                    billing_cache_creation,
                                    billing_cache_read,
                                },
                            }))
                            .await;

                        let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
                        result_received = true;
                        break;
                    }

                    CliMessage::User { .. } => {
                        tracing::debug!("Qwen CLI → user turn (tool_result)");
                        if tx.send(Ok(StreamEvent::Ping)).await.is_err() {
                            break;
                        }
                    }

                    CliMessage::RateLimitEvent {} => {
                        tracing::warn!("Qwen CLI → rate_limit_event");
                        if tx.send(Ok(StreamEvent::Ping)).await.is_err() {
                            break;
                        }
                    }
                }
            }

            // Synthesize MessageStop on EOF if qwen never sent a `result` envelope.
            // This is the normal path for qwen CLI (it does not emit a result like
            // claude CLI does). Always emit so the agent loop sees a valid
            // stop_reason and doesn't retry the request.
            if started && !result_received {
                if current_block_started {
                    let _ = tx
                        .send(Ok(StreamEvent::ContentBlockStop {
                            index: completed_blocks + block_index_offset,
                        }))
                        .await;
                }
                tracing::info!(
                    "Qwen CLI EOF — synthesizing MessageStop (saw_tool_use={}, in={}, out={})",
                    saw_tool_use_in_message,
                    input_tokens,
                    output_tokens
                );
                let _ = tx
                    .send(Ok(StreamEvent::MessageDelta {
                        delta: MessageDelta {
                            stop_reason: Some(StopReason::EndTurn),
                            stop_sequence: None,
                        },
                        usage: TokenUsage {
                            input_tokens,
                            output_tokens,
                            cache_creation_tokens: cache_creation_tokens_last,
                            cache_read_tokens: cache_read_tokens_last,
                            billing_cache_creation: cache_creation_tokens_billing,
                            billing_cache_read: cache_read_tokens_billing,
                        },
                    }))
                    .await;
                let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
            }

            let exit_status = child.wait().await;
            match &exit_status {
                Ok(status) if !status.success() => {
                    if !started {
                        let _ = tx
                            .send(Err(ProviderError::Internal(format!(
                                "qwen CLI exited with {} before producing any output",
                                status
                            ))))
                            .await;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to wait on qwen CLI: {}", e);
                }
                Ok(_) => {
                    if !started {
                        tracing::warn!(
                            "qwen CLI exited successfully but produced no stream events"
                        );
                    }
                }
            }
        });

        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        "qwen-cli"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "qwen3-coder-plus".to_string(),
            "qwen3.5-plus".to_string(),
            "qwen3.6-plus".to_string(),
            "qwen3-coder-480a35".to_string(),
            "qwen3-coder-30ba3b".to_string(),
            "qwen3-max-2026-01-23".to_string(),
        ]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(256_000) // Qwen3-Coder supports 256k context
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        crate::pricing::PricingConfig::load().calculate_cost(model, input_tokens, output_tokens)
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
        false
    }

    fn cli_handles_tools(&self) -> bool {
        true
    }

    /// Qwen CLI is spawned cold on every turn with a fresh `--session-id`,
    /// and the entire message history is re-serialized into the `-p` prompt
    /// (see `build_prompt`). It has zero persistent state across our calls,
    /// so OpenCrabs must compact context itself to stay under the window.
    fn cli_manages_context(&self) -> bool {
        false
    }
}

// ── Helper functions (same as claude_cli) ──

fn cli_block_text(block: &CliContentBlock) -> &str {
    match block {
        CliContentBlock::Text { text } => text.as_str(),
        CliContentBlock::Thinking { thinking } => thinking.as_str(),
        _ => "",
    }
}

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

fn normalize_cli_tool_name(name: &str) -> String {
    match name {
        // Claude-CLI style (capitalized)
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
        // Qwen Code CLI built-in tool names → opencrabs equivalents
        "run_shell_command" => "bash".to_string(),
        "search_file_content" => "grep".to_string(),
        "replace" => "edit_file".to_string(),
        "web_fetch" => "http_request".to_string(),
        "google_web_search" => "web_search".to_string(),
        "list_directory" => "glob".to_string(),
        "read_many_files" => "read_file".to_string(),
        other => other.to_string(),
    }
}

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
