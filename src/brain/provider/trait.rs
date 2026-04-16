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

    /// Whether the CLI subprocess handles tool execution internally.
    /// When true, the tool_loop emits ToolStarted/ToolCompleted progress
    /// events for display but does NOT execute tools itself.
    fn cli_handles_tools(&self) -> bool {
        false
    }

    /// Whether the CLI subprocess persists conversation context across
    /// invocations on its own (e.g. claude-cli's `--session-id`/`--resume`).
    ///
    /// When `true`, the agent service skips its own context-budget
    /// enforcement and trusts the CLI to compact/manage history. When
    /// `false`, OpenCrabs sends the full message history on every spawn
    /// and MUST run its own compaction to stay within the model's window.
    ///
    /// Defaults to `cli_handles_tools()` for backwards compatibility.
    fn cli_manages_context(&self) -> bool {
        self.cli_handles_tools()
    }

    /// Get the provider name
    fn name(&self) -> &str;

    /// Base URL this provider sends requests to. Returns `None` for
    /// providers that use hardcoded endpoints; `Some(url)` for custom
    /// OpenAI-compatible providers where the user configured the host.
    /// Used to identify specific proxies by URL regardless of what the
    /// user named them in config.
    fn base_url(&self) -> Option<&str> {
        None
    }

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

    /// User-configured context window override from `config.toml`
    /// (`providers.<name>.context_window`). Only custom OpenAI-compatible
    /// providers expose this; everything else inherits the agent's static
    /// `context_limit`. When set, this becomes the compaction budget.
    fn configured_context_window(&self) -> Option<u32> {
        None
    }

    /// Force the fallback wrapper to advance to the next provider.
    /// Used by the tool loop when stream drops exhaust retries — the
    /// stream started OK so `FallbackProvider::stream()` never saw an error,
    /// but the response was empty/broken. Returns `true` if a fallback was
    /// promoted, `false` if there are no more fallbacks or the provider
    /// isn't a fallback wrapper.
    fn force_next_fallback(&self, _reason: &str) -> bool {
        false
    }

    /// Take any pending swap event from a sticky fallback wrapper.
    /// Returns `None` for non-fallback providers; only `FallbackProvider`
    /// implements this. Called once per turn by the agent service so it
    /// can surface a UI alert when the active sub-provider changes.
    fn take_swap_event(&self) -> Option<super::fallback::SwapEvent> {
        None
    }

    /// Name of the currently-active sub-provider when wrapped in a sticky
    /// fallback. Returns `None` when on the primary (or for non-fallback
    /// providers). Used by the footer/splash to show the live target.
    fn active_subprovider_name(&self) -> Option<String> {
        None
    }

    /// Default model of the currently-active sub-provider, paired with
    /// `active_subprovider_name()`.
    fn active_subprovider_model(&self) -> Option<String> {
        None
    }

    /// Calculate cost for token usage (in USD)
    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64;

    /// Calculate cost with full cache token breakdown.
    /// Default: tries PricingConfig for cache-aware pricing, falls back to
    /// `calculate_cost` with all input tokens at the regular rate.
    fn calculate_cost_with_cache(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_creation_tokens: u32,
        cache_read_tokens: u32,
    ) -> f64 {
        let cost = crate::pricing::PricingConfig::load().calculate_cost_with_cache(
            model,
            input_tokens,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
        );
        if cost > 0.0 {
            cost
        } else {
            // Fallback: no pricing entry matched — use provider's own rate
            // treating all tokens (including cache) at the regular input rate.
            let total_input = input_tokens + cache_creation_tokens + cache_read_tokens;
            self.calculate_cost(model, total_input, output_tokens)
        }
    }
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
