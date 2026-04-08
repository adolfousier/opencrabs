//! Custom OpenAI-Compatible Provider Implementation
//!
//! Implements the Provider trait for any OpenAI-compatible API, including:
//! - Official OpenAI (GPT-4, GPT-3.5, etc.)
//! - OpenRouter (100+ models)
//! - Minimax
//! - Local LLMs via LM Studio, Ollama, LocalAI
//! - Any endpoint that speaks the OpenAI chat completions protocol

use super::error::{ProviderError, Result};
use super::rate_limiter::RateLimiter;
use super::r#trait::{Provider, ProviderStream};
use super::types::*;
use crate::brain::tokenizer::{count_message_tokens, count_tokens};
use async_trait::async_trait;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
const DEFAULT_OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Open/close tag pairs to strip from streaming/non-streaming content.
/// Covers DeepSeek-style `<think>` and Kimi-style `<!-- reasoning -->` blocks.
/// The generic `<!--` entry catches ALL HTML comments (tools-v2, lens, /tools-v2,
/// and any future hallucinated markers) so they never reach the TUI during streaming.
/// Each entry in STRIP_CLOSE_TAGS is a list of accepted close tags (first match wins).
/// MiniMax closes `<!-- reasoning -->` with `</think>` instead of `<!-- /reasoning -->`.
/// Order matters: more specific patterns must come before the generic `<!--` catch-all.
/// NOTE: Only reasoning/markup blocks belong here — NOT XML tool-call tags.
/// Tool-call XML (`<tool_call>`, `<tool_use>`, `<result>`, etc.) must NOT be
/// stripped during streaming because the model may MENTION these tags in prose
/// (e.g. "strip `<result>` tags"). Stripping them here eats the rest of the
/// response when no closing tag arrives in the same chunk. Tool-call XML is
/// handled post-response in tool_loop.rs where the full text is available.
const STRIP_OPEN_TAGS: &[&str] = &["<think>", "<!-- reasoning -->", "<!--"];
const STRIP_CLOSE_TAGS: &[&[&str]] = &[
    &["</think>"],
    &["<!-- /reasoning -->", "</think>", "-->"], // Kimi uses <!-- /reasoning -->, MiniMax uses </think>, --> catches split-chunk close tags
    &["-->"],
];

/// Filter reasoning/markup blocks from a streaming chunk.
///
/// Tracks state across chunks via `inside_think`. Returns the portion of `text`
/// that is outside any stripped block. Handles tags split across chunk boundaries.
/// Maximum bytes to consume inside a `<!-- ... -->` block before assuming the
/// closing tag will never arrive.  When exceeded we abandon filtering and pass
/// content through — the model likely hallucinated an open tag (e.g.
/// `<!-- tools-v2:`) without ever sending `-->`. Kept small (v0.2.99 value) so
/// the filter never holds large amounts of API streaming text hostage waiting
/// for a close tag that is not coming.
const THINK_BLOCK_MAX_BYTES: usize = 400;

fn filter_think_tags(
    text: &str,
    inside_think: &mut bool,
    active_close_tag: &mut usize,
    bytes_consumed: &mut usize,
) -> String {
    let mut result = String::new();
    let mut remaining = text;

    loop {
        if *inside_think {
            // Safety valve: if we've consumed too many bytes without finding the
            // closing tag, the model probably hallucinated an unclosed open tag
            // (e.g. `<!-- tools-v2: ...` with no `-->`).
            // Discard the accumulated garbage (it's inside an HTML comment) and
            // stop filtering — future chunks will pass through normally.
            *bytes_consumed += remaining.len();
            if *bytes_consumed > THINK_BLOCK_MAX_BYTES {
                tracing::warn!(
                    "⚠️ Abandoned think-tag filter after {} bytes without close tag \
                     (tag_idx={}) — discarding buffered content, future chunks pass through",
                    *bytes_consumed,
                    *active_close_tag,
                );
                // Don't push remaining — it's inside the unclosed comment block.
                // Just exit the think state so subsequent chunks pass through.
                *inside_think = false;
                *bytes_consumed = 0;
                break;
            }

            // Find the earliest matching close tag among the candidates for this block.
            let close_candidates = STRIP_CLOSE_TAGS[*active_close_tag];
            let earliest_close = close_candidates
                .iter()
                .filter_map(|close| remaining.find(close).map(|pos| (pos, *close)))
                .min_by_key(|(pos, _)| *pos);

            if let Some((end, close)) = earliest_close {
                remaining = &remaining[end + close.len()..];
                *inside_think = false;
                *bytes_consumed = 0;
            } else {
                break;
            }
        } else {
            // Find the earliest open tag
            let mut earliest: Option<(usize, usize)> = None; // (position, tag_index)
            for (i, open) in STRIP_OPEN_TAGS.iter().enumerate() {
                if let Some(pos) = remaining.find(open)
                    && earliest.is_none_or(|(best, _)| pos < best)
                {
                    earliest = Some((pos, i));
                }
            }

            if let Some((pos, tag_idx)) = earliest {
                result.push_str(&remaining[..pos]);
                remaining = &remaining[pos + STRIP_OPEN_TAGS[tag_idx].len()..];
                *inside_think = true;
                *active_close_tag = tag_idx;
                *bytes_consumed = 0;
            } else {
                result.push_str(remaining);
                break;
            }
        }
    }

    result
}

/// Strip complete reasoning/markup blocks from non-streaming content.
fn strip_think_blocks(text: &str) -> String {
    let mut result = text.to_string();
    for (open, close_candidates) in STRIP_OPEN_TAGS.iter().zip(STRIP_CLOSE_TAGS.iter()) {
        while let Some(start) = result.find(open) {
            // Find the earliest close tag among the candidates.
            let earliest_close = close_candidates
                .iter()
                .filter_map(|close| result[start..].find(close).map(|end| (end, *close)))
                .min_by_key(|(end, _)| *end);

            if let Some((end, close)) = earliest_close {
                result = format!(
                    "{}{}",
                    &result[..start],
                    &result[start + end + close.len()..]
                );
            } else {
                // Unclosed tag — strip from open tag to end
                result = result[..start].to_string();
                break;
            }
        }
    }
    result.trim().to_string()
}

/// Dynamic token provider — called on every request to get the current bearer token.
/// Used by Copilot provider where the token rotates every ~30 minutes.
pub type TokenFn = Arc<dyn Fn() -> String + Send + Sync>;

/// OpenAI provider for GPT models
#[derive(Clone)]
pub struct OpenAIProvider {
    api_key: String,
    base_url: String,
    client: Client,
    custom_default_model: Option<String>,
    name: String,
    /// When set, swap to this model for requests containing images.
    vision_model: Option<String>,
    /// Extra headers injected into every request (e.g. GitHub Copilot API versioning).
    pub(crate) extra_headers: Vec<(String, String)>,
    /// Configured context window size (overrides model-name heuristics).
    configured_context_window: Option<u32>,
    /// Optional dynamic token provider (overrides api_key when set).
    token_fn: Option<TokenFn>,
    /// Proactive rate limiter — shared via Arc so all clones throttle together.
    /// Used for OpenRouter `:free` models (~3s between requests).
    rate_limiter: Option<Arc<RateLimiter>>,
}

impl OpenAIProvider {
    /// Returns true if this provider targets OpenRouter (detected by base_url).
    fn is_openrouter(&self) -> bool {
        self.base_url.to_lowercase().contains("openrouter")
    }

