//! Provider trait definition
//!
//! Defines the interface that all LLM providers must implement.

use super::error::Result;
use super::types::{LLMRequest, LLMResponse, StreamEvent};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

/// Stream type for provider responses
pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

/// LLM Provider trait
///
/// All LLM providers (Anthropic, OpenAI, Gemini, etc.) implement this trait
/// to provide a uniform interface for the agent service.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a completion request and get the full response
    ///
    /// This is a non-streaming request that waits for the complete response.
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse>;

    /// Send a streaming completion request
    ///
    /// Returns a stream of events that can be consumed incrementally.
    /// Not all providers support streaming.
    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream>;

    /// Check if this provider supports streaming responses
    fn supports_streaming(&self) -> bool {
        true // Most modern providers support streaming
    }

    /// Check if this provider supports tool/function calling
    fn supports_tools(&self) -> bool {
        true // Most modern providers support tools
    }

    /// Check if this provider supports vision/image inputs
    fn supports_vision(&self) -> bool {
        false // Not all providers support vision
    }

    /// Get the provider name
    fn name(&self) -> &str;

    /// Get the default model for this provider
    fn default_model(&self) -> &str;

    /// Get supported models (hardcoded fallback list)
    fn supported_models(&self) -> Vec<String>;

    /// Fetch available models from the provider API.
    /// Falls back to the hardcoded `supported_models()` list on error.
    async fn fetch_models(&self) -> Vec<String> {
        self.supported_models()
    }

    /// Validate that a model is supported
    fn validate_model(&self, model: &str) -> bool {
        self.supported_models().iter().any(|m| m == model)
    }

    /// Get context window size for a model
    fn context_window(&self, model: &str) -> Option<u32>;

    /// Calculate cost for token usage (in USD)
    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64;
}

/// Provider capabilities
#[derive(Debug, Clone, Copy)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tools: bool,
    pub vision: bool,
    pub json_mode: bool,
}

impl ProviderCapabilities {
    /// Get capabilities for a provider
    pub fn for_provider(provider: &dyn Provider) -> Self {
        Self {
            streaming: provider.supports_streaming(),
            tools: provider.supports_tools(),
            vision: provider.supports_vision(),
            json_mode: false, // Provider-specific
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock provider for testing
    struct MockProvider;

    #[async_trait]
    impl Provider for MockProvider {
        async fn complete(&self, _request: LLMRequest) -> Result<LLMResponse> {
            unimplemented!("Mock provider")
        }

        async fn stream(&self, _request: LLMRequest) -> Result<ProviderStream> {
            unimplemented!("Mock provider")
        }

        fn name(&self) -> &str {
            "mock"
        }

        fn default_model(&self) -> &str {
            "mock-model-1"
        }

        fn supported_models(&self) -> Vec<String> {
            vec!["mock-model-1".to_string(), "mock-model-2".to_string()]
        }

        fn context_window(&self, _model: &str) -> Option<u32> {
            Some(4096)
        }

        fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
            0.0
        }
    }

    #[test]
    fn test_provider_validate_model() {
        let provider = MockProvider;
        assert!(provider.validate_model("mock-model-1"));
        assert!(provider.validate_model("mock-model-2"));
        assert!(!provider.validate_model("unknown-model"));
    }

    #[test]
    fn test_provider_capabilities() {
        let provider = MockProvider;
        let caps = ProviderCapabilities::for_provider(&provider);
        assert!(caps.streaming);
        assert!(caps.tools);
        assert!(!caps.vision);
    }
}
