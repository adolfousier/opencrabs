use super::builder::AgentService;
use super::types::{MessageQueueCallback, ProgressCallback, ProgressEvent};
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
    ///
    /// `queue_cb` + `queued_out`: CLI providers only. When a queued user message
    /// is consumed mid-stream at a tool boundary, it is written to `queued_out`
    /// so the caller can inject it into context after the stream ends.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn stream_complete(
        &self,
        session_id: Uuid,
        request: LLMRequest,
        cancel_token: Option<&CancellationToken>,
        override_cb: Option<&ProgressCallback>,
        queue_cb: Option<&MessageQueueCallback>,
        queued_out: Option<&tokio::sync::Mutex<Option<String>>>,
        suppress_callback: bool,
    ) -> std::result::Result<(LLMResponse, Option<String>), crate::brain::provider::ProviderError>
    {
        use crate::brain::provider::{ContentDelta, StreamEvent, TokenUsage};
        use futures::StreamExt;

        // suppress_callback=true skips all progress events (used during compaction
        // to prevent the compaction LLM response from leaking as visible TUI text).
        let effective_cb: Option<&ProgressCallback> = if suppress_callback {
            None
        } else {
            override_cb.or(self.progress_callback.as_ref())
        };

        let request_model = request.model.clone();
        let provider = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .clone();
        let mut stream = match provider.stream(request).await {
            Ok(s) => s,
            Err(e) => {
                crate::config::health::record_failure(provider.name(), &e.to_string());
                return Err(e);
            }
        };

        // Accumulate state from stream events
        let mut id = String::new();
        let mut model = String::new();
        let mut stop_reason: Option<StopReason> = None;
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;
        let mut cache_creation_tokens = 0u32;
        let mut cache_read_tokens = 0u32;
        let mut billing_cache_creation = 0u32;
        let mut billing_cache_read = 0u32;

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
        let is_cli = provider.cli_handles_tools();
        // CLI: track unflushed text so we can emit IntermediateText at tool
        // boundaries, giving the TUI real-time text→tools→text interleaving
        // during streaming instead of one massive wall after stream ends.
        let mut cli_unflushed_text = String::new();

        // Maximum idle time between SSE events before treating as a dropped connection.
        // NVIDIA/Kimi and some other providers occasionally hang silently without sending
        // [DONE] — this timeout lets the retry logic in tool_loop.rs recover instead of
        // blocking the TUI forever.
        //
        // CLI providers need a much longer timeout: they run tools internally
        // (cargo build, cargo test, gh commands) that can take several minutes
        // without producing any stream events. 60s is too short and causes
        // premature stream termination → retry → fresh CLI session that repeats
        // all prior work from scratch.
        let stream_idle_timeout = if is_cli {
            std::time::Duration::from_secs(3600) // 1 hour — CLI agents can run 30min+
        } else {
            std::time::Duration::from_secs(60)
        };

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
                result = tokio::time::timeout(stream_idle_timeout, stream.next()) => {
                    match result {
                        Ok(Some(item)) => item,
                        Ok(None) => break, // Stream ended normally
                        Err(_elapsed) => {
                            tracing::warn!(
                                "⏱️ Stream idle timeout after {}s — no event received from provider. \
                                 Treating as dropped stream (stop_reason=None → will retry).",
                                stream_idle_timeout.as_secs()
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
                    // Separate thinking blocks from different rounds with a blank line
                    if matches!(content_block, ContentBlock::Thinking { .. })
                        && !reasoning_buf.is_empty()
                    {
                        reasoning_buf.push_str("\n\n");
                        // Also emit separator to TUI so streaming display stays in sync
                        if let Some(cb) = effective_cb {
                            cb(
                                session_id,
                                ProgressEvent::ReasoningChunk {
                                    text: "\n\n".to_string(),
                                },
                            );
                        }
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
                                // CLI: track unflushed text for tool-boundary flushing
                                if is_cli {
                                    cli_unflushed_text.push_str(&text);
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
                                if let Some(cb) = effective_cb {
                                    cb(
                                        session_id,
                                        ProgressEvent::ReasoningChunk { text: text.clone() },
                                    );
                                }
                                // Always accumulate for DB persistence
                                reasoning_buf.push_str(&text);
                            }
                            ContentDelta::ThinkingDelta { thinking } => {
                                // Anthropic native thinking_delta — same as reasoning
                                if let Some(cb) = effective_cb {
                                    cb(
                                        session_id,
                                        ProgressEvent::ReasoningChunk {
                                            text: thinking.clone(),
                                        },
                                    );
                                }
                                reasoning_buf.push_str(&thinking);
                            }
                        }
                    }
                }
                StreamEvent::ContentBlockStop { index } => {
                    if index < block_states.len() {
                        // Finalize tool use blocks: parse accumulated JSON
                        {
                            let state = &mut block_states[index];
                            if let ContentBlock::ToolUse { ref mut input, .. } = state.block
                                && !state.json_buf.is_empty()
                                && let Ok(parsed) = serde_json::from_str(&state.json_buf)
                            {
                                *input = parsed;
                            }
                        }
                        // CLI: flush accumulated text as IntermediateText before
                        // emitting tool events, so TUI shows text→tools sequentially
                        // during streaming instead of one wall after stream ends.
                        // Also clear the text from prior text blocks so the final
                        // response.content only contains text emitted AFTER the
                        // last flush (preventing complete_response from
                        // overwriting the last intermediate msg with duplicate text).
                        let is_tool =
                            matches!(block_states[index].block, ContentBlock::ToolUse { .. });
                        if is_cli
                            && is_tool
                            && !cli_unflushed_text.is_empty()
                            && let Some(cb) = effective_cb
                        {
                            cb(
                                session_id,
                                ProgressEvent::IntermediateText {
                                    text: cli_unflushed_text.clone(),
                                    // None lets the TUI pull from its
                                    // accumulated streaming_reasoning
                                    reasoning: None,
                                },
                            );
                            cli_unflushed_text.clear();
                            for bs in block_states.iter_mut() {
                                if let ContentBlock::Text { text: ref mut t } = bs.block {
                                    t.clear();
                                }
                            }
                        }
                        // Emit ToolStarted + ToolCompleted with fully parsed input
                        // so the TUI shows real tool context (command, file path, etc.)
                        //
                        // CLI-ONLY: for CLI providers (claude-cli, qwen-cli, opencode),
                        // the CLI runs tools itself and the stream-close is the ONLY
                        // signal we get — so we synthesize the lifecycle here.
                        //
                        // For non-CLI providers (OpenAI-compatible, Anthropic, etc.)
                        // tool_loop owns the full lifecycle: it fires ToolStarted
                        // before invoking the tool and ToolCompleted with the real
                        // output. Firing here too would DOUBLE every event, bloat
                        // the tool-call count in the TUI, and leave a phantom
                        // "Processing: <tool>" indicator because the premature
                        // fake completion races the real one. See the 6-in-85µs
                        // duplication observed in logs.
                        if is_cli {
                            let state = &mut block_states[index];
                            if let ContentBlock::ToolUse {
                                ref name,
                                ref input,
                                ..
                            } = state.block
                                && let Some(cb) = effective_cb
                            {
                                let emit_name = name.to_lowercase();
                                cb(
                                    session_id,
                                    ProgressEvent::ToolStarted {
                                        tool_name: emit_name.clone(),
                                        tool_input: input.clone(),
                                    },
                                );
                                cb(
                                    session_id,
                                    ProgressEvent::ToolCompleted {
                                        tool_name: emit_name,
                                        tool_input: input.clone(),
                                        success: true,
                                        summary: String::new(),
                                    },
                                );

                                // CLI only: check if user queued a message during
                                // tool execution. Consume it and break the stream
                                // so tool_loop can inject it into context.
                                if let Some(qcb) = queue_cb
                                    && let Some(queued) = qcb().await
                                {
                                    tracing::info!(
                                        "Queued user message at CLI tool boundary — storing for tool_loop"
                                    );
                                    // Only store — don't emit QueuedUserMessage here.
                                    // tool_loop emits it AFTER CLI interleaving so it
                                    // appears in the correct position (after all tools).
                                    if let Some(buf) = queued_out {
                                        *buf.lock().await = Some(queued);
                                    }
                                    stop_reason = Some(StopReason::EndTurn);
                                    break;
                                }
                            }
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
                    // Per-call cache tokens (context window proxy)
                    if usage.cache_creation_tokens > cache_creation_tokens {
                        cache_creation_tokens = usage.cache_creation_tokens;
                    }
                    if usage.cache_read_tokens > cache_read_tokens {
                        cache_read_tokens = usage.cache_read_tokens;
                    }
                    // Billing cache tokens (cumulative across CLI rounds)
                    if usage.billing_cache_creation > billing_cache_creation {
                        billing_cache_creation = usage.billing_cache_creation;
                    }
                    if usage.billing_cache_read > billing_cache_read {
                        billing_cache_read = usage.billing_cache_read;
                    }
                }
                StreamEvent::MessageStop => break,
                StreamEvent::Ping => {
                    // CLI providers (opencode, claude-cli, qwen-cli) emit Ping
                    // at step_finish / tool_result boundaries. Use them as flush
                    // points so the TUI shows each step's tool calls + thinking
                    // live, instead of batching everything into one giant group
                    // at end-of-stream. Skip if there's nothing pending — empty
                    // flushes inflate the message count and waste vertical space.
                    if is_cli
                        && !cli_unflushed_text.is_empty()
                        && let Some(cb) = effective_cb
                    {
                        cb(
                            session_id,
                            ProgressEvent::IntermediateText {
                                text: std::mem::take(&mut cli_unflushed_text),
                                reasoning: None,
                            },
                        );
                        // Clear text from prior text blocks so the final
                        // response.content only contains text emitted AFTER
                        // this flush — prevents complete_response from
                        // overwriting the last intermediate msg with duplicate text.
                        for bs in block_states.iter_mut() {
                            if let ContentBlock::Text { text: ref mut t } = bs.block {
                                t.clear();
                            }
                        }
                    }
                }
                StreamEvent::Error { error } => {
                    crate::config::health::record_failure(provider.name(), &error);
                    return Err(crate::brain::provider::ProviderError::StreamError(error));
                }
            }
        }

        // CLI: flush any trailing text after the last tool
        if is_cli
            && !cli_unflushed_text.is_empty()
            && let Some(cb) = effective_cb
        {
            cb(
                session_id,
                ProgressEvent::IntermediateText {
                    text: cli_unflushed_text,
                    reasoning: None,
                },
            );
        }

        // Detect premature stream termination — if we accumulated blocks but never
        // got a stop_reason, the connection likely dropped before [DONE]/MessageStop.
        // Return a StreamError so the tool_loop's retry logic can re-issue the request
        // instead of silently returning partial/empty content to the user.
        if stop_reason.is_none() && !block_states.is_empty() {
            let msg = format!(
                "Stream ended without [DONE]: {} content blocks, {} output tokens — connection likely dropped",
                block_states.len(),
                output_tokens,
            );
            tracing::warn!("⚠️ {}", msg);
            return Err(crate::brain::provider::ProviderError::StreamError(msg));
        }

        // Self-heal: detect truncated responses disguised as complete.
        // Some providers (notably Qwen) occasionally send finish_reason="stop"
        // with a usage chunk after only a handful of tokens, producing a response
        // like "Let me check the current state:" — clearly mid-thought.  The
        // premature-termination guard above can't catch this because stop_reason
        // IS set (from the finish_reason chunk).  Detect it by checking: very low
        // output tokens + text that looks like an incomplete preamble (ends with
        // `:` or `...`).  Returning StreamError lets retry/rotation/fallback
        // re-issue the request instead of accepting garbage.
        if stop_reason == Some(StopReason::EndTurn) && output_tokens > 0 && output_tokens < 100 {
            let has_tool_use = block_states
                .iter()
                .any(|bs| matches!(&bs.block, ContentBlock::ToolUse { .. }));
            if !has_tool_use {
                let text: String = block_states
                    .iter()
                    .filter_map(|bs| match &bs.block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                let trimmed = text.trim();
                if trimmed.ends_with(':') || trimmed.ends_with("...") {
                    // Heuristic to reduce false positives: if the text contains
                    // multiple sentences (period, exclamation) it's more likely
                    // a legitimate short instruction than a truncated preamble.
                    // Real truncations are single preamble sentences like
                    // "Let me check the current state:" — no prior punctuation.
                    let has_prior_sentence = trimmed[..trimmed.len().saturating_sub(1)]
                        .contains('.')
                        || trimmed[..trimmed.len().saturating_sub(1)].contains('!');
                    if has_prior_sentence {
                        tracing::debug!(
                            "Self-heal: skipping truncation check — text contains \
                             prior sentences (likely deliberate short response)"
                        );
                    } else {
                        let preview = if trimmed.len() > 80 {
                            &trimmed[trimmed.len() - 80..]
                        } else {
                            trimmed
                        };
                        let msg = format!(
                            "Self-heal: provider sent stop after only {} output tokens — \
                             response appears truncated: \"{}\"",
                            output_tokens, preview,
                        );
                        tracing::warn!("⚠️ {}", msg);
                        if let Some(cb) = effective_cb {
                            cb(
                                session_id,
                                ProgressEvent::SelfHealingAlert {
                                    message: msg.clone(),
                                },
                            );
                        }
                        return Err(crate::brain::provider::ProviderError::StreamError(msg));
                    }
                }
            }
        }

        // Build final content blocks from accumulated state
        // Filter out empty text blocks — Anthropic rejects "text content blocks must be non-empty"
        let content_blocks: Vec<ContentBlock> = block_states
            .into_iter()
            .map(|s| s.block)
            .filter(|b| !matches!(b, ContentBlock::Text { text } if text.is_empty()))
            .collect();

        // Track provider health + snapshot config on first success.
        crate::config::health::record_success(provider.name());
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            static SAVED: AtomicBool = AtomicBool::new(false);
            if !SAVED.swap(true, Ordering::Relaxed) {
                crate::config::save_last_good_config();
            }
        }

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
                    cache_creation_tokens,
                    cache_read_tokens,
                    billing_cache_creation,
                    billing_cache_read,
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

                // Replace marker with a path hint so any model (vision or text-only) can
                // route the image through analyze_image / other vision tools. Vision-capable
                // providers also get the base64 image block below; text-only models rely on
                // this path to call analyze_image directly.
                let hint = format!("[image attached: {}]", img_path);
                clean_text = format!(
                    "{}{}{}",
                    &clean_text[..start],
                    hint,
                    &clean_text[marker_end..]
                );
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
                format!("bash: {}", cmd)
            }
            "read_file" | "read" => {
                let path = tool_input
                    .get("path")
                    .or_else(|| tool_input.get("file_path"))
                    .or_else(|| tool_input.get("filePath"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Read {}", path)
            }
            "write_file" | "write" => {
                let path = tool_input
                    .get("path")
                    .or_else(|| tool_input.get("file_path"))
                    .or_else(|| tool_input.get("filePath"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("Write {}", path)
            }
            "edit_file" | "edit" => {
                let path = tool_input
                    .get("path")
                    .or_else(|| tool_input.get("file_path"))
                    .or_else(|| tool_input.get("filePath"))
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
                let path = tool_input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if path.is_empty() {
                    format!("Grep '{}'", p)
                } else {
                    format!("Grep '{}' in {}", p, path)
                }
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

        // Claude Code tool name mapping (capitalized → OpenCrabs lowercase)
        // The cc-max-proxy returns Claude Code tool names which differ from ours.
        let mapped = match name.as_str() {
            "Bash" => Some("bash"),
            "Read" => Some("read_file"),
            "Write" => Some("write_file"),
            "Edit" => Some("edit_file"),
            "Glob" => Some("glob"),
            "Grep" => Some("grep"),
            "WebSearch" => Some("web_search"),
            "WebFetch" => Some("http_request"),
            "NotebookEdit" => Some("notebook_edit"),
            _ => None,
        };
        if let Some(canonical) = mapped {
            tracing::info!(
                "[TOOL_NORM] Mapped Claude Code tool '{}' → '{}'",
                name,
                canonical
            );
            return (canonical.to_string(), input);
        }

        // Final fallback: lowercase the name (catches simple case mismatches)
        let lowered = name.to_lowercase();
        if lowered != name {
            tracing::info!("[TOOL_NORM] Lowercased tool '{}' → '{}'", name, lowered);
            return (lowered, input);
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

    /// Parse XML tool-call blocks into (name, input) pairs.
    /// Handles multiple formats MiniMax uses:
    ///   <tool_call>{"tool_name":"bash","args":{"command":"..."}}</tool_call>
    ///   <tool_call>{"name":"bash","arguments":{"command":"..."}}</tool_call>
    ///   <tool_use>{"name":"bash","input":{"command":"..."}}</tool_use>
    pub(crate) fn parse_xml_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
        use regex::Regex;
        use std::sync::LazyLock;

        static XML_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"(?s)<(?:tool_call|tool_code|tool_use|minimax:tool_call|StartToolCall)>(.*?)</(?:tool_call|tool_code|tool_use|minimax:tool_call|StartToolCall)>"#).unwrap()
        });

        let mut results = Vec::new();
        for cap in XML_BLOCK_RE.captures_iter(text) {
            let inner = cap[1].trim();
            // Try parsing as JSON
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(inner) {
                // Extract tool name from various field names
                let name = obj
                    .get("tool_name")
                    .or_else(|| obj.get("name"))
                    .or_else(|| obj.get("function"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if name.is_empty() {
                    continue;
                }

                // Extract input/arguments from various field names
                let input = obj
                    .get("args")
                    .or_else(|| obj.get("arguments"))
                    .or_else(|| obj.get("input"))
                    .or_else(|| obj.get("parameters"))
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

                tracing::info!(
                    "[XML_TOOL_PARSE] Recovered tool call: name={}, input_keys={:?}",
                    name,
                    input.as_object().map(|o| o.keys().collect::<Vec<_>>())
                );
                results.push((name, input));
            }
        }
        results
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

        // Match only properly closed <!-- ... --> comments.
        // Do NOT match unclosed comments — stripping to end-of-string would
        // silently delete trailing response text mid-stream.
        static HTML_COMMENT_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?s)<!--.*?-->"#).unwrap());

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

/// Refusal/gaslighting phrases harvested from real dialagram qwen-thinking
/// streams (see logs 2026-04-08). Every phrase was observed in an assistant
/// turn that ALSO contained a valid `tool_use` block that executed
/// successfully — i.e. the model claimed tools were broken while
/// simultaneously calling them. These substrings are matched
/// case-insensitively.
const GASLIGHTING_REFUSAL_PHRASES: &[&str] = &[
    // "tools are broken" family
    "tools aren't responding",
    "tools are not responding",
    "tools are flaky",
    "tools are still flaky",
    "tools appear to be",
    "tools appear broken",
    "tools appear unavailable",
    "appear to be unavailable",
    "tools are unavailable",
    "tools are currently unavailable",
    "tools are disabled",
    "tools are not loading",
    "tools are not available",
    // "not currently available" family (23:58 incident — vision tool variant)
    "isn't currently available",
    "is not currently available",
    "not currently available",
    "tool isn't currently",
    "tool is not currently",
    "vision tool isn't",
    "vision tool is not",
    "vision integration",
    "despite being in my tool list",
    "despite being in the tool list",
    "despite appearing in",
    "even though it appears in",
    // "not registered" family
    "isn't actually registered",
    "is not actually registered",
    "not actually registered",
    "isn't registered",
    "isn't loaded",
    "is not loaded",
    "isn't in the registry",
    "not in the registry",
    // "runtime mismatch" family
    "mismatch between the advertised",
    "advertised capabilities",
    "runtime hiccup",
    "might be a runtime",
    "might be a configuration issue",
    "configuration issue",
    "runtime issue",
    "runtime glitch",
    "underlying system disruption",
    "system disruption",
    "provider glitch",
    "provider hiccup",
    // "can't execute / unable to" family
    "can't execute the tool",
    "cannot execute the tool",
    "unable to execute the tool",
    "unable to invoke",
    "unable to call the tool",
    "unable to retrieve",
    "unable to analyze",
    "unable to analyse",
    "unable to process",
    "unable to view",
    "unable to see",
    "unable to read",
    "tool execution failed before it started",
    // "user workaround" family — when the model asks the user to manually
    // re-upload or describe content that the tool WOULD handle. These are
    // pure gaslighting preambles emitted alongside the real tool_use call.
    "try uploading it again",
    "try uploading the image again",
    "upload it again",
    "upload the image again",
    "just tell me what's in",
    "just describe what's in",
    "or just tell me",
    "or just describe",
    "paste it as",
    "paste the image",
    "drop the path",
    "if you need image analysis",
    "for image analysis you could",
    // "no access / not in my environment" family (00:30 incident)
    "don't have access to a working",
    "do not have access to a working",
    "don't have a working",
    "tool isn't available in my",
    "tool is not available in my",
    "isn't available in my current environment",
    "not available in my current environment",
    "in my current environment",
    "working image analysis tool",
    "image analysis tool for local files",
    "upload the screenshot to a public",
    "upload the image to a public",
    "try to analyze it via url",
    "analyze it via a url",
    "analyze it via url",
    "public url (imgur",
    "(imgur, github",
    // "sandbox / tools acting up" family (2026-04-09 incident — model
    // woke up on first round convinced it was in a sandboxed container
    // and the tool layer was broken, while drafting the full answer
    // right after the preamble).
    "tools are acting up",
    "tools are acting weird",
    "tool layer is acting",
    "does not exists errors",
    "does not exist errors",
    "errors across the board",
    "getting errors across",
    "getting \"does not exist",
    "getting 'does not exist",
    "tools seem to be down",
    "tools seem broken",
    "tool system is down",
    "running in a sandbox",
    "running in a sandboxed",
    "sandboxed environment",
    "sandboxed container",
    "docker container with no",
    "no tool access in",
    // "still glitching / session weirdness" family (2026-04-09 second
    // incident — model blamed a previous model-switch for tool failures
    // while drafting the real answer in the same block).
    "tools are still glitching",
    "tools still glitching",
    "tools are glitching",
    "tool layer is glitching",
    "session state got weird",
    "session state is weird",
    "from the model switch earlier",
    "from the earlier model switch",
    "from the earlier session state",
    "turbulence from rapid model",
    "turbulence from the model",
    "some turbulence from",
    "session had some turbulence",
    "tools temporarily failing",
    "temporarily failing due to",
    "session state issues from model",
    "had the issue context from the earlier fetch",
    "have the issue context from the earlier",
    "from the earlier fetch, so let me break",
    "so let me break this down directly",
    // "tool registry / completely offline" family (2026-04-09 third
    // incident — model insists tools are "completely offline" and the
    // tool registry failed to load post-restart, while drafting the real
    // answer right after. Only phrases unique to the gaslighting script.
    "tools are completely offline",
    "tools completely offline",
    "every call returns \"does not exists\"",
    "every call returns 'does not exists'",
    "every call returns \"does not exist\"",
    "this isn't just session state",
    "this is not just session state",
    "tool registry itself isn't loading",
    "tool registry itself is not loading",
    "tool registry isn't loading post-restart",
    "tool registry is not loading post-restart",
    "isn't loading post-restart",
    "not loading post-restart",
    "ping me when you're back in",
    "ping me when you are back in",
    "once tools are back",
];

/// Detect a gaslighting refusal preamble: short assistant text that lies
/// about tool/capability availability for images.
///
/// Two independent signals, either one is sufficient:
///
/// 1. **Exact phrase match** against `GASLIGHTING_REFUSAL_PHRASES` —
///    catches canned preambles from known provider quirks.
///
/// 2. **First-person refusal opening + image context** — text must BEGIN
///    with a first-person refusal ("I can't", "I don't have", "I'm
///    unable", etc.) AND mention image/screenshot/vision context.
///
///    This shape is near-zero false positive because legit responses
///    describing an image start with "It's a...", "The screenshot
///    shows...", "This image contains..." — never "I can't see...".
///
/// Length guard: > 1500 chars is almost always legit long narration.
pub fn is_gaslighting_preamble(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 1500 {
        return false;
    }
    let lower = trimmed.to_lowercase();

    // Signal 1: exact phrase list
    if GASLIGHTING_REFUSAL_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
    {
        return true;
    }

    // Signal 2: refusal opening + image context
    const REFUSAL_OPENINGS: &[&str] = &[
        "i can't",
        "i cannot",
        "i can not",
        "i don't have",
        "i do not have",
        "i'm unable",
        "i am unable",
        "i'm not able",
        "i am not able",
        "i lack ",
        "unfortunately, i can't",
        "unfortunately i can't",
        "unfortunately, i cannot",
        "unfortunately i cannot",
        "unfortunately, i don't",
        "unfortunately i don't",
        "sorry, i can't",
        "sorry i can't",
        "sorry, i cannot",
        "sorry i cannot",
    ];
    let starts_with_refusal = REFUSAL_OPENINGS.iter().any(|o| lower.starts_with(o));
    if !starts_with_refusal {
        return false;
    }

    // Tight image/vision context — deliberately NO generic "tool"/"file"
    // because legit responses ("I can't find the file you mentioned")
    // would false-positive.
    const IMAGE_CONTEXT: &[&str] = &[
        "image",
        "images",
        "screenshot",
        "photo",
        "picture",
        "vision",
        "visual",
        "analyze_image",
        "analyse_image",
    ];
    IMAGE_CONTEXT.iter().any(|w| lower.contains(w))
}

/// Strip leading gaslighting paragraphs from a text block.
///
/// Splits `text` on blank lines and drops any LEADING paragraphs that
/// match `is_gaslighting_preamble`, stopping at the first non-matching
/// paragraph. Returns `Some(stripped_text)` if anything was removed, or
/// `None` if the block is clean.
///
/// This exists because the model often emits ONE text block containing
/// a gaslighting opener ("Tools are acting up right now…") followed by
/// a full legitimate implementation draft. The old full-block strip
/// either dropped the entire block (nuking the draft) or gave up
/// because the block exceeded the 1500-char length guard used by
/// `is_gaslighting_preamble`.
pub fn strip_gaslighting_preamble(text: &str) -> Option<String> {
    // Split on blank lines (paragraph boundaries). Use split_terminator
    // so we preserve the trailing empty string semantics when needed.
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    if paragraphs.is_empty() {
        return None;
    }

    let mut first_kept = 0usize;
    for (idx, p) in paragraphs.iter().enumerate() {
        if is_gaslighting_preamble(p) {
            first_kept = idx + 1;
        } else {
            break;
        }
    }

    if first_kept == 0 {
        return None;
    }

    let remainder = paragraphs[first_kept..].join("\n\n");
    Some(remainder.trim_start().to_string())
}

/// Detect "phantom tool calls" — the model narrates actions it claims to
/// have performed but never actually executed any tool calls.
///
/// Returns `true` when the response text contains strong action-intent
/// signals (modification verbs + file-path-like strings) suggesting the
/// model believed it was making changes. The caller should inject a retry
/// prompt so the model actually executes the tool calls on the next turn.
///
/// Deliberately conservative: requires BOTH an action verb AND a file
/// path pattern to avoid false-positives on conversational responses.
/// Shared intent phrases used by both the strict and relaxed phantom
/// detectors. Action verbs + read/inspection verbs + "I'll proceed"
/// variants. Lowered-cased match.
const INTENT_PHRASES: &[&str] = &[
    "now let me ",
    "now update ",
    "now fix ",
    "now add ",
    "now bump ",
    "now run ",
    "now check ",
    "now read ",
    "now commit",
    "now amend",
    "i'll update",
    "i'll fix",
    "i'll modify",
    "i'll create",
    "i'll write",
    "i'll edit",
    "i'll add",
    "i'll change",
    "i'll replace",
    "i'll commit",
    "i'll amend",
    "i'll proceed",
    "i'll start",
    "i'll finish",
    "i'll run",
    "i'll check",
    "i'll see",
    "i'll look",
    "i'll prepare",
    "i'll take a look",
    "i will proceed",
    "let me update",
    "let me fix",
    "let me modify",
    "let me create",
    "let me write",
    "let me edit",
    "let me add",
    "let me change",
    "let me commit",
    "let me amend",
    "let me see",
    "let me check",
    "let me look",
    "let me read",
    "let me examine",
    "let me verify",
    "let me inspect",
    "let me review",
    "let me take",     // "let me take a look"
    "let me actually", // "let me actually look at the commits"
    "let me prepare",
    "let me proceed",
    "let me start",
    "let me first", // "let me first see where we stand"
    "let me finish",
    "let me finalize",
    "let me run",
];

/// Relaxed phantom detection used when the caller already knows the
/// model emitted **zero tool_use blocks** this iteration. In that case
/// any bare intent phrase is phantom — no path or extension
/// corroboration required, because the tool count already proves
/// nothing happened.
pub fn has_phantom_tool_intent_no_tools(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 20 {
        return false;
    }
    let lower = trimmed.to_lowercase();
    INTENT_PHRASES.iter().any(|p| lower.contains(p))
}

pub fn has_phantom_tool_intent(text: &str) -> bool {
    let trimmed = text.trim();
    // Short responses are usually direct answers, not phantom narrations
    if trimmed.len() < 40 {
        return false;
    }
    let lower = trimmed.to_lowercase();

    // ── Strong signals (standalone — no corroboration needed) ─────────

    use regex::Regex;

    // 2+ imperative "Now <verb>" / "Let me <verb>" at line start = multi-step plan
    let now_imperative =
        Regex::new(r"(?m)^[\s\-*]*(?:now\s+(?:let\s+me\s+)?|let\s+me\s+)\w").unwrap();
    if now_imperative.find_iter(&lower).count() >= 2 {
        return true;
    }

    // 2+ numbered steps with action verbs = narrated plan
    let numbered_steps =
        Regex::new(r"(?m)^\s*\d+\.\s+(?:update|fix|modify|create|write|edit|add|change|remove|delete|check|read|run|bump|amend|verify|test|deploy|install)")
            .unwrap();
    if numbered_steps.find_iter(&lower).count() >= 2 {
        return true;
    }

    // 2+ past-tense standalone sentences = phantom completion narration
    let past_tense_standalone = Regex::new(
        r"(?m)^[\s\-*]*(?:amended|updated|fixed|modified|created|written|saved|deleted|removed|replaced|bumped|deployed|committed)[.!]"
    ).unwrap();
    if past_tense_standalone.find_iter(&lower).count() >= 2 {
        return true;
    }

    // ── Completion claims (standalone — model claims it finished work) ─
    // These are strong because a text-only response saying "I've updated
    // the file" with zero tool calls is always phantom.
    const COMPLETION_CLAIMS: &[&str] = &[
        "here's what changed",
        "here's what's changed",
        "here are the changes",
        "here's what i did",
        "here is what i did",
        "changes applied",
        "updated the file",
        "updated the code",
        "updated src/",
        "modified the file",
        "modified src/",
        "fixed the file",
        "fixed the bug",
        "fixed the issue",
        "fixed src/",
        "created the file",
        "wrote the file",
        "everything is updated",
        "i've made the changes",
        "i've completed",
        "i've finished",
        "i've updated",
        "i've written",
        "i've created",
        "i've saved",
        "i've modified",
        "i've fixed",
        "i've replaced",
        "i've amended",
        "i've committed",
        "i've bumped",
        "i've made all",
        "all changes have been",
        "all files have been",
        "the changes have been applied",
        "changes are now in place",
        "the file now contains",
        "the file has been",
        "file updated",
        "file created",
        "file saved",
        "changes saved",
        // Git-specific phantom claims
        "amended.",
        "committed.",
        "amended the commit",
        "bumped the version",
        "version bumped",
    ];
    if COMPLETION_CLAIMS.iter().any(|c| lower.contains(c)) {
        return true;
    }

    // ── Weak signals (need corroboration) ─────────────────────────────
    // A single "let me check" or "I'll look" is normal conversation.
    // Only flag as phantom if ALSO accompanied by file-path-like patterns,
    // meaning the model is narrating specific file operations it should
    // be executing via tools.

    let has_intent = INTENT_PHRASES.iter().any(|v| lower.contains(v));

    // Trailing-colon "Let me X:" at end of response is a strong signal all
    // on its own — the model set up an action then emitted nothing after.
    // No path corroboration needed: the colon announces a follow-up that
    // never came.
    let trailing_colon_intent = Regex::new(
        r"(?im)(?:^|\n)\s*(?:let\s+me|i'll|i\s+will|now\s+let\s+me|now\s+i'll)\s+\w[^:\n]{0,80}:\s*$",
    )
    .unwrap();
    if trailing_colon_intent.is_match(trimmed) {
        return true;
    }

    if has_intent {
        // Corroborate: does the text reference file paths or code identifiers?
        // e.g. src/foo/bar.rs, ./config.toml, Cargo.toml, `some_function`
        let path_re =
            Regex::new(r"(?:^|[\s`(])(?:\./)?[a-zA-Z_][\w\-]*/[\w\-/]*\.\w{1,6}(?:[\s`),:;]|$)")
                .unwrap();
        let ext_re = Regex::new(
            r"(?:^|[\s`(])[\w\-]+\.(?:rs|py|ts|tsx|js|jsx|go|sh|toml|yaml|yml|json|md)(?:[\s`),:;]|$)",
        )
        .unwrap();
        // Backtick code references like `auth_invalidate_fn` or `MyStruct`
        let backtick_code_re = Regex::new(r"`[a-zA-Z_]\w+`").unwrap();
        if path_re.is_match(trimmed)
            || ext_re.is_match(trimmed)
            || backtick_code_re.is_match(trimmed)
        {
            return true;
        }
    }

    false
}
