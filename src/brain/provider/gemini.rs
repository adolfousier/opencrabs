//! Google Gemini Provider Implementation
//!
//! Implements the Provider trait for Google's Gemini models.
//! Uses the Gemini REST API (different format from OpenAI-compatible APIs).
//!
//! ## API Format
//! - Base URL: `https://generativelanguage.googleapis.com/v1beta`
//! - Auth: `?key={api_key}` query parameter
//! - Chat: `POST /models/{model}:generateContent`
//! - Stream: `POST /models/{model}:streamGenerateContent?alt=sse`
//!
//! ## Role Mapping
//! Gemini uses `"user"` and `"model"` (not `"assistant"`)
//!
//! ## Supported Models
//! - gemini-2.0-flash
//! - gemini-3.1-flash-image-preview
//! - gemini-1.5-pro
//! - gemini-1.5-flash

use super::error::{ProviderError, Result};
use super::r#trait::{Provider, ProviderStream};
use super::types::*;
use async_trait::async_trait;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Google Gemini provider
#[derive(Clone)]
pub struct GeminiProvider {
    api_key: String,
    client: Client,
    model: String,
}

impl GeminiProvider {
    /// Create a new Gemini provider
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
            client,
            model: "gemini-2.0-flash".to_string(),
        }
    }

    /// Set the default model
    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }

    /// Build the generate content URL for a given model
    fn generate_url(&self, model: &str, stream: bool) -> String {
        if stream {
            format!(
                "{}/models/{}:streamGenerateContent?alt=sse&key={}",
                GEMINI_BASE_URL, model, self.api_key
            )
        } else {
            format!(
                "{}/models/{}:generateContent?key={}",
                GEMINI_BASE_URL, model, self.api_key
            )
        }
    }

    /// Convert our LLMRequest to the Gemini request format
    fn build_gemini_request(&self, request: &LLMRequest) -> Value {
        let mut contents: Vec<Value> = Vec::new();
        let mut pending_tool_results: Vec<Value> = Vec::new();

        for msg in &request.messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "model",
                Role::System => continue, // System messages go into systemInstruction
            };

            // Collect parts for this message
            let mut text_parts: Vec<Value> = Vec::new();
            let mut tool_use_parts: Vec<Value> = Vec::new();
            let mut tool_result_parts: Vec<Value> = Vec::new();

            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        text_parts.push(serde_json::json!({"text": text}));
                    }
                    ContentBlock::Thinking { .. } => {
                        // Gemini doesn't use Anthropic-style thinking blocks; skip.
                    }
                    ContentBlock::Image { source } => {
                        let inline_data = match source {
                            ImageSource::Base64 { media_type, data } => {
                                serde_json::json!({
                                    "inlineData": {
                                        "mimeType": media_type,
                                        "data": data
                                    }
                                })
                            }
                            ImageSource::Url { url } => {
                                serde_json::json!({
                                    "fileData": {
                                        "fileUri": url
                                    }
                                })
                            }
                        };
                        text_parts.push(inline_data);
                    }
                    ContentBlock::ToolUse { id: _, name, input } => {
                        tool_use_parts.push(serde_json::json!({
                            "functionCall": {
                                "name": name,
                                "args": input
                            }
                        }));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id: _,
                        content,
                        is_error: _,
                    } => {
                        tool_result_parts.push(serde_json::json!({
                            "functionResponse": {
                                "name": "tool_result",
                                "response": {"output": content}
                            }
                        }));
                    }
                }
            }

            // Tool results must be bundled into a "user" message with functionResponse parts
            if !tool_result_parts.is_empty() {
                pending_tool_results.extend(tool_result_parts);
                continue;
            }

            // Flush any pending tool results before this message
            if !pending_tool_results.is_empty() {
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": pending_tool_results.clone()
                }));
                pending_tool_results.clear();
            }

            let mut all_parts = text_parts;
            all_parts.extend(tool_use_parts);

            if all_parts.is_empty() {
                all_parts.push(serde_json::json!({"text": ""}));
            }

            contents.push(serde_json::json!({
                "role": role,
                "parts": all_parts
            }));
        }

        // Flush any remaining tool results
        if !pending_tool_results.is_empty() {
            contents.push(serde_json::json!({
                "role": "user",
                "parts": pending_tool_results
            }));
        }

        let mut body = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": request.max_tokens.unwrap_or(65536)
            }
        });

        // System instruction
        if let Some(ref system) = request.system {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": system}]
            });
        }

        // Tools
        if let Some(ref tools) = request.tools
            && !tools.is_empty()
        {
            let function_declarations: Vec<Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema
                    })
                })
                .collect();
            body["tools"] = serde_json::json!([{
                "functionDeclarations": function_declarations
            }]);
            body["toolConfig"] = serde_json::json!({
                "functionCallingConfig": {"mode": "AUTO"}
            });
        }

        body
    }

    /// Parse a Gemini response JSON into an LLMResponse
    fn parse_response(&self, model: &str, json: Value) -> LLMResponse {
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut stop_reason = Some(StopReason::EndTurn);

        let empty_vec = vec![];
        let candidates = json["candidates"].as_array().unwrap_or(&empty_vec);

        if let Some(candidate) = candidates.first() {
            let finish_reason = candidate["finishReason"].as_str().unwrap_or("");
            stop_reason = match finish_reason {
                "STOP" => Some(StopReason::EndTurn),
                "MAX_TOKENS" => Some(StopReason::MaxTokens),
                "TOOL_CODE" | "TOOL_CALLS" => Some(StopReason::ToolUse),
                _ => Some(StopReason::EndTurn),
            };

            let empty_parts = vec![];
            let parts = candidate["content"]["parts"]
                .as_array()
                .unwrap_or(&empty_parts);
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    if !text.is_empty() {
                        content_blocks.push(ContentBlock::Text {
                            text: text.to_string(),
                        });
                    }
                } else if part["functionCall"].is_object() {
                    let fc = &part["functionCall"];
                    let name = fc["name"].as_str().unwrap_or("unknown").to_string();
                    let args = fc["args"].clone();
                    let id = format!("gemini-tc-{}", uuid::Uuid::new_v4().simple());
                    content_blocks.push(ContentBlock::ToolUse {
                        id,
                        name,
                        input: args,
                    });
                    stop_reason = Some(StopReason::ToolUse);
                }
            }
        }

        let usage_meta = &json["usageMetadata"];
        let input_tokens = usage_meta["promptTokenCount"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage_meta["candidatesTokenCount"].as_u64().unwrap_or(0) as u32;

        LLMResponse {
            id: format!("gemini-{}", uuid::Uuid::new_v4().simple()),
            model: model.to_string(),
            content: content_blocks,
            stop_reason,
            usage: TokenUsage {
                input_tokens,
                output_tokens,
            },
        }
    }

    /// Handle API error response
    async fn handle_error(&self, response: reqwest::Response) -> ProviderError {
        let status = response.status().as_u16();
        if let Ok(body) = response.json::<Value>().await {
            let message = body["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            if status == 429 {
                return ProviderError::RateLimitExceeded(message);
            }
            return ProviderError::ApiError {
                status,
                message,
                error_type: body["error"]["status"].as_str().map(|s| s.to_string()),
            };
        }
        ProviderError::ApiError {
            status,
            message: "Unknown error".to_string(),
            error_type: None,
        }
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        use super::retry::{RetryConfig, retry_with_backoff};

        let model = request.model.clone();
        let message_count = request.messages.len();
        tracing::info!(
            "Gemini API request: model={}, messages={}",
            model,
            message_count
        );

        let body = self.build_gemini_request(&request);
        let url = self.generate_url(&model, false);
        let retry_config = RetryConfig::default();

        let result = retry_with_backoff(
            || async {
                let response = self
                    .client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await?;

                let status = response.status();
                if !status.is_success() {
                    return Err(self.handle_error(response).await);
                }

                let json: Value = response.json().await?;
                let llm_response = self.parse_response(&model, json);

                tracing::info!(
                    "Gemini API response: input_tokens={}, output_tokens={}, stop_reason={:?}",
                    llm_response.usage.input_tokens,
                    llm_response.usage.output_tokens,
                    llm_response.stop_reason
                );

                Ok(llm_response)
            },
            &retry_config,
        )
        .await;

        if let Err(ref e) = result {
            tracing::error!("Gemini API request failed: {}", e);
        }

        result
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        use super::retry::{RetryConfig, retry_with_backoff};

        let model = request.model.clone();
        let message_count = request.messages.len();
        tracing::info!(
            "Gemini streaming request: model={}, messages={}",
            model,
            message_count
        );

        let body = self.build_gemini_request(&request);
        let url = self.generate_url(&model, true);
        let retry_config = RetryConfig::default();

        let response = retry_with_backoff(
            || async {
                let response = self
                    .client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await?;

                if !response.status().is_success() {
                    return Err(self.handle_error(response).await);
                }

                Ok(response)
            },
            &retry_config,
        )
        .await?;

        let model_clone = model.clone();
        let byte_stream = response.bytes_stream();
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

        // Track accumulated state for tool calls across chunks
        let state = std::sync::Arc::new(std::sync::Mutex::new(GeminiStreamState {
            emitted_message_start: false,
            accumulated_text: String::new(),
            tool_calls: std::collections::HashMap::new(),
            input_tokens: 0,
            output_tokens: 0,
        }));

        let event_stream = byte_stream
            .map(
                move |chunk_result| -> Vec<std::result::Result<StreamEvent, ProviderError>> {
                    match chunk_result {
                        Err(e) => vec![Err(ProviderError::StreamError(e.to_string()))],
                        Ok(chunk) => {
                            let text = String::from_utf8_lossy(&chunk);
                            let mut buf = buffer.lock().expect("SSE buffer lock");
                            buf.push_str(&text);

                            let mut events = Vec::new();
                            let mut st = state.lock().expect("SSE state lock");

                            while let Some(newline_pos) = buf.find('\n') {
                                let line = buf[..newline_pos].trim().to_string();
                                buf.drain(..=newline_pos);

                                let json_str = if let Some(s) = line.strip_prefix("data: ") {
                                    s
                                } else {
                                    continue;
                                };

                                if json_str == "[DONE]" {
                                    continue;
                                }

                                let json: Value = match serde_json::from_str(json_str) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        tracing::warn!(
                                            "Gemini: failed to parse SSE JSON: {} | data: {}",
                                            e,
                                            &json_str[..json_str.len().min(200)]
                                        );
                                        continue;
                                    }
                                };

                                // Emit MessageStart once
                                if !st.emitted_message_start {
                                    st.emitted_message_start = true;
                                    events.push(Ok(StreamEvent::MessageStart {
                                        message: StreamMessage {
                                            id: format!("gemini-{}", uuid::Uuid::new_v4().simple()),
                                            model: model_clone.clone(),
                                            role: Role::Assistant,
                                            usage: TokenUsage {
                                                input_tokens: 0,
                                                output_tokens: 0,
                                            },
                                        },
                                    }));
                                    events.push(Ok(StreamEvent::ContentBlockStart {
                                        index: 0,
                                        content_block: ContentBlock::Text {
                                            text: String::new(),
                                        },
                                    }));
                                }

                                let empty_candidates = vec![];
                                let candidates =
                                    json["candidates"].as_array().unwrap_or(&empty_candidates);

                                for candidate in candidates {
                                    let empty_parts = vec![];
                                    let parts = candidate["content"]["parts"]
                                        .as_array()
                                        .unwrap_or(&empty_parts);

                                    for part in parts {
                                        if let Some(text) = part["text"].as_str() {
                                            if !text.is_empty() {
                                                st.accumulated_text.push_str(text);
                                                events.push(Ok(StreamEvent::ContentBlockDelta {
                                                    index: 0,
                                                    delta: ContentDelta::TextDelta {
                                                        text: text.to_string(),
                                                    },
                                                }));
                                            }
                                        } else if part["functionCall"].is_object() {
                                            let fc = &part["functionCall"];
                                            let name =
                                                fc["name"].as_str().unwrap_or("").to_string();
                                            let args = fc["args"].clone();
                                            let id = format!(
                                                "gemini-tc-{}",
                                                uuid::Uuid::new_v4().simple()
                                            );
                                            let tool_idx = st.tool_calls.len();
                                            st.tool_calls.insert(
                                                tool_idx,
                                                GeminiToolCall {
                                                    id: id.clone(),
                                                    name: name.clone(),
                                                    args: args.clone(),
                                                },
                                            );

                                            // Emit tool use as a new content block
                                            events.push(Ok(StreamEvent::ContentBlockStop {
                                                index: 0,
                                            }));
                                            events.push(Ok(StreamEvent::ContentBlockStart {
                                                index: tool_idx + 1,
                                                content_block: ContentBlock::ToolUse {
                                                    id,
                                                    name,
                                                    input: args,
                                                },
                                            }));
                                            events.push(Ok(StreamEvent::ContentBlockStop {
                                                index: tool_idx + 1,
                                            }));
                                        }
                                    }
                                }

                                // Capture token usage from usageMetadata
                                if let Some(usage) = json["usageMetadata"].as_object() {
                                    if let Some(v) = usage.get("promptTokenCount") {
                                        st.input_tokens = v.as_u64().unwrap_or(0) as u32;
                                    }
                                    if let Some(v) = usage.get("candidatesTokenCount") {
                                        st.output_tokens = v.as_u64().unwrap_or(0) as u32;
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
                },
            )
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
        true
    }

    fn name(&self) -> &str {
        "gemini"
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "gemini-2.0-flash".to_string(),
            "gemini-3.1-flash-image-preview".to_string(),
            "gemini-1.5-pro".to_string(),
            "gemini-1.5-flash".to_string(),
        ]
    }

    async fn fetch_models(&self) -> Vec<String> {
        // Fetch live model list from Gemini models API
        let url = format!(
            "{}/models?key={}&pageSize=100",
            GEMINI_BASE_URL, self.api_key
        );

        #[derive(serde::Deserialize)]
        struct ModelEntry {
            name: String,
        }
        #[derive(serde::Deserialize)]
        struct ModelsResponse {
            models: Option<Vec<ModelEntry>>,
        }

        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<ModelsResponse>().await {
                    Ok(body) => {
                        let models: Vec<String> = body
                            .models
                            .unwrap_or_default()
                            .into_iter()
                            .map(|m| {
                                // Strip "models/" prefix from name like "models/gemini-2.0-flash"
                                m.name
                                    .strip_prefix("models/")
                                    .unwrap_or(&m.name)
                                    .to_string()
                            })
                            .filter(|m| {
                                // Only surfacing generative text/multimodal models
                                m.contains("gemini") || m.contains("gemma")
                            })
                            .collect();
                        if models.is_empty() {
                            self.supported_models()
                        } else {
                            models
                        }
                    }
                    Err(_) => self.supported_models(),
                }
            }
            _ => self.supported_models(),
        }
    }

    fn context_window(&self, model: &str) -> Option<u32> {
        match model {
            "gemini-2.0-flash" => Some(1_000_000),
            "gemini-3.1-flash-image-preview" => Some(1_000_000),
            "gemini-1.5-pro" => Some(2_000_000),
            "gemini-1.5-flash" => Some(1_000_000),
            _ => Some(1_000_000),
        }
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        // Gemini 2.0 Flash pricing: $0.075/M input, $0.30/M output (≤128k)
        match model {
            m if m.contains("flash") => {
                let input_cost = (input_tokens as f64 / 1_000_000.0) * 0.075;
                let output_cost = (output_tokens as f64 / 1_000_000.0) * 0.30;
                input_cost + output_cost
            }
            m if m.contains("pro") => {
                let input_cost = (input_tokens as f64 / 1_000_000.0) * 1.25;
                let output_cost = (output_tokens as f64 / 1_000_000.0) * 5.0;
                input_cost + output_cost
            }
            _ => {
                let input_cost = (input_tokens as f64 / 1_000_000.0) * 0.075;
                let output_cost = (output_tokens as f64 / 1_000_000.0) * 0.30;
                input_cost + output_cost
            }
        }
    }
}

/// Streaming state persisted across SSE chunks
struct GeminiStreamState {
    emitted_message_start: bool,
    accumulated_text: String,
    tool_calls: std::collections::HashMap<usize, GeminiToolCall>,
    input_tokens: u32,
    output_tokens: u32,
}

/// Accumulated tool call from streaming chunks
#[allow(dead_code)]
struct GeminiToolCall {
    id: String,
    name: String,
    args: Value,
}

/// Gemini-specific error response format
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GeminiError {
    error: GeminiErrorDetail,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GeminiErrorDetail {
    code: u32,
    message: String,
    status: String,
}

// Suppress "unused" warnings on Serialize for the request building helpers
#[allow(dead_code)]
#[derive(Serialize)]
struct GeminiPart {
    text: String,
}
