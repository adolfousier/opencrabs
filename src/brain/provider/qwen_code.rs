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
    for candidate in &[
        "/opt/homebrew/bin/qwen",
        "/usr/local/bin/qwen",
    ] {
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
    if let Ok(output) = std::process::Command::new("which").arg("qwen-code").output()
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
                        tracing::error!("Qwen CLI stdout read error after {} lines: {}", line_count, e);
                        break;
                    }
                };
                line_count += 1;
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                tracing::debug!("Qwen CLI stdout raw: {}", &line[..line.floor_char_boundary(300)]);

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
                                    if let Some(msg) = event.get("message")
                                        && let Some(u) = msg.get("usage")
                                        && let Ok(cli_u) =
                                            serde_json::from_value::<CliUsage>(u.clone())
                                    {
                                        input_tokens = cli_u.total_input();
                                    }
                                    match serde_json::from_value::<StreamEvent>(event) {
                                        Ok(mut se) => {
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
                                match serde_json::from_value::<StreamEvent>(event.clone()) {
                                    Ok(se) => {
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
                                output_tokens = u.output_tokens;
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

                        let (result_cc, result_cr) = usage
                            .as_ref()
                            .map(|u| (u.cache_creation_input_tokens, u.cache_read_input_tokens))
                            .unwrap_or((0, 0));

                        let ctx_cache_creation = if cache_creation_tokens_last > 0 {
                            cache_creation_tokens_last
                        } else {
                            result_cc
                        };
                        let ctx_cache_read = if cache_read_tokens_last > 0 {
                            cache_read_tokens_last
                        } else {
                            result_cr
                        };

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

                        let _ = tx
                            .send(Ok(StreamEvent::MessageDelta {
                                delta: MessageDelta {
                                    stop_reason: reason,
                                    stop_sequence: None,
                                },
                                usage: TokenUsage {
                                    input_tokens: final_input,
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

            if started && !result_received && (input_tokens > 0 || output_tokens > 0) {
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
                            ..Default::default()
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
                        tracing::warn!("qwen CLI exited successfully but produced no stream events");
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
        "qwen-code"
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