    /// Create a new OpenAI provider with official API
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .pool_idle_timeout(DEFAULT_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(2)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key,
            base_url: DEFAULT_OPENAI_API_URL.to_string(),
            client,
            custom_default_model: None,
            name: "openai".to_string(),
            vision_model: None,
            extra_headers: vec![],
            configured_context_window: None,
            token_fn: None,
            rate_limiter: None,
        }
    }

    /// Create provider for local LLM (LM Studio, Ollama, etc.)
    pub fn local(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .pool_idle_timeout(DEFAULT_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(2)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key: "not-needed".to_string(),
            base_url,
            client,
            custom_default_model: None,
            name: "openai-compatible".to_string(),
            vision_model: None,
            extra_headers: vec![],
            configured_context_window: None,
            token_fn: None,
            rate_limiter: None,
        }
    }

    /// Create with custom base URL
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .pool_idle_timeout(DEFAULT_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(2)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key,
            base_url,
            client,
            custom_default_model: None,
            name: "openai-compatible".to_string(),
            vision_model: None,
            extra_headers: vec![],
            configured_context_window: None,
            token_fn: None,
            rate_limiter: None,
        }
    }

    /// Add extra headers to every request (e.g. API versioning).
    pub fn with_extra_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }

    /// Set a configured context window size that overrides model-name heuristics.
    pub fn with_context_window(mut self, size: u32) -> Self {
        self.configured_context_window = Some(size);
        self
    }

    /// Set provider name (for logging)
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Set custom default model (useful for local LLMs with specific model names)
    pub fn with_default_model(mut self, model: String) -> Self {
        self.custom_default_model = Some(model);
        self
    }

    /// Set a dynamic token provider (overrides static api_key in headers).
    /// Used by Copilot where the bearer token rotates every ~30 minutes.
    pub fn with_token_fn(mut self, f: TokenFn) -> Self {
        self.token_fn = Some(f);
        self
    }

    /// Set vision model — used by the `analyze_image` tool as a provider-native
    /// vision backend when Gemini vision isn't configured.
    pub fn with_vision_model(mut self, model: String) -> Self {
        self.vision_model = Some(model);
        self
    }

    /// Set a proactive rate limiter — enforces minimum interval between API
    /// calls to stay under provider rate limits (e.g. OpenRouter :free at 20/min).
    /// Takes an `Arc<RateLimiter>` so multiple provider instances share ONE limiter.
    pub fn with_rate_limiter(mut self, limiter: Arc<RateLimiter>) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    /// Get the configured vision model name (if any).
    pub fn vision_model(&self) -> Option<&str> {
        self.vision_model.as_deref()
    }

    /// Build request headers
    fn headers(&self) -> std::result::Result<reqwest::header::HeaderMap, ProviderError> {
        let mut headers = reqwest::header::HeaderMap::new();

        // Resolve the bearer token: dynamic token_fn takes priority over static api_key
        let bearer_key = if let Some(ref f) = self.token_fn {
            let token = f();
            if token.is_empty() { None } else { Some(token) }
        } else if self.api_key != "not-needed" {
            Some(self.api_key.trim().to_string())
        } else {
            None
        };

        if let Some(key) = bearer_key {
            let header_value: reqwest::header::HeaderValue =
                format!("Bearer {}", key).parse().map_err(|_| {
                    tracing::error!(
                        "API key contains invalid characters (length={}). Check keys.toml.",
                        key.len()
                    );
                    ProviderError::InvalidApiKey
                })?;
            headers.insert(reqwest::header::AUTHORIZATION, header_value);
        }

        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().expect("valid content-type"),
        );

        // OpenRouter-specific optimization headers
        if self.base_url.to_lowercase().contains("openrouter") {
            if let (Ok(k1), Ok(v1)) = (
                "HTTP-Referer".parse::<reqwest::header::HeaderName>(),
                "https://opencrabs.com".parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(k1, v1);
            }
            if let (Ok(k2), Ok(v2)) = (
                "X-Title".parse::<reqwest::header::HeaderName>(),
                "OpenCrabs".parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(k2, v2);
            }
            if let (Ok(k3), Ok(v3)) = (
                "X-OpenRouter-Category".parse::<reqwest::header::HeaderName>(),
                "cli-agent,personal-agent,programming-app".parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(k3, v3);
            }
            tracing::debug!("OpenRouter optimization headers attached");
        }

        for (key, value) in &self.extra_headers {
            if let (Ok(k), Ok(v)) = (
                key.parse::<reqwest::header::HeaderName>(),
                value.parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(k, v);
            }
        }

        Ok(headers)
    }

    /// Convert our generic request to OpenAI-specific format
    fn to_openai_request(&self, request: LLMRequest) -> OpenAIRequest {
        let mut messages = Vec::new();

        // Debug: log system brain
        if let Some(ref system) = request.system {
            tracing::debug!("System brain present: {} chars", system.len());
        } else {
            tracing::warn!("NO SYSTEM BRAIN in request!");
        }

        // Add system message if present
        if let Some(system) = request.system {
            messages.push(OpenAIMessage {
                role: "system".to_string(),
                content: Some(serde_json::Value::String(system)),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add conversation messages
        for msg in request.messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
            };

            // Separate content blocks by type
            let mut text_parts = Vec::new();
            let mut image_parts: Vec<serde_json::Value> = Vec::new();
            let mut tool_uses = Vec::new();
            let mut tool_results = Vec::new();

            for block in msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        text_parts.push(text);
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_uses.push((id, name, input));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        tool_results.push((tool_use_id, content));
                    }
                    ContentBlock::Thinking { .. } => {
                        // OpenAI-compatible providers don't support thinking blocks; skip.
                    }
                    ContentBlock::Image { source } => {
                        let url = match source {
                            ImageSource::Base64 { media_type, data } => {
                                format!("data:{};base64,{}", media_type, data)
                            }
                            ImageSource::Url { url } => url,
                        };
                        image_parts.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": { "url": url }
                        }));
                    }
                }
            }

            // Build content value: array when images present, string otherwise
            let make_content =
                |texts: &[String], images: &[serde_json::Value]| -> Option<serde_json::Value> {
                    if !images.is_empty() {
                        let mut parts: Vec<serde_json::Value> = Vec::new();
                        if !texts.is_empty() {
                            parts.push(serde_json::json!({
                                "type": "text",
                                "text": texts.join("\n")
                            }));
                        }
                        parts.extend(images.iter().cloned());
                        Some(serde_json::Value::Array(parts))
                    } else if !texts.is_empty() {
                        Some(serde_json::Value::String(texts.join("\n")))
                    } else {
                        None
                    }
                };

            // Handle assistant messages with tool calls
            if !tool_uses.is_empty() {
                let openai_tool_calls = tool_uses
                    .into_iter()
                    .map(|(id, name, input)| OpenAIToolCall {
                        id,
                        r#type: "function".to_string(),
                        function: OpenAIFunctionCall {
                            name,
                            arguments: serde_json::to_string(&input).unwrap_or_default(),
                        },
                    })
                    .collect();

                let content_val = make_content(&text_parts, &image_parts);

                messages.push(OpenAIMessage {
                    role: role.to_string(),
                    content: content_val,
                    tool_calls: Some(openai_tool_calls),
                    tool_call_id: None,
                });
            }
            // Handle tool result messages
            else if !tool_results.is_empty() {
                for (tool_use_id, content) in tool_results {
                    messages.push(OpenAIMessage {
                        role: "tool".to_string(),
                        content: Some(serde_json::Value::String(content)),
                        tool_calls: None,
                        tool_call_id: Some(tool_use_id),
                    });
                }
            }
            // Handle regular text messages (with optional images)
            else {
                let content_val = make_content(&text_parts, &image_parts)
                    .unwrap_or(serde_json::Value::String(String::new()));

                messages.push(OpenAIMessage {
                    role: role.to_string(),
                    content: Some(content_val),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        // Convert tools to OpenAI format
        let tools: Option<Vec<OpenAITool>> = request.tools.map(|tools| {
            tools
                .iter()
                .map(|tool| OpenAITool {
                    r#type: "function".to_string(),
                    function: OpenAIFunction {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: tool.input_schema.clone(),
                    },
                })
                .collect()
        });

        // Newer OpenAI models (gpt-4.1-*, gpt-5-*, o1-*, o3-*) require
        // max_completion_tokens instead of max_tokens. Use the new field
        // for these models and fall back to max_tokens for everything else.
        let uses_completion_tokens = uses_max_completion_tokens(&request.model);
        let (max_tokens, max_completion_tokens) = if uses_completion_tokens {
            (None, request.max_tokens)
        } else {
            (request.max_tokens, None)
        };

        // Set tool_choice to "auto" when tools are present so the model
        // knows it is allowed to call them (MiniMax requires this explicitly).
        let tool_choice = tools
            .as_ref()
            .filter(|t| !t.is_empty())
            .map(|_| serde_json::json!("auto"));

        // Enable reasoning/thinking for OpenRouter and compatible endpoints.
        // Detection is intentionally broad — models that don't support the field ignore it.
        let base = self.base_url.to_lowercase();
        let include_reasoning = if base.contains("openrouter")
            || base.contains("openrouter.ai")
            || std::env::var("OPENCRABS_ENABLE_REASONING").is_ok()
        {
            Some(true)
        } else {
            None
        };

        OpenAIRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            max_tokens,
            max_completion_tokens,
            stream: Some(request.stream),
            stream_options: None,
            tools,
            tool_choice,
            include_reasoning,
        }
    }

    /// Convert to Anthropic-format request for OpenRouter with prompt caching.
    /// OpenRouter accepts this format and passes cache_control through to Anthropic
    /// models, caching the system prompt and tools across turns.
    fn to_anthropic_or_request(&self, request: LLMRequest) -> AnthropicORRequest {
        let cache = AnthropicORCacheControl {
            r#type: "ephemeral".to_string(),
        };

        // System prompt as cached content blocks
        let system = request.system.map(|s| {
            vec![AnthropicORSystemBlock {
                r#type: "text".to_string(),
                text: s,
                cache_control: Some(cache.clone()),
            }]
        });

        // Messages with content blocks
        let messages: Vec<AnthropicORMessage> = request
            .messages
            .into_iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "user", // system → user for Anthropic
                };

                let content: Vec<AnthropicORContentBlock> = msg
                    .content
                    .into_iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(AnthropicORContentBlock::Text {
                            text,
                            cache_control: None,
                        }),
                        ContentBlock::ToolUse { id, name, input } => {
                            Some(AnthropicORContentBlock::ToolUse { id, name, input })
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => Some(AnthropicORContentBlock::ToolResult {
                            tool_use_id,
                            content,
                        }),
                        ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => None,
                    })
                    .collect();

                AnthropicORMessage {
                    role: role.to_string(),
                    content,
                }
            })
            .collect();

        // Tools with cache_control on the last one
        let tools = request.tools.map(|tools| {
            let len = tools.len();
            tools
                .into_iter()
                .enumerate()
                .map(|(i, t)| AnthropicORTool {
                    name: t.name,
                    description: t.description,
                    input_schema: t.input_schema,
                    cache_control: if i == len - 1 {
                        Some(cache.clone())
                    } else {
                        None
                    },
                })
                .collect()
        });

        AnthropicORRequest {
            model: request.model,
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(16384),
            temperature: request.temperature,
            tools,
            stream: Some(request.stream),
        }
    }

    /// Convert OpenAI response to our generic format
    #[allow(clippy::wrong_self_convention)]
    fn from_openai_response(&self, response: OpenAIResponse) -> LLMResponse {
        let choice = response
            .choices
            .into_iter()
            .next()
            .unwrap_or_else(|| OpenAIChoice {
                index: 0,
                message: OpenAIMessage {
                    role: "assistant".to_string(),
                    content: Some(serde_json::Value::String(String::new())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("error".to_string()),
            });

        // Convert content to content blocks
        let mut content_blocks = Vec::new();

        // Add text content if present, stripping <think>...</think> reasoning blocks
        if let Some(ref content_val) = choice.message.content {
            let text = match content_val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(parts) => {
                    // Extract text from content parts array
                    parts
                        .iter()
                        .filter_map(|p| {
                            if p.get("type")?.as_str()? == "text" {
                                p.get("text")?.as_str().map(String::from)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                _ => String::new(),
            };
            if !text.is_empty() {
                let clean = strip_think_blocks(&text);
                if !clean.is_empty() {
                    content_blocks.push(ContentBlock::Text { text: clean });
                }
            }
        }

        // Convert tool_calls to ToolUse content blocks
        if let Some(tool_calls) = choice.message.tool_calls {
            tracing::debug!(
                "Converting {} tool calls from OpenAI response",
                tool_calls.len()
            );
            for tool_call in tool_calls {
                // Parse arguments JSON string
                let input =
                    serde_json::from_str(&tool_call.function.arguments).unwrap_or_else(|e| {
                        tracing::warn!(
                            "Failed to parse tool arguments for {}: {}",
                            tool_call.function.name,
                            e
                        );
                        serde_json::json!({})
                    });

                tracing::debug!(
                    "Converted tool call: {} with id {}",
                    tool_call.function.name,
                    tool_call.id
                );

                content_blocks.push(ContentBlock::ToolUse {
                    id: tool_call.id,
                    name: tool_call.function.name,
                    input,
                });
            }
        }

        // Detect models that dump tool JSON as text instead of structured calls
        let has_tool_text = content_blocks.iter().any(|b| {
            if let ContentBlock::Text { text } = b {
                (text.contains("\"function\"") && text.contains("\"arguments\""))
                    || (text.contains("tool_call") && text.contains("\"name\""))
                    || (text.contains("```json") && text.contains("\"command\""))
            } else {
                false
            }
        });
        let has_structured_tools = content_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        if has_tool_text && !has_structured_tools {
            tracing::warn!(
                "Model returned tool call JSON as text — likely does not support function calling"
            );
            content_blocks.push(ContentBlock::Text {
                text: "\n\n⚠️ **This model does not support function calling.** Tool requests were returned as text instead of executable calls. Consider switching to a model that supports tool use (e.g. Claude, GPT-4, Gemini).".to_string(),
            });
        }

        // Map finish_reason to StopReason
        let stop_reason = choice
            .finish_reason
            .and_then(|reason| match reason.as_str() {
                "stop" => Some(StopReason::EndTurn),
                "length" => Some(StopReason::MaxTokens),
                "tool_calls" | "function_call" => Some(StopReason::ToolUse),
                _ => None,
            });

        LLMResponse {
            id: response.id,
            model: response.model,
            content: content_blocks,
            stop_reason,
            usage: TokenUsage {
                input_tokens: response.usage.prompt_tokens.unwrap_or(0),
                output_tokens: response.usage.completion_tokens.unwrap_or(0),
                cache_creation_tokens: response.usage.cache_creation_input_tokens.unwrap_or(0),
                cache_read_tokens: response.usage.cache_read_input_tokens.unwrap_or(0),
                ..Default::default()
            },
        }
    }

    /// Handle API error response
    async fn handle_error(&self, response: reqwest::Response) -> ProviderError {
        let status = response.status().as_u16();

        // Extract Retry-After header for rate limits
        let retry_after = response.headers().get("retry-after").and_then(|v| {
            v.to_str().ok().and_then(|s| {
                // Retry-After can be either seconds or HTTP date
                // Try parsing as seconds first
                s.parse::<u64>().ok()
            })
        });

        // Try to parse error body
        if let Ok(error_body) = response.json::<OpenAIErrorResponse>().await {
            let message = if status == 429 {
                // Enhance rate limit error message
                if let Some(secs) = retry_after {
                    format!(
                        "{} (retry after {} seconds)",
                        error_body.error.message, secs
                    )
                } else {
                    format!(
                        "{} (rate limited, please retry later)",
                        error_body.error.message
                    )
                }
            } else {
                error_body.error.message
            };

            return if status == 429 {
                ProviderError::RateLimitExceeded(message)
            } else {
                ProviderError::ApiError {
                    status,
                    message,
                    error_type: Some(error_body.error.error_type.unwrap_or_default()),
                }
            };
        }

        // Fallback error
        if status == 429 {
            let message = if let Some(secs) = retry_after {
                format!("Rate limit exceeded (retry after {} seconds)", secs)
            } else {
                "Rate limit exceeded, please retry later".to_string()
            };
            ProviderError::RateLimitExceeded(message)
        } else {
            ProviderError::ApiError {
                status,
                message: "Unknown error".to_string(),
                error_type: None,
            }
        }
    }

    /// Execute an Anthropic-format request (used for OpenRouter prompt caching).
    /// OpenRouter returns OpenAI-format responses even when sent Anthropic format.
    async fn complete_with_anthropic_format(
        &self,
        model: String,
        message_count: usize,
        anthropic_request: AnthropicORRequest,
        retry_config: super::retry::RetryConfig,
    ) -> Result<LLMResponse> {
        use super::retry::retry_with_backoff;

        let tool_count = anthropic_request
            .tools
            .as_ref()
            .map(|t| t.len())
            .unwrap_or(0);
        tracing::info!(
            "OpenRouter (Anthropic format): model={}, messages={}, tools={}, cache_control=system+last_tool",
            model,
            message_count,
            tool_count
        );

        // Proactive pacing
        if let Some(ref limiter) = self.rate_limiter {
            let waited = limiter.wait().await;
            if !waited.is_zero() {
                tracing::debug!("Rate limiter: waited {:?} before request", waited);
            }
        }

        let result = retry_with_backoff(
            || async {
                let response = self
                    .client
                    .post(&self.base_url)
                    .headers(self.headers()?)
                    .json(&anthropic_request)
                    .send()
                    .await?;

                let status = response.status();
                if !status.is_success() {
                    return Err(self.handle_error(response).await);
                }

                let openai_response: OpenAIResponse = response.json().await?;
                let llm_response = self.from_openai_response(openai_response);

                // Log cache tokens if present
                if llm_response.usage.cache_creation_tokens > 0
                    || llm_response.usage.cache_read_tokens > 0
                {
                    tracing::info!(
                        "Cache: creation={}, read={}, total_cached={}",
                        llm_response.usage.cache_creation_tokens,
                        llm_response.usage.cache_read_tokens,
                        llm_response.usage.cache_creation_tokens
                            + llm_response.usage.cache_read_tokens
                    );
                }

                Ok(llm_response)
            },
            &retry_config,
        )
        .await;

        // Handle 400 token field mismatch — retry with swapped fields
        if let Err(ref e) = result
            && let ProviderError::ApiError {
                status: 400,
                message,
                ..
            } = e
            && is_token_field_mismatch(message)
        {
            tracing::warn!("Retrying with swapped max_tokens/max_completion_tokens");
            return Box::pin(self.complete_with_anthropic_format(
                model,
                message_count,
                anthropic_request,
                retry_config,
            ))
            .await;
        }

        result
    }

    /// Execute an Anthropic-format streaming request to OpenRouter.
    /// OpenRouter returns OpenAI-format SSE responses even when sent Anthropic format.
    async fn stream_with_anthropic_format(
        &self,
        model: String,
        message_count: usize,
        anthropic_request: AnthropicORRequest,
    ) -> Result<ProviderStream> {
        use super::retry::{RetryConfig, retry_with_backoff};

        let tool_count = anthropic_request
            .tools
            .as_ref()
            .map(|t| t.len())
            .unwrap_or(0);
        tracing::info!(
            "OpenRouter stream (Anthropic format): model={}, messages={}, tools={}, cache_control=system+last_tool",
            model,
            message_count,
            tool_count
        );

        // Proactive pacing
        if let Some(ref limiter) = self.rate_limiter {
            let waited = limiter.wait().await;
            if !waited.is_zero() {
                tracing::debug!("Rate limiter: waited {:?} before streaming request", waited);
            }
        }

        let response = retry_with_backoff(
            || async {
                let response = self
                    .client
                    .post(&self.base_url)
                    .headers(self.headers()?)
                    .json(&anthropic_request)
                    .send()
                    .await?;

                let status = response.status();
                if !status.is_success() {
                    return Err(self.handle_error(response).await);
                }

                Ok(response)
            },
            &RetryConfig::default(),
        )
        .await?;

        // Parse the SSE stream — OpenRouter returns OpenAI-format SSE.
        // total_input_tokens=0 since we don't have tiktoken counts for Anthropic format.
        self.parse_openai_stream(response, 0)
    }

    /// Parse an OpenAI-compatible SSE stream into a ProviderStream.
    /// `total_input_tokens` is used as fallback usage on stream end if no real usage arrives.
    fn parse_openai_stream(
        &self,
        response: reqwest::Response,
        total_input_tokens: usize,
    ) -> Result<ProviderStream> {
        use futures::stream::StreamExt;

        let byte_stream = response.bytes_stream();
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

        // Accumulated state for a single streamed tool call
        #[derive(Debug, Clone, Default)]
        struct ToolCallAccum {
            id: String,
            name: String,
            arguments: String,
        }

        /// State persisted across SSE chunks via Arc<Mutex<_>>
        struct StreamState {
            emitted_message_start: bool,
            emitted_content_start: bool,
            seen_delta_content: bool,
            tool_calls: std::collections::HashMap<usize, ToolCallAccum>,
            pending_stop_reason: Option<crate::brain::provider::types::StopReason>,
        }

        let state = std::sync::Arc::new(std::sync::Mutex::new(StreamState {
            emitted_message_start: false,
            emitted_content_start: false,
            seen_delta_content: false,
            tool_calls: std::collections::HashMap::new(),
            pending_stop_reason: None,
        }));

        let event_stream = byte_stream
            .map(move |chunk_result| -> Vec<std::result::Result<StreamEvent, ProviderError>> {
                match chunk_result {
                    Err(e) => vec![Err(ProviderError::StreamError(e.to_string()))],
                    Ok(chunk) => {
                        let raw_text = String::from_utf8_lossy(&chunk);
                        let mut buf = buffer.lock().expect("SSE buffer lock poisoned");
                        buf.push_str(&raw_text);

                        let mut events = Vec::new();
                        let mut st = state.lock().expect("SSE state lock");

                        while let Some(newline_pos) = buf.find('\n') {
                            let line = buf[..newline_pos].trim().to_string();
                            buf.drain(..=newline_pos);

                            if let Some(json_str) = line.strip_prefix("data: ") {
                                if json_str == "[DONE]" {
                                    for (_idx, accum) in st.tool_calls.drain() {
                                        let input = serde_json::from_str(&accum.arguments)
                                            .unwrap_or_else(|_| serde_json::json!({}));
                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                            index: _idx + 1,
                                            content_block: ContentBlock::ToolUse {
                                                id: accum.id,
                                                name: accum.name,
                                                input,
                                            },
                                        }));
                                    }
                                    if let Some(stop_reason) = st.pending_stop_reason.take() {
                                        events.push(Ok(StreamEvent::MessageDelta {
                                            delta: crate::brain::provider::types::MessageDelta {
                                                stop_reason: Some(stop_reason),
                                                stop_sequence: None,
                                            },
                                            usage: crate::brain::provider::types::TokenUsage {
                                                input_tokens: total_input_tokens as u32,
                                                output_tokens: 0, ..Default::default() },
                                        }));
                                    }
                                    events.push(Ok(StreamEvent::MessageStop));
                                    continue;
                                }

                                match serde_json::from_str::<OpenAIStreamChunk>(json_str) {
                                    Ok(chunk) => {
                                        if !st.emitted_message_start && !chunk.id.is_empty() {
                                            st.emitted_message_start = true;
                                            let model = chunk.model.clone().unwrap_or_default();
                                            events.push(Ok(StreamEvent::MessageStart {
                                                message: crate::brain::provider::types::StreamMessage {
                                                    id: chunk.id,
                                                    model,
                                                    role: Role::Assistant,
                                                    usage: crate::brain::provider::types::TokenUsage {
                                                        input_tokens: 0,
                                                        output_tokens: 0, ..Default::default() },
                                                },
                                            }));
                                        }

                                        let delta_content = chunk.choices.first()
                                            .and_then(|c| c.delta.as_ref())
                                            .and_then(|d| d.content.as_ref());

                                        let is_finish = chunk.choices.first()
                                            .map(|c| c.finish_reason.is_some())
                                            .unwrap_or(false);

                                        if is_finish {
                                            if let Some(usage) = chunk.usage.as_ref() {
                                                let input = usage.prompt_tokens.unwrap_or(0);
                                                let output = usage.completion_tokens.unwrap_or(0);
                                                let mut token_usage = crate::brain::provider::types::TokenUsage {
                                                    input_tokens: input,
                                                    output_tokens: output,
                                                    ..Default::default()
                                                };
                                                if let Some(cache_create) = usage.cache_creation_input_tokens {
                                                    token_usage.cache_creation_tokens = cache_create;
                                                }
                                                if let Some(cache_read) = usage.cache_read_input_tokens {
                                                    token_usage.cache_read_tokens = cache_read;
                                                }
                                                events.push(Ok(StreamEvent::MessageDelta {
                                                    delta: crate::brain::provider::types::MessageDelta {
                                                        stop_reason: None,
                                                        stop_sequence: None,
                                                    },
                                                    usage: token_usage,
                                                }));
                                            }
                                            continue;
                                        }

                                        if let Some(text) = delta_content {
                                            if !st.emitted_content_start {
                                                st.emitted_content_start = true;
                                                st.seen_delta_content = true;
                                                events.push(Ok(StreamEvent::ContentBlockStart {
                                                    index: 0,
                                                    content_block: ContentBlock::Text { text: text.clone() },
                                                }));
                                            } else {
                                                events.push(Ok(StreamEvent::ContentBlockDelta {
                                                    index: 0,
                                                    delta: ContentDelta::TextDelta { text: text.clone() },
                                                }));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::debug!("[STREAM_PARSE] Failed to parse SSE chunk: {}", e);
                                    }
                                }
                            }
                        }

                        if events.is_empty() {
                            vec![]
                        } else {
                            events
                        }
                    }
                }
            })
            .filter(|events| {
                let non_empty = !events.is_empty();
                async move { non_empty }
            })
            .flat_map(futures::stream::iter);

        Ok(Box::pin(event_stream))
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        use super::retry::{RetryConfig, retry_with_backoff};

        let model = request.model.clone();
        let message_count = request.messages.len();
        let retry_config = RetryConfig::default();

        // For OpenRouter, use Anthropic format with cache_control for prompt caching.
        // For all other providers, use standard OpenAI format.
        if self.is_openrouter() {
            let anthropic_request = self.to_anthropic_or_request(request.clone());
            return self
                .complete_with_anthropic_format(
                    model,
                    message_count,
                    anthropic_request,
                    retry_config,
                )
                .await;
        }

        let mut openai_request = self.to_openai_request(request);

        let tool_count = openai_request.tools.as_ref().map(|t| t.len()).unwrap_or(0);
        tracing::info!(
            "OpenAI API request: model={}, messages={}, max_tokens={:?}, max_completion_tokens={:?}, tools={}",
            model,
            message_count,
            openai_request.max_tokens,
            openai_request.max_completion_tokens,
            tool_count
        );
        if tool_count == 0 {
            tracing::warn!(
                "OpenAI request has NO tools - LLM won't know about file/bash operations!"
            );
        }

        // Proactive pacing: stay under provider rate limits (e.g. OpenRouter :free at 20/min)
        if let Some(ref limiter) = self.rate_limiter {
            let waited = limiter.wait().await;
            if !waited.is_zero() {
                tracing::debug!(
                    "Rate limiter: waited {:?} before request to {}",
                    waited,
                    self.base_url
                );
            }
        }

        // Retry the entire API call with exponential backoff
        let result = retry_with_backoff(
            || async {
                tracing::debug!("Sending request to OpenAI API: {}", self.base_url);
                let response = self
                    .client
                    .post(&self.base_url)
                    .headers(self.headers()?)
                    .json(&openai_request)
                    .send()
                    .await?;

                let status = response.status();
                tracing::debug!("OpenAI API response status: {}", status);

                if !status.is_success() {
                    return Err(self.handle_error(response).await);
                }

                let openai_response: OpenAIResponse = response.json().await?;
                let llm_response = self.from_openai_response(openai_response);

                tracing::info!(
                    "OpenAI API response: input_tokens={}, output_tokens={}, stop_reason={:?}",
                    llm_response.usage.input_tokens,
                    llm_response.usage.output_tokens,
                    llm_response.stop_reason
                );

                Ok(llm_response)
            },
            &retry_config,
        )
        .await;

        // Resilient fallback: if the API rejected max_tokens / max_completion_tokens,
        // swap the fields and retry once.
        if let Err(ref e) = result {
            if is_token_field_mismatch(&e.to_string()) {
                tracing::warn!(
                    "Token field mismatch for model {}, retrying with swapped fields",
                    model
                );
                openai_request.swap_token_fields();
                return retry_with_backoff(
                    || async {
                        let response = self
                            .client
                            .post(&self.base_url)
                            .headers(self.headers()?)
                            .json(&openai_request)
                            .send()
                            .await?;
                        if !response.status().is_success() {
                            return Err(self.handle_error(response).await);
                        }
                        let openai_response: OpenAIResponse = response.json().await?;
                        Ok(self.from_openai_response(openai_response))
                    },
                    &retry_config,
                )
                .await;
            }
            tracing::error!("OpenAI API request failed: {}", e);
        }

        result
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        use super::retry::{RetryConfig, retry_with_backoff};

        let model = request.model.clone();
        let message_count = request.messages.len();

        // Proactive pacing: stay under provider rate limits
        if let Some(ref limiter) = self.rate_limiter {
            let waited = limiter.wait().await;
            if !waited.is_zero() {
                tracing::debug!(
                    "Rate limiter: waited {:?} before streaming request to {}",
                    waited,
                    self.base_url
                );
            }
        }

        tracing::info!(
            "{} streaming request: model={}, messages={}, base_url={}",
            self.name(),
            model,
            message_count,
            self.base_url
        );

        // For OpenRouter, use Anthropic format with cache_control for prompt caching.
        if self.is_openrouter() {
            let anthropic_request = self.to_anthropic_or_request(request);
            return self
                .stream_with_anthropic_format(model, message_count, anthropic_request)
                .await;
        }

        let mut openai_request = self.to_openai_request(request);
        openai_request.stream = Some(true);
        openai_request.stream_options = Some(StreamOptions {
            include_usage: true,
        });

        let tools_count = openai_request.tools.as_ref().map(|t| t.len()).unwrap_or(0);

        // Count input tokens via tiktoken (cl100k_base) to monitor context window usage.
        // Each message: content tokens + serialized tool_calls tokens + 4 overhead per message.
        let message_tokens: usize = openai_request
            .messages
            .iter()
            .map(|m| {
                let content = m
                    .content
                    .as_ref()
                    .map(|v| {
                        let s = v.as_str().unwrap_or("");
                        count_message_tokens(s)
                    })
                    .unwrap_or(4);
                let tool_calls = m
                    .tool_calls
                    .as_ref()
                    .map(|tc| count_tokens(&serde_json::to_string(tc).unwrap_or_default()))
                    .unwrap_or(0);
                content + tool_calls
            })
            .sum();
        let tool_schema_tokens = openai_request
            .tools
            .as_ref()
            .map(|tools| count_tokens(&serde_json::to_string(tools).unwrap_or_default()))
            .unwrap_or(0);
        let total_input_tokens = message_tokens + tool_schema_tokens;
        let context_pct = (total_input_tokens as f32 / 200_000.0 * 100.0).round() as u32;
        tracing::debug!(
            "OpenAI stream request: ~{} input tokens ({}% of 200k window) — {} messages, {} tool schemas",
            total_input_tokens,
            context_pct,
            openai_request.messages.len(),
            tools_count
        );

        let retry_config = RetryConfig::default();

        // Retry the stream connection establishment
        let mut response = retry_with_backoff(
            || async {
                let response = self
                    .client
                    .post(&self.base_url)
                    .headers(self.headers()?)
                    .json(&openai_request)
                    .send()
                    .await?;

                tracing::debug!("OpenAI response status: {}", response.status());

                if !response.status().is_success() {
                    return Err(self.handle_error(response).await);
                }

                Ok(response)
            },
            &retry_config,
        )
        .await;

        // Resilient fallback: if the API rejected max_tokens / max_completion_tokens,
        // swap the fields and retry once.
        if let Err(ref e) = response
            && is_token_field_mismatch(&e.to_string())
        {
            tracing::warn!(
                "Token field mismatch for model {} (stream), retrying with swapped fields",
                model
            );
            openai_request.swap_token_fields();
            response = retry_with_backoff(
                || async {
                    let r = self
                        .client
                        .post(&self.base_url)
                        .headers(self.headers()?)
                        .json(&openai_request)
                        .send()
                        .await?;
                    if !r.status().is_success() {
                        return Err(self.handle_error(r).await);
                    }
                    Ok(r)
                },
                &retry_config,
            )
            .await;
        }
        let response = response?;

        // Parse Server-Sent Events stream - return Vec to emit multiple events like Anthropic
        let byte_stream = response.bytes_stream();
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

        // Accumulated state for a single streamed tool call
        #[derive(Debug, Clone, Default)]
        struct ToolCallAccum {
            id: String,
            name: String,
            arguments: String,
        }

        /// State persisted across SSE chunks via Arc<Mutex<_>>
        struct StreamState {
            emitted_message_start: bool,
            emitted_content_start: bool,
            /// True once we've received real content via `delta` field
            seen_delta_content: bool,
            /// Index -> accumulated tool call
            tool_calls: std::collections::HashMap<usize, ToolCallAccum>,
            /// True while inside a stripped block (think/reasoning/tools-v2)
            inside_think: bool,
            /// Index into STRIP_CLOSE_TAGS for the currently active block
            active_close_tag: usize,
            /// Bytes consumed while inside_think is true (no close tag found).
            /// If this exceeds the threshold, we abandon filtering and pass
            /// content through — the model likely hallucinated an open tag
            /// without a matching close (e.g. `<!-- tools-v2: ...` with no `-->`).
            think_bytes_consumed: usize,
            /// Stashed stop_reason from finish_reason chunk, emitted with
            /// the final usage-only chunk (MiniMax/OpenAI include_usage flow).
            pending_stop_reason: Option<crate::brain::provider::types::StopReason>,
        }

        let state = std::sync::Arc::new(std::sync::Mutex::new(StreamState {
            emitted_message_start: false,
            emitted_content_start: false,
            seen_delta_content: false,
            tool_calls: std::collections::HashMap::new(),
            inside_think: false,
            active_close_tag: 0,
            think_bytes_consumed: 0,
            pending_stop_reason: None,
        }));

        let event_stream = byte_stream
            .map(move |chunk_result| -> Vec<std::result::Result<StreamEvent, ProviderError>> {
                match chunk_result {
                    Err(e) => vec![Err(ProviderError::StreamError(e.to_string()))],
                    Ok(chunk) => {
                        // GRANULAR LOG: Raw SSE chunk
                        let raw_text = String::from_utf8_lossy(&chunk);
                        tracing::debug!("[STREAM_RAW] SSE chunk: {}", raw_text.chars().take(500).collect::<String>());
                        if raw_text.contains("tool_calls") {
                            tracing::debug!("[STREAM_RAW] SSE chunk with tool_calls: {}", raw_text.chars().take(500).collect::<String>());
                        }

                        let mut buf = buffer.lock().expect("SSE buffer lock poisoned");
                        buf.push_str(&raw_text);

                        let mut events = Vec::new();
                        let mut st = state.lock().expect("SSE state lock");

                        // Process complete lines (terminated by \n)
                        while let Some(newline_pos) = buf.find('\n') {
                            let line = buf[..newline_pos].trim().to_string();
                            buf.drain(..=newline_pos);

                            if let Some(json_str) = line.strip_prefix("data: ") {
                                if json_str == "[DONE]" {
                                    // Flush any accumulated tool calls before DONE
                                    for (_idx, accum) in st.tool_calls.drain() {
                                        let input = serde_json::from_str(&accum.arguments)
                                            .unwrap_or_else(|_| serde_json::json!({}));
                                        tracing::info!(
                                            "[TOOL_EMIT] Flushing tool on DONE: id={}, name={}, args={}",
                                            accum.id, accum.name, &accum.arguments.chars().take(200).collect::<String>()
                                        );
                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                            index: _idx + 1, // Offset to avoid collision with text block at index 0
                                            content_block: ContentBlock::ToolUse {
                                                id: accum.id,
                                                name: accum.name,
                                                input,
                                            },
                                        }));
                                    }
                                    // If we still have a pending stop_reason (no usage-only chunk
                                    // arrived), emit MessageDelta with fallback usage now.
                                    if let Some(stop_reason) = st.pending_stop_reason.take() {
                                        tracing::info!("[STREAM_USAGE] Final usage (fallback on DONE): input={}, output=0", total_input_tokens);
                                        events.push(Ok(StreamEvent::MessageDelta {
                                            delta: crate::brain::provider::types::MessageDelta {
                                                stop_reason: Some(stop_reason),
                                                stop_sequence: None,
                                            },
                                            usage: crate::brain::provider::types::TokenUsage {
                                                input_tokens: total_input_tokens as u32,
                                                output_tokens: 0, ..Default::default() },
                                        }));
                                    }
                                    events.push(Ok(StreamEvent::MessageStop));
                                    continue;
                                }

                                // Check for z.ai/provider-specific inline errors (HTTP 200 with error in body)
                                if let Ok(raw) = serde_json::from_str::<serde_json::Value>(json_str)
                                    && let Some(status_msg) = raw.pointer("/base_resp/status_msg").and_then(|v| v.as_str())
                                {
                                    let status_code = raw.pointer("/base_resp/status_code").and_then(|v| v.as_u64()).unwrap_or(0);
                                    if status_code != 0 {
                                        tracing::error!("[STREAM_ERROR] Provider returned inline error: code={}, msg={}", status_code, status_msg);
                                        events.push(Err(ProviderError::ApiError {
                                            status: status_code as u16,
                                            message: status_msg.to_string(),
                                            error_type: Some("provider_error".to_string()),
                                        }));
                                        continue;
                                    }
                                }

                                match serde_json::from_str::<OpenAIStreamChunk>(json_str) {
                                    Ok(chunk) => {
                                        // Emit MessageStart on first chunk with id
                                        if !st.emitted_message_start && !chunk.id.is_empty() {
                                            st.emitted_message_start = true;
                                            let model = chunk.model.clone().unwrap_or_default();
                                            events.push(Ok(StreamEvent::MessageStart {
                                                message: crate::brain::provider::types::StreamMessage {
                                                    id: chunk.id,
                                                    model,
                                                    role: Role::Assistant,
                                                    usage: crate::brain::provider::types::TokenUsage {
                                                        input_tokens: 0,
                                                        output_tokens: 0, ..Default::default() },
                                                },
                                            }));
                                        }

                                        // Get content from delta or message (MiniMax uses message).
                                        // IMPORTANT: Some providers (LM Studio, etc.) send the FULL
                                        // response in the final chunk's `message` field while `delta`
                                        // is absent. If we already received content via delta, we must
                                        // NOT fall back to `message` or we'll duplicate the entire text.
                                        let delta_content = chunk.choices.first()
                                            .and_then(|c| c.delta.as_ref())
                                            .and_then(|d| d.content.as_ref())
                                            .cloned();
                                        let content = if delta_content.is_some() {
                                            if delta_content.as_ref().is_some_and(|s| !s.is_empty()) {
                                                st.seen_delta_content = true;
                                            }
                                            delta_content
                                        } else if !st.seen_delta_content {
                                            // Only use message field if we've never seen delta content
                                            // (MiniMax always uses message, standard providers don't)
                                            chunk.choices.first()
                                                .and_then(|c| c.message.as_ref())
                                                .and_then(|d| d.content.as_ref())
                                                .cloned()
                                        } else {
                                            None
                                        };

                                        // Get streaming tool_calls from delta or message
                                        let tool_calls = chunk.choices.first()
                                            .and_then(|c| c.delta.as_ref().or(c.message.as_ref()))
                                            .and_then(|d| d.tool_calls.as_ref());

                                        // Accumulate tool calls across chunks
                                        // OpenAI streaming sends: chunk1={index,id,type,name,args:""}, chunk2..N={index,args:"<fragment>"}
                                        if let Some(tc_list) = tool_calls {
                                            for tc_item in tc_list {
                                                let idx = tc_item.index;
                                                let accum = st.tool_calls.entry(idx).or_default();

                                                // First chunk for this index carries id + name
                                                if let Some(ref id) = tc_item.id
                                                    && !id.is_empty() {
                                                        accum.id = id.clone();
                                                    }
                                                if let Some(ref func) = tc_item.function {
                                                    if let Some(ref name) = func.name
                                                        && !name.is_empty() {
                                                            accum.name = name.clone();
                                                        }
                                                    // Append argument fragment
                                                    if let Some(ref args) = func.arguments {
                                                        accum.arguments.push_str(args);
                                                    }
                                                }

                                                tracing::debug!(
                                                    "[TOOL_ACCUM] idx={}, id={}, name={}, args_len={}, args_tail={}",
                                                    idx, accum.id, accum.name, accum.arguments.len(),
                                                    accum.arguments.chars().rev().take(60).collect::<String>().chars().rev().collect::<String>()
                                                );
                                            }
                                        }

                                        // Check finish_reason — emit accumulated tool calls when done
                                        let finish_reason_str = chunk.choices.first()
                                            .and_then(|c| c.finish_reason.as_ref());

                                        // Flush accumulated tool calls on any terminal finish_reason.
                                        // Some providers (MiniMax) send "stop" even with tool_calls.
                                        if finish_reason_str.is_some() && !st.tool_calls.is_empty() {
                                                // Emit all accumulated tool calls
                                                for (idx, accum) in st.tool_calls.drain() {
                                                    let input = serde_json::from_str(&accum.arguments)
                                                        .unwrap_or_else(|e| {
                                                            tracing::warn!(
                                                                "[TOOL_EMIT] Failed to parse accumulated args for '{}': {} | args: {}",
                                                                accum.name, e, &accum.arguments.chars().take(300).collect::<String>()
                                                            );
                                                            serde_json::json!({})
                                                        });
                                                    tracing::info!(
                                                        "[TOOL_EMIT] Emitting tool call: idx={}, id={}, name={}, args_len={}",
                                                        idx, accum.id, accum.name, accum.arguments.len()
                                                    );
                                                    events.push(Ok(StreamEvent::ContentBlockStart {
                                                        index: idx + 1, // Offset by 1 to avoid collision with text block at index 0
                                                        content_block: ContentBlock::ToolUse {
                                                            id: accum.id,
                                                            name: accum.name,
                                                            input,
                                                        },
                                                    }));
                                                }
                                            }

                                        // Emit text content, filtering <think>...</think> reasoning blocks
                                        if let Some(ref c) = content {
                                            let (mut inside, mut close_idx, mut consumed) =
                                                (st.inside_think, st.active_close_tag, st.think_bytes_consumed);
                                            let filtered = filter_think_tags(c, &mut inside, &mut close_idx, &mut consumed);
                                            st.inside_think = inside;
                                            st.active_close_tag = close_idx;
                                            st.think_bytes_consumed = consumed;

                                            if !filtered.is_empty() {
                                                if !st.emitted_content_start {
                                                    st.emitted_content_start = true;
                                                    events.push(Ok(StreamEvent::ContentBlockStart {
                                                        index: 0,
                                                        content_block: ContentBlock::Text { text: String::new() },
                                                    }));
                                                }

                                                events.push(Ok(StreamEvent::ContentBlockDelta {
                                                    index: 0,
                                                    delta: ContentDelta::TextDelta {
                                                        text: filtered,
                                                    },
                                                }));
                                            } else if !st.emitted_content_start && c.is_empty() {
                                                st.emitted_content_start = true;
                                                events.push(Ok(StreamEvent::ContentBlockStart {
                                                    index: 0,
                                                    content_block: ContentBlock::Text { text: String::new() },
                                                }));
                                            }
                                        }

                                        // Extract reasoning_content (MiniMax thinking process).
                                        // MiniMax sends incremental deltas in `delta.reasoning_content`,
                                        // and the full accumulated string in `message.reasoning_content`.
                                        // Use delta first (incremental), skip message to avoid duplication.
                                        let reasoning = chunk.choices.first()
                                            .and_then(|c| c.delta.as_ref())
                                            .and_then(|d| d.reasoning_content.as_ref())
                                            .cloned();
                                        if let Some(rc) = reasoning && !rc.is_empty() {
                                            events.push(Ok(StreamEvent::ContentBlockDelta {
                                                index: 0,
                                                delta: ContentDelta::ReasoningDelta {
                                                    text: rc,
                                                },
                                            }));
                                        }

                                        // Emit MessageDelta when finish_reason is present.
                                        // Do NOT emit MessageStop here — providers that support
                                        // stream_options.include_usage (MiniMax, OpenAI) send a
                                        // final usage-only chunk AFTER this one. We handle
                                        // MessageStop on [DONE] or the usage-only chunk below.
                                        if let Some(reason) = finish_reason_str {
                                            let (raw_input, raw_output) = if let Some(ref usage) = chunk.usage {
                                                (usage.prompt_tokens.unwrap_or(0), usage.completion_tokens.unwrap_or(0))
                                            } else {
                                                (0, 0)
                                            };

                                            let stop_reason = Some(match reason.as_str() {
                                                "stop" => crate::brain::provider::types::StopReason::EndTurn,
                                                "length" => crate::brain::provider::types::StopReason::MaxTokens,
                                                "tool_calls" | "function_call" => crate::brain::provider::types::StopReason::ToolUse,
                                                _ => crate::brain::provider::types::StopReason::EndTurn,
                                            });

                                            // If this chunk already carries real usage (some
                                            // providers inline it), emit immediately + stop.
                                            if raw_input > 0 || raw_output > 0 {
                                                tracing::info!("[STREAM_USAGE] Final usage (inline): input={}, output={}", raw_input, raw_output);
                                                events.push(Ok(StreamEvent::MessageDelta {
                                                    delta: crate::brain::provider::types::MessageDelta {
                                                        stop_reason,
                                                        stop_sequence: None,
                                                    },
                                                    usage: crate::brain::provider::types::TokenUsage {
                                                        input_tokens: raw_input,
                                                        output_tokens: raw_output, ..Default::default() },
                                                }));
                                                events.push(Ok(StreamEvent::MessageStop));
                                            } else {
                                                // Stash stop_reason — we'll emit the final MessageDelta
                                                // with real usage once the usage-only chunk arrives.
                                                st.pending_stop_reason = stop_reason;
                                            }
                                        }

                                        // Handle usage-only chunk: choices is empty, usage has
                                        // real token counts. MiniMax and OpenAI send this as
                                        // the final chunk when stream_options.include_usage=true.
                                        if chunk.choices.is_empty()
                                            && let Some(ref usage) = chunk.usage {
                                                let input = usage.prompt_tokens.unwrap_or(0);
                                                let output = usage.completion_tokens.unwrap_or(0);
                                                if input > 0 || output > 0 {
                                                    tracing::info!("[STREAM_USAGE] Final usage: input={}, output={}", input, output);
                                                    events.push(Ok(StreamEvent::MessageDelta {
                                                        delta: crate::brain::provider::types::MessageDelta {
                                                            stop_reason: st.pending_stop_reason.take(),
                                                            stop_sequence: None,
                                                        },
                                                        usage: crate::brain::provider::types::TokenUsage {
                                                            input_tokens: input,
                                                            output_tokens: output, ..Default::default() },
                                                    }));
                                                    events.push(Ok(StreamEvent::MessageStop));
                                                }
                                        }
                                    }
                                    Err(e) => {
                                        let json_preview = json_str.chars().take(300).collect::<String>();
                                        tracing::warn!(
                                            "[STREAM_PARSE] Failed to parse chunk: {} | Raw: {}",
                                            e, json_preview
                                        );
                                    }
                                }
                            }
                        }

                        if events.is_empty() {
                            vec![Ok(StreamEvent::Ping)]
                        } else {
                            events
                        }
                    }
                }
            })
            .flat_map(futures::stream::iter);

        Ok(Box::pin(event_stream))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
        self.vision_model.is_some()
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> &str {
        self.custom_default_model.as_deref().unwrap_or_else(|| {
            tracing::error!(
                "No default_model configured for provider '{}' — check config.toml",
                self.name
            );
            "MISSING_MODEL"
        })
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "gpt-4-turbo-preview".to_string(),
            "gpt-4".to_string(),
            "gpt-4-32k".to_string(),
            "gpt-3.5-turbo".to_string(),
            "gpt-3.5-turbo-16k".to_string(),
        ]
    }

    async fn fetch_models(&self) -> Vec<String> {
        // Derive models URL from base_url (replace /chat/completions with /models)
        let models_url = self.base_url.replace("/chat/completions", "/models");

        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }
        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }

        let headers = match self.headers() {
            Ok(h) => h,
            Err(_) => return self.supported_models(),
        };
        match self.client.get(&models_url).headers(headers).send().await {
            Ok(resp) if resp.status().is_success() => match resp.json::<ModelsResponse>().await {
                Ok(body) => {
                    let mut models: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
                    models.sort();
                    if models.is_empty() {
                        return self.supported_models();
                    }
                    models
                }
                Err(_) => self.supported_models(),
            },
            _ => self.supported_models(),
        }
    }

    fn configured_context_window(&self) -> Option<u32> {
        self.configured_context_window
    }

    fn context_window(&self, model: &str) -> Option<u32> {
        // User-configured value takes priority over model-name heuristics
        if let Some(cw) = self.configured_context_window {
            return Some(cw);
        }
        let m = model.to_lowercase();
        // gpt-5 family
        if m.starts_with("gpt-5") {
            return Some(1_047_576); // 1M tokens
        }
        // gpt-4.1 family
        if m.starts_with("gpt-4.1") {
            return Some(1_047_576); // 1M tokens
        }
        // o-series reasoning models
        if m.starts_with("o4") || m.starts_with("o3") {
            return Some(200_000);
        }
        if m.starts_with("o1") {
            return Some(200_000);
        }
        // gpt-4o family
        if m.starts_with("gpt-4o") {
            return Some(128_000);
        }
        match model {
            "gpt-4-turbo" | "gpt-4-turbo-preview" => Some(128_000),
            "gpt-4" => Some(8_192),
            "gpt-4-32k" => Some(32_768),
            "gpt-3.5-turbo" => Some(16_384),
            "gpt-3.5-turbo-16k" => Some(16_384),
            _ => None,
        }
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        // Always load fresh from disk — avoids stale OnceLock cache
        // that may have been initialized before usage_pricing.toml existed
        crate::pricing::PricingConfig::load().calculate_cost(model, input_tokens, output_tokens)
    }
}

