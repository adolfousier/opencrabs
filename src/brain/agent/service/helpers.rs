use super::builder::AgentService;
use super::types::{ProgressCallback, ProgressEvent};
use crate::brain::provider::{
    ContentBlock, ImageSource, LLMRequest, LLMResponse, Message, Role, StopReason,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

impl AgentService {
    /// Actual token count for the serialized tool schemas (cached per call).
    pub(super) fn actual_tool_schema_tokens(&self) -> usize {
        crate::brain::tokenizer::count_tokens(
            &serde_json::to_string(&self.tool_registry.get_tool_definitions()).unwrap_or_default(),
        )
    }

    /// Stream a request and accumulate into an LLMResponse.
    ///
    /// Sends text deltas to the progress callback as `StreamingChunk` events
    /// so the TUI can display them in real-time. Returns the full response
    /// once the stream completes, ready for tool extraction.
    ///
    /// `override_cb` takes precedence over the service-level `self.progress_callback`
    /// so per-call callbacks (e.g. Telegram) receive real-time streaming chunks.
    pub(super) async fn stream_complete(
        &self,
        session_id: Uuid,
        request: LLMRequest,
        cancel_token: Option<&CancellationToken>,
        override_cb: Option<&ProgressCallback>,
    ) -> std::result::Result<(LLMResponse, Option<String>), crate::brain::provider::ProviderError>
    {
        use crate::brain::provider::{ContentDelta, StreamEvent, TokenUsage};
        use futures::StreamExt;

        // Per-call override wins over service-level callback
        let effective_cb: Option<&ProgressCallback> =
            override_cb.or(self.progress_callback.as_ref());

        let request_model = request.model.clone();
        let provider = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .clone();
        let mut stream = provider.stream(request).await?;

        // Accumulate state from stream events
        let mut id = String::new();
        let mut model = String::new();
        let mut stop_reason: Option<StopReason> = None;
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;

        // --- Text repetition detection ---
        // Some providers (e.g. MiniMax) loop the same content indefinitely without
        // sending a stop signal. We keep a sliding window of recent text chunks and
        // detect when a long enough substring repeats, indicating a stuck loop.
        let mut total_text_len: usize = 0;
        let mut text_window = String::new(); // rolling window of recent text
        const REPEAT_WINDOW: usize = 2048; // bytes to keep in window
        const REPEAT_MIN_MATCH: usize = 200; // minimum repeated substring to trigger

        // Track partial content blocks by index
        // Text blocks: accumulate text deltas
        // ToolUse blocks: accumulate JSON deltas
        struct BlockState {
            block: ContentBlock,
            json_buf: String, // for tool use JSON accumulation
        }
        let mut block_states: Vec<BlockState> = Vec::new();
        let mut reasoning_buf = String::new();

        // Maximum idle time between SSE events before treating as a dropped connection.
        // NVIDIA/Kimi and some other providers occasionally hang silently without sending
        // [DONE] — this timeout lets the retry logic in tool_loop.rs recover instead of
        // blocking the TUI forever.
        const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

        loop {
            // Race stream.next() against cancellation token and idle timeout.
            // This ensures /stop takes effect immediately even mid-chunk.
            let next = tokio::select! {
                biased;
                _ = async {
                    if let Some(token) = cancel_token {
                        token.cancelled().await;
                    } else {
                        // No cancel token — never resolves
                        std::future::pending::<()>().await;
                    }
                } => {
                    tracing::info!("Stream cancelled by user");
                    break;
                }
                result = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()) => {
                    match result {
                        Ok(Some(item)) => item,
                        Ok(None) => break, // Stream ended normally
                        Err(_elapsed) => {
                            tracing::warn!(
                                "⏱️ Stream idle timeout after {}s — no event received from provider. \
                                 Treating as dropped stream (stop_reason=None → will retry).",
                                STREAM_IDLE_TIMEOUT.as_secs()
                            );
                            break; // stop_reason stays None → triggers retry in tool_loop
                        }
                    }
                }
            };

            let event = match next {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Stream error: {}", e);
                    return Err(e);
                }
            };

            match event {
                StreamEvent::MessageStart { message } => {
                    id = message.id;
                    model = message.model;
                    input_tokens = message.usage.input_tokens;
                }
                StreamEvent::ContentBlockStart {
                    index,
                    content_block,
                } => {
                    // Ensure block_states has enough capacity
                    while block_states.len() <= index {
                        block_states.push(BlockState {
                            block: ContentBlock::Text {
                                text: String::new(),
                            },
                            json_buf: String::new(),
                        });
                    }
                    block_states[index] = BlockState {
                        block: content_block,
                        json_buf: String::new(),
                    };
                }
                StreamEvent::ContentBlockDelta { index, delta } => {
                    if index < block_states.len() {
                        match delta {
                            ContentDelta::TextDelta { text } => {
                                // Forward to TUI / per-call callback for real-time display
                                if let Some(cb) = effective_cb {
                                    cb(
                                        session_id,
                                        ProgressEvent::StreamingChunk { text: text.clone() },
                                    );
                                }
                                // Accumulate into block
                                if let ContentBlock::Text { text: ref mut t } =
                                    block_states[index].block
                                {
                                    t.push_str(&text);
                                }

                                // --- Repetition & size detection ---
                                total_text_len += text.len();
                                text_window.push_str(&text);
                                if text_window.len() > REPEAT_WINDOW {
                                    let mut drain = text_window.len() - REPEAT_WINDOW;
                                    // Advance to a valid char boundary
                                    while !text_window.is_char_boundary(drain)
                                        && drain < text_window.len()
                                    {
                                        drain += 1;
                                    }
                                    text_window.drain(..drain);
                                }

                                // Check for repeated substring in window
                                if detect_text_repetition(&text_window, REPEAT_MIN_MATCH) {
                                    tracing::warn!(
                                        "🔁 Repetition detected in streaming response after {} bytes. \
                                         Provider appears to be looping. Terminating stream.",
                                        total_text_len,
                                    );
                                    stop_reason = Some(StopReason::EndTurn);
                                    break;
                                }
                            }
                            ContentDelta::InputJsonDelta { partial_json } => {
                                block_states[index].json_buf.push_str(&partial_json);
                            }
                            ContentDelta::ReasoningDelta { text } => {
                                // Forward reasoning content to TUI / per-call callback
                                if let Some(cb) = effective_cb {
                                    cb(
                                        session_id,
                                        ProgressEvent::ReasoningChunk { text: text.clone() },
                                    );
                                }
                                // Accumulate for persistence
                                reasoning_buf.push_str(&text);
                            }
                        }
                    }
                }
                StreamEvent::ContentBlockStop { index } => {
                    if index < block_states.len() {
                        let state = &mut block_states[index];
                        // Finalize tool use blocks: parse accumulated JSON
                        if let ContentBlock::ToolUse { ref mut input, .. } = state.block
                            && !state.json_buf.is_empty()
                            && let Ok(parsed) = serde_json::from_str(&state.json_buf)
                        {
                            *input = parsed;
                        }
                    }
                }
                StreamEvent::MessageDelta { delta, usage } => {
                    // Only update stop_reason if the delta carries one — deferred
                    // usage chunks send a second MessageDelta with stop_reason=None
                    // that must not overwrite the real stop_reason.
                    if delta.stop_reason.is_some() {
                        stop_reason = delta.stop_reason;
                    }
                    // Take the largest values — MiniMax sends two deltas:
                    // first (0,0), then the real usage. Other providers
                    // may only send one. Using max() handles both cases.
                    if usage.input_tokens > input_tokens {
                        input_tokens = usage.input_tokens;
                    }
                    if usage.output_tokens > output_tokens {
                        output_tokens = usage.output_tokens;
                    }
                }
                StreamEvent::MessageStop => break,
                StreamEvent::Ping => {}
                StreamEvent::Error { error } => {
                    return Err(crate::brain::provider::ProviderError::StreamError(error));
                }
            }
        }

        // Detect premature stream termination — if we accumulated blocks but never
        // got a stop_reason, the connection likely dropped before [DONE]/MessageStop.
        if stop_reason.is_none() && !block_states.is_empty() {
            tracing::warn!(
                "⚠️ Stream ended without MessageStop/[DONE]. {} content blocks accumulated, \
                 {} output tokens counted. Possible network interruption or provider timeout.",
                block_states.len(),
                output_tokens,
            );
        }

        // Build final content blocks from accumulated state
        // Filter out empty text blocks — Anthropic rejects "text content blocks must be non-empty"
        let content_blocks: Vec<ContentBlock> = block_states
            .into_iter()
            .map(|s| s.block)
            .filter(|b| !matches!(b, ContentBlock::Text { text } if text.is_empty()))
            .collect();

        let reasoning = if reasoning_buf.is_empty() {
            None
        } else {
            Some(reasoning_buf)
        };
        Ok((
            LLMResponse {
                id,
                // Some providers (e.g. MiniMax) don't include the model name in stream chunks.
                // Fall back to the request model so pricing lookup never gets an empty string.
                model: if model.is_empty() {
                    request_model
                } else {
                    model
                },
                content: content_blocks,
                stop_reason,
                usage: TokenUsage {
                    input_tokens,
                    output_tokens,
                },
            },
            reasoning,
        ))
    }

    /// Build a user Message, auto-attaching images from `<<IMG:path>>` markers.
    /// The TUI inserts these markers for detected image paths/URLs (handles spaces).
    pub(super) async fn build_user_message(text: &str) -> Message {
        let mut image_blocks: Vec<ContentBlock> = Vec::new();

        // Extract <<IMG:path>> markers
        let mut clean_text = text.to_string();
        while let Some(start) = clean_text.find("<<IMG:") {
            if let Some(end) = clean_text[start..].find(">>") {
                let marker_end = start + end + 2;
                let img_path = &clean_text[start + 6..start + end];

                // URL image
                if img_path.starts_with("http://") || img_path.starts_with("https://") {
                    image_blocks.push(ContentBlock::Image {
                        source: ImageSource::Url {
                            url: img_path.to_string(),
                        },
                    });
                    tracing::info!("Auto-attached image URL: {}", img_path);
                }
                // Local file
                else {
                    let path = std::path::Path::new(img_path);
                    if let Ok(data) = tokio::fs::read(path).await {
                        let lower = img_path.to_lowercase();
                        let media_type = match lower.rsplit('.').next().unwrap_or("") {
                            "png" => "image/png",
                            "jpg" | "jpeg" => "image/jpeg",
                            "gif" => "image/gif",
                            "webp" => "image/webp",
                            "bmp" => "image/bmp",
                            "svg" => "image/svg+xml",
                            _ => "application/octet-stream",
                        };
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                        image_blocks.push(ContentBlock::Image {
                            source: ImageSource::Base64 {
                                media_type: media_type.to_string(),
                                data: b64,
                            },
                        });
                        tracing::info!(
                            "Auto-attached image: {} ({}, {} bytes)",
                            img_path,
                            media_type,
                            data.len()
                        );
                    } else {
                        tracing::warn!("Could not read image file: {}", img_path);
                    }
                }

                // Remove marker from text
                clean_text = format!("{}{}", &clean_text[..start], &clean_text[marker_end..]);
            } else {
                break; // Malformed marker
            }
        }

        let clean_text = clean_text.trim().to_string();

        if image_blocks.is_empty() {
            Message::user(clean_text)
        } else {
            // Text first, then images
            let mut blocks = vec![ContentBlock::Text { text: clean_text }];
            blocks.extend(image_blocks);
            Message {
                role: Role::User,
                content: blocks,
            }
        }
    }

    /// Compact tool description for DB persistence (mirrors TUI's format_tool_description)
    pub(super) fn format_tool_summary(tool_name: &str, tool_input: &Value) -> String {
        match tool_name {
            "bash" => {
                let cmd = tool_input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let short: String = cmd.chars().take(60).collect();
                if cmd.len() > 60 {
                    format!("bash: {}…", short)
                } else {
                    format!("bash: {}", short)
                }
            }
            "read_file" | "read" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Read {}", path)
            }
            "write_file" | "write" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Write {}", path)
            }
            "edit_file" | "edit" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Edit {}", path)
            }
            "ls" => {
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                format!("ls {}", path)
            }
            "glob" => {
                let p = tool_input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Glob {}", p)
            }
            "grep" => {
                let p = tool_input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Grep '{}'", p)
            }
            "web_search" | "exa_search" | "brave_search" => {
                let q = tool_input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Search: {}", q)
            }
            "plan" => {
                let op = tool_input
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Plan: {}", op)
            }
            "task_manager" => {
                let op = tool_input
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Task: {}", op)
            }
            "memory_search" => {
                let q = tool_input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Memory: {}", q)
            }
            other => other.to_string(),
        }
    }

    /// Normalize hallucinated tool names from providers.
    ///
    /// Some models (e.g. MiniMax) send tool names like `"Plan: complete_task"`
    /// instead of `tool="plan"` with `operation="complete_task"` in the input.
    /// This recovers the intended call so it doesn't fail with "Tool not found".
    pub(super) fn normalize_tool_call(
        name: String,
        mut input: serde_json::Value,
    ) -> (String, serde_json::Value) {
        // "Plan: <op>" or "plan: <op>" → tool="plan", inject operation into input
        if let Some(op) = name
            .strip_prefix("Plan: ")
            .or_else(|| name.strip_prefix("plan: "))
            .or_else(|| name.strip_prefix("Plan:"))
            .or_else(|| name.strip_prefix("plan:"))
        {
            let op = op.trim().replace(' ', "_");
            if !op.is_empty() {
                if let Some(obj) = input.as_object_mut() {
                    obj.entry("operation")
                        .or_insert_with(|| serde_json::Value::String(op));
                }
                tracing::info!(
                    "[TOOL_NORM] Normalized '{}' → tool='plan', input={:?}",
                    name,
                    input
                );
                return ("plan".to_string(), input);
            }
        }

        // Generic fallback: if name contains ": " and isn't a registered tool,
        // try the part before ": " as the tool name (lowercased)
        if name.contains(": ") {
            let parts: Vec<&str> = name.splitn(2, ": ").collect();
            if parts.len() == 2 {
                let candidate = parts[0].to_lowercase().replace(' ', "_");
                let suffix = parts[1].trim().replace(' ', "_");
                if !suffix.is_empty() {
                    if let Some(obj) = input.as_object_mut() {
                        obj.entry("operation")
                            .or_insert_with(|| serde_json::Value::String(suffix));
                    }
                    tracing::info!(
                        "[TOOL_NORM] Normalized '{}' → tool='{}', input={:?}",
                        name,
                        candidate,
                        input
                    );
                    return (candidate, input);
                }
            }
        }

        (name, input)
    }

    /// Strip XML tool-call blocks from text so raw XML
    /// doesn't get persisted to DB or shown to the user.
    /// Catches `<tool_call>`, `<tool_code>`, `<StartToolCall>`, `<minimax:tool_call>`,
    /// `<tool_use>`, `<result>`, and any `<parameter>` blocks providers hallucinate.
    /// Check if text contains actual XML tool-call blocks (not just mentions).
    /// Requires BOTH opening AND closing tags to exist so that prose mentions
    /// like `` `<tool_use>` `` don't trigger false positives.
    pub(crate) fn has_xml_tool_block(text: &str) -> bool {
        (text.contains("<tool_call>") && text.contains("</tool_call>"))
            || (text.contains("<tool_code>") && text.contains("</tool_code>"))
            || (text.contains("<StartToolCall>") && text.contains("</StartToolCall>"))
            || (text.contains("<minimax:tool_call>") && text.contains("</minimax:tool_call>"))
            || (text.contains("<invoke") && text.contains("</invoke>"))
            || (text.contains("<tool_use>") && text.contains("</tool_use>"))
    }

    pub(crate) fn strip_xml_tool_calls(text: &str) -> String {
        use regex::Regex;
        use std::sync::LazyLock;

        // Match only properly closed XML tool-call blocks.
        // NO |$ fallback — unclosed tags (prose mentions) must NOT match.
        static TOOL_CALL_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"(?s)(<tool_call>.*?</tool_call>|<tool_code>.*?</tool_code>|<StartToolCall>.*?</StartToolCall>|<minimax:tool_call>.*?</minimax:tool_call>|<invoke\b.*?</invoke>|<param(?:eter)?\b[^>]*>.*?</param(?:eter)?>|<tool_use>.*?</tool_use>|<result>.*?</result>)"#).unwrap()
        });

        let result = TOOL_CALL_BLOCK_RE.replace_all(text, "");
        result.trim().to_string()
    }

    /// Strip ALL HTML comments from text.
    ///
    /// LLMs echo or hallucinate various HTML comment markers from context:
    /// `<!-- tools-v2: ... -->`, `<!-- lens -->`, `<!-- /tools-v2>`, etc.
    /// Rather than playing whack-a-mole with each pattern, strip everything
    /// between `<!--` and `-->` (or end of string for malformed tags).
    pub(crate) fn strip_html_comments(text: &str) -> String {
        use regex::Regex;
        use std::sync::LazyLock;

        // Match <!-- ... --> (proper) or <!-- ... (unclosed, to end of string)
        static HTML_COMMENT_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?s)<!--.*?(?:-->|$)"#).unwrap());

        let result = HTML_COMMENT_RE.replace_all(text, "");
        // Collapse any runs of 3+ newlines left by stripping
        let collapsed = result.lines().collect::<Vec<_>>().join("\n");
        let trimmed = collapsed.trim().to_string();
        // Collapse multiple blank lines
        use std::sync::LazyLock as LL;
        static MULTI_BLANK: LL<Regex> = LL::new(|| Regex::new(r"\n{3,}").unwrap());
        MULTI_BLANK.replace_all(&trimmed, "\n\n").to_string()
    }
}

/// Detect repetition in a streaming text window.
///
/// Returns `true` if a substring of `min_match` bytes from the second half
/// of `window` also appears in the first half, indicating the provider is
/// looping the same content.
pub fn detect_text_repetition(window: &str, min_match: usize) -> bool {
    if min_match == 0 || window.len() < min_match * 2 {
        return false;
    }
    // Find a valid char boundary at or after the midpoint
    let mut half = window.len() / 2;
    while !window.is_char_boundary(half) && half < window.len() {
        half += 1;
    }
    let second_half = &window[half..];
    let mut check_len = min_match.min(second_half.len());
    // Ensure check_len lands on a char boundary within second_half
    while !second_half.is_char_boundary(check_len) && check_len < second_half.len() {
        check_len += 1;
    }
    if let Some(needle) = second_half.get(..check_len) {
        window[..half].contains(needle)
    } else {
        false
    }
}
