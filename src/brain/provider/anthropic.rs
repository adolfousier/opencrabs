//! Anthropic (Claude) Provider Implementation
//!
//! Implements the Provider trait for Anthropic's Claude models.
//! Supports both standard API key auth (`x-api-key`) and OAuth Bearer tokens
//! (detected via `sk-ant-oat` prefix).
//!
//! ## Supported Models
//! - claude-opus-4-6
//! - claude-sonnet-4-5-20250929
//! - claude-haiku-4-5-20251001
//! - claude-3-5-sonnet-20241022
//! - claude-3-5-haiku-20241022
//! - claude-3-opus-20240229 (legacy)
//! - claude-3-sonnet-20240229 (legacy)
//! - claude-3-haiku-20240307 (legacy)

use super::error::{ProviderError, Result};
use super::r#trait::{Provider, ProviderStream};
use super::types::*;
use async_trait::async_trait;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120); // Total request timeout
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10); // Connection timeout
const DEFAULT_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90); // Keep connections alive

/// Anthropic provider for Claude models
#[derive(Clone)]
pub struct AnthropicProvider {
    api_key: String,
    client: Client,
    custom_default_model: Option<String>,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT) // Total request timeout (including streaming)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT) // Connection establishment timeout
            .pool_idle_timeout(DEFAULT_POOL_IDLE_TIMEOUT) // Keep connections in pool
            .pool_max_idle_per_host(2) // Max idle connections per host
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key,
            client,
            custom_default_model: None,
        }
    }

    /// Create with custom HTTP client
    pub fn with_client(api_key: String, client: Client) -> Self {
        Self {
            api_key,
            client,
            custom_default_model: None,
        }
    }

    /// Set custom default model
    pub fn with_default_model(mut self, model: String) -> Self {
        self.custom_default_model = Some(model);
        self
    }

    /// Check if the API key is an OAuth token (starts with sk-ant-oat)
    fn is_oauth_token(&self) -> bool {
        self.api_key.starts_with("sk-ant-oat")
    }

    /// Build request headers
    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();

        if self.is_oauth_token() {
            // OAuth token: use Authorization: Bearer header
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.api_key)
                    .parse()
                    .expect("Invalid OAuth token format"),
            );
            // Required beta header for OAuth tokens
            headers.insert(
                "anthropic-beta",
                "oauth-2025-04-20".parse().expect("Invalid beta header"),
            );
        } else {
            // Standard API key: use x-api-key header
            headers.insert(
                "x-api-key",
                self.api_key.parse().expect("Invalid API key format"),
            );
        }

        headers.insert(
            "anthropic-version",
            ANTHROPIC_VERSION.parse().expect("Invalid version"),
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().expect("valid content-type"),
        );
        headers
    }

    /// Convert our generic request to Anthropic-specific format
    fn to_anthropic_request(&self, request: LLMRequest) -> AnthropicRequest {
        AnthropicRequest {
            model: request.model,
            messages: request.messages,
            system: request.system,
            max_tokens: request.max_tokens.unwrap_or(16384),
            temperature: request.temperature,
            tools: request.tools,
            stream: Some(request.stream),
            metadata: request.metadata,
        }
    }

    /// Convert Anthropic response to our generic format
    #[allow(clippy::wrong_self_convention)]
    fn from_anthropic_response(&self, response: AnthropicResponse) -> LLMResponse {
        LLMResponse {
            id: response.id,
            model: response.model,
            content: response.content,
            stop_reason: response.stop_reason,
            usage: response.usage,
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
        if let Ok(error_body) = response.json::<AnthropicError>().await {
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
                    error_type: Some(error_body.error.error_type),
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
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        use super::retry::{retry_with_backoff, RetryConfig};

        let model = request.model.clone();
        let message_count = request.messages.len();
        tracing::info!(
            "Anthropic API request: model={}, messages={}, max_tokens={}",
            model,
            message_count,
            request.max_tokens.unwrap_or(16384)
        );

        let anthropic_request = self.to_anthropic_request(request);
        let retry_config = RetryConfig::default();

        // Retry the entire API call with exponential backoff
        let result = retry_with_backoff(
            || async {
                tracing::debug!("Sending request to Anthropic API");
                let response = self
                    .client
                    .post(ANTHROPIC_API_URL)
                    .headers(self.headers())
                    .json(&anthropic_request)
                    .send()
                    .await?;

                let status = response.status();
                tracing::debug!("Anthropic API response status: {}", status);

                if !status.is_success() {
                    return Err(self.handle_error(response).await);
                }

                let anthropic_response: AnthropicResponse = response.json().await?;
                let llm_response = self.from_anthropic_response(anthropic_response);

                tracing::info!(
                    "Anthropic API response: input_tokens={}, output_tokens={}, stop_reason={:?}",
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
            tracing::error!("Anthropic API request failed: {}", e);
        }

        result
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        use super::retry::{retry_with_backoff, RetryConfig};

        let model = request.model.clone();
        let message_count = request.messages.len();
        tracing::info!(
            "Anthropic streaming request: model={}, messages={}",
            model,
            message_count
        );

        let mut anthropic_request = self.to_anthropic_request(request);
        anthropic_request.stream = Some(true);
        let retry_config = RetryConfig::default();

        // Retry the stream connection establishment
        let response = retry_with_backoff(
            || async {
                let response = self
                    .client
                    .post(ANTHROPIC_API_URL)
                    .headers(self.headers())
                    .json(&anthropic_request)
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

        // Parse Server-Sent Events stream with cross-chunk buffering.
        // TCP chunks can split SSE events, so we buffer partial lines.
        let byte_stream = response.bytes_stream();
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

        let event_stream = byte_stream
            .map(
                move |chunk_result| -> Vec<std::result::Result<StreamEvent, ProviderError>> {
                    match chunk_result {
                        Err(e) => vec![Err(ProviderError::StreamError(e.to_string()))],
                        Ok(chunk) => {
                            let text = String::from_utf8_lossy(&chunk);
                            let mut buf = buffer.lock().expect("SSE buffer lock poisoned");
                            buf.push_str(&text);

                            let mut events = Vec::new();

                            // Process complete lines (terminated by \n)
                            while let Some(newline_pos) = buf.find('\n') {
                                let line = buf[..newline_pos].trim().to_string();
                                buf.drain(..=newline_pos);

                                if let Some(json_str) = line.strip_prefix("data: ") {
                                    if json_str == "[DONE]" {
                                        continue;
                                    }
                                    match serde_json::from_str::<StreamEvent>(json_str) {
                                        Ok(event) => events.push(Ok(event)),
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to parse SSE event JSON: {}. Data: {}",
                                                e,
                                                json_str.chars().take(200).collect::<String>()
                                            );
                                            // Don't propagate parse errors for individual events
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
        "anthropic"
    }

    fn default_model(&self) -> &str {
        self.custom_default_model
            .as_deref()
            .unwrap_or("claude-sonnet-4-5")
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            // Claude 4.x models
            "claude-opus-4-6".to_string(),
            "claude-sonnet-4-5-20250929".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
            // Claude 3.5 models
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            // Claude 3 models (legacy)
            "claude-3-opus-20240229".to_string(),
            "claude-3-sonnet-20240229".to_string(),
            "claude-3-5-sonnet-20240620".to_string(),
            "claude-3-haiku-20240307".to_string(),
        ]
    }

    async fn fetch_models(&self) -> Vec<String> {
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }
        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }

        let mut req = self
            .client
            .get(ANTHROPIC_MODELS_URL)
            .header("anthropic-version", ANTHROPIC_VERSION);

        if self.api_key.starts_with("sk-ant-oat") {
            req = req
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("anthropic-beta", "oauth-2025-04-20");
        } else {
            req = req.header("x-api-key", &self.api_key);
        }

        match req.send().await {
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

    fn context_window(&self, model: &str) -> Option<u32> {
        match model {
            "claude-opus-4-6" => Some(200_000),
            "claude-sonnet-4-5-20250929" => Some(200_000),
            "claude-haiku-4-5-20251001" => Some(200_000),
            "claude-3-5-sonnet-20241022" => Some(200_000),
            "claude-3-5-haiku-20241022" => Some(200_000),
            "claude-3-opus-20240229" => Some(200_000),
            "claude-3-sonnet-20240229" => Some(200_000),
            "claude-3-5-sonnet-20240620" => Some(200_000),
            "claude-3-haiku-20240307" => Some(200_000),
            _ => None,
        }
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        // Costs per million tokens (verified via OpenRouter API 2026-02-25)
        // Pattern-match on prefix so future point-release suffixes still match
        let model_lower = model.to_lowercase();
        let (input_cost, output_cost) = if model_lower.contains("claude-opus-4") {
            // claude-opus-4-6, claude-opus-4.6, claude-opus-4.5, claude-opus-4 — $5/$25
            (5.0_f64, 25.0_f64)
        } else if model_lower.contains("claude-opus-3") || model == "claude-3-opus-20240229" {
            // Legacy claude-3-opus — $15/$75
            (15.0, 75.0)
        } else if model_lower.contains("claude-sonnet-4")
            || model == "claude-3-7-sonnet-20250219"
            || model == "claude-sonnet-4-5-20250929"
            || model == "claude-3-5-sonnet-20241022"
            || model == "claude-3-5-sonnet-20240620"
            || model == "claude-3-sonnet-20240229"
        {
            // All sonnet-4.x and claude-3.5-sonnet — $3/$15
            (3.0, 15.0)
        } else if model_lower.contains("claude-haiku-4") || model == "claude-haiku-4-5-20251001" {
            // claude-haiku-4.5+ — $1/$5
            (1.0, 5.0)
        } else if model == "claude-3-5-haiku-20241022" {
            (0.80, 4.0)
        } else if model == "claude-3-haiku-20240307" {
            (0.25, 1.25)
        } else {
            return 0.0;
        };

        let input_cost_total = (input_tokens as f64 / 1_000_000.0) * input_cost;
        let output_cost_total = (output_tokens as f64 / 1_000_000.0) * output_cost;

        input_cost_total + output_cost_total
    }
}

// Anthropic-specific request format
#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<std::collections::HashMap<String, String>>,
}

// Anthropic-specific response format
#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<ContentBlock>,
    stop_reason: Option<StopReason>,
    usage: TokenUsage,
}

// Anthropic error format
#[derive(Debug, Deserialize)]
struct AnthropicError {
    error: AnthropicErrorDetail,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_provider_creation() {
        let provider = AnthropicProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.default_model(), "claude-sonnet-4-5");
    }

    #[test]
    fn test_custom_default_model() {
        let provider = AnthropicProvider::new("test-key".to_string())
            .with_default_model("claude-opus-4-6".to_string());
        assert_eq!(provider.default_model(), "claude-opus-4-6");
    }

    #[test]
    fn test_supported_models() {
        let provider = AnthropicProvider::new("test-key".to_string());
        let models = provider.supported_models();
        assert!(models.contains(&"claude-opus-4-6".to_string()));
        assert!(models.contains(&"claude-sonnet-4-5-20250929".to_string()));
        assert!(models.contains(&"claude-haiku-4-5-20251001".to_string()));
        // Legacy models still present
        assert!(models.contains(&"claude-3-opus-20240229".to_string()));
    }

    #[test]
    fn test_context_window() {
        let provider = AnthropicProvider::new("test-key".to_string());
        assert_eq!(provider.context_window("claude-opus-4-6"), Some(200_000));
        assert_eq!(
            provider.context_window("claude-3-opus-20240229"),
            Some(200_000)
        );
        assert_eq!(provider.context_window("unknown-model"), None);
    }

    #[test]
    fn test_cost_calculation() {
        let provider = AnthropicProvider::new("test-key".to_string());

        // Test Opus 4 pricing (corrected: $5/$25 per OpenRouter 2026-02-25)
        let cost = provider.calculate_cost("claude-opus-4-6", 1_000_000, 1_000_000);
        assert_eq!(cost, 30.0); // $5 input + $25 output

        // Test Sonnet 4.6 pricing (was missing — main model)
        let cost = provider.calculate_cost("claude-sonnet-4-6", 1_000_000, 1_000_000);
        assert_eq!(cost, 18.0); // $3 input + $15 output

        // Test legacy Opus 3 pricing ($15/$75)
        let cost = provider.calculate_cost("claude-3-opus-20240229", 1_000_000, 1_000_000);
        assert_eq!(cost, 90.0);

        // Test Haiku 4.5 pricing ($1/$5)
        let cost = provider.calculate_cost("claude-haiku-4-5-20251001", 1_000_000, 1_000_000);
        assert_eq!(cost, 6.0); // $1 input + $5 output

        // Test legacy Haiku 3.5 pricing
        let cost = provider.calculate_cost("claude-3-5-haiku-20241022", 1_000_000, 1_000_000);
        assert_eq!(cost, 4.8); // $0.80 input + $4.0 output

        // Test legacy Haiku pricing
        let cost = provider.calculate_cost("claude-3-haiku-20240307", 1_000_000, 1_000_000);
        assert_eq!(cost, 1.5); // $0.25 input + $1.25 output
    }

    #[test]
    fn test_oauth_token_detection() {
        let standard = AnthropicProvider::new("sk-ant-api-key".to_string());
        assert!(!standard.is_oauth_token());

        let oauth = AnthropicProvider::new("sk-ant-oat01-something".to_string());
        assert!(oauth.is_oauth_token());
    }

    #[test]
    fn test_oauth_headers() {
        let provider = AnthropicProvider::new("sk-ant-oat01-test-token".to_string());
        let headers = provider.headers();
        assert!(headers.contains_key(reqwest::header::AUTHORIZATION));
        assert!(headers.contains_key("anthropic-beta"));
        assert!(!headers.contains_key("x-api-key"));
    }

    #[test]
    fn test_standard_headers() {
        let provider = AnthropicProvider::new("sk-ant-api-key".to_string());
        let headers = provider.headers();
        assert!(headers.contains_key("x-api-key"));
        assert!(!headers.contains_key(reqwest::header::AUTHORIZATION));
        assert!(!headers.contains_key("anthropic-beta"));
    }

    #[test]
    fn test_capabilities() {
        let provider = AnthropicProvider::new("test-key".to_string());
        assert!(provider.supports_streaming());
        assert!(provider.supports_tools());
        assert!(provider.supports_vision());
    }
}