/// Returns true if this model requires `max_completion_tokens` instead of `max_tokens`.
/// Newer OpenAI models (gpt-4.1-*, gpt-5-*, o1-*, o3-*) reject `max_tokens`.
pub(crate) fn uses_max_completion_tokens(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("gpt-4.1")
        || m.starts_with("gpt-5")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
}

// ============================================================================
// OpenAI API Types
// ============================================================================
// Anthropic-format request for OpenRouter (enables prompt caching)
// ============================================================================

/// Anthropic-style request for OpenRouter when routing to Anthropic models.
/// OpenRouter accepts this format and passes cache_control through to Anthropic.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORRequest {
    model: String,
    messages: Vec<AnthropicORMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<AnthropicORSystemBlock>>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicORTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

/// System content block with cache_control support.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORSystemBlock {
    r#type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<AnthropicORCacheControl>,
}

/// Message in Anthropic format with content blocks.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORMessage {
    role: String,
    content: Vec<AnthropicORContentBlock>,
}

/// Content block for messages (text, tool_use, tool_result).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicORContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicORCacheControl>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// Tool definition with cache_control support.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<AnthropicORCacheControl>,
}

/// Ephemeral cache control marker.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORCacheControl {
    r#type: String,
}

// ============================================================================
// OpenAI-compatible request/response types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    /// Legacy token limit field — used by older OpenAI models (gpt-4o, gpt-3.5, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// New token limit field — required by newer OpenAI models (gpt-4.1-*, gpt-5-*, o1-*, o3-*)
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool>>,
    /// Tells the model whether/how to call tools. "auto" = model decides.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    /// OpenRouter: request reasoning/thinking tokens in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    include_reasoning: Option<bool>,
}

impl OpenAIRequest {
    /// Swap max_tokens ↔ max_completion_tokens for retry after a 400 error.
    fn swap_token_fields(&mut self) {
        let old_max = self.max_tokens.take();
        let old_completion = self.max_completion_tokens.take();
        self.max_tokens = old_completion;
        self.max_completion_tokens = old_max;
    }
}

/// Returns true if the error message indicates a max_tokens / max_completion_tokens mismatch.
pub(crate) fn is_token_field_mismatch(msg: &str) -> bool {
    let m = msg.to_lowercase();
    (m.contains("max_tokens") || m.contains("max_completion_tokens")) && m.contains("unsupported")
}

#[derive(Debug, Clone, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAIMessage {
    role: String,
    /// Either a plain string or an array of content parts (text + image_url).
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAIToolCall {
    id: String,
    r#type: String,
    function: OpenAIFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAITool {
    r#type: String,
    function: OpenAIFunction,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAIFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIResponse {
    id: String,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OpenAIChoice {
    index: u32,
    message: OpenAIMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIUsage {
    #[serde(rename = "prompt_tokens")]
    prompt_tokens: Option<u32>,
    #[serde(rename = "completion_tokens")]
    completion_tokens: Option<u32>,
    /// Anthropic-style cache tokens — passed through by OpenRouter for Anthropic models.
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OpenAIStreamChunk {
    id: String,
    model: Option<String>,
    choices: Vec<OpenAIStreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<OpenAIUsage>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OpenAIStreamChoice {
    index: u32,
    delta: Option<OpenAIMessageDelta>,
    message: Option<OpenAIMessageDelta>,
    finish_reason: Option<String>,
}

/// Streaming tool call — fields are optional because OpenAI sends them
/// incrementally: first chunk has id/type/name, continuation chunks only
/// have index + argument fragments.
#[derive(Debug, Clone, Deserialize)]
struct StreamingToolCall {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamingFunctionCall>,
}

#[derive(Debug, Clone, Deserialize)]
struct StreamingFunctionCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OpenAIMessageDelta {
    role: Option<String>,
    content: Option<String>,
    #[serde(default, alias = "reasoning")]
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<StreamingToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIErrorResponse {
    error: OpenAIError,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIError {
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
}
