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

    /// User-configured or provider-specific HTTP request timeout.
    /// Returns `None` when default client timeouts apply.
    fn request_timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// User-configured or provider-specific inter-chunk streaming inactivity timeout.
    /// Returns `None` when standard default idle timeouts apply (20s remote, 3600s local/CLI).
    fn stream_idle_timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// Force the fallback wrapper to advance to the next provider.
    /// Used by the tool loop when stream drops exhaust retries — the
    /// stream started OK so `FallbackProvider::stream()` never saw an error,
    /// but the response was empty/broken. Returns `true` if a fallback was
    /// promoted, `false` if there are no more fallbacks or the provider
    /// isn't a fallback wrapper.
    ///
    /// `current_model` is the model the session was actually running, so the
    /// swap the caller surfaces names the pair the user was on rather than the
    /// provider's default (#918). There is no request in flight here, so the
    /// promoted provider's own default is the best available answer for the
    /// model it moves TO.
    fn force_next_fallback(&self, _reason: &str, _current_model: &str) -> bool {
        false
    }

    /// Take any pending swap event from a sticky fallback wrapper.
    /// Returns `None` for non-fallback providers; only `FallbackProvider`
    /// implements this. Called once per turn by the agent service so it
    /// can surface a UI alert when the active sub-provider changes.
    fn take_swap_event(&self) -> Option<super::fallback::SwapEvent> {
        None
    }

    /// Drain any retry notices recorded since the last call. Each entry is
    /// `(attempt, max, reason)` for an in-place retry the provider performed
    /// (connection blip, 5xx, rate limit). The agent service polls this once
    /// per stream attempt and surfaces each as a `RetryAttempt` UI event so
    /// the user sees the resilience working ("⏳ Retry 2/4 …"). Default
    /// empty — only providers that retry internally override it.
    fn take_retry_notices(&self) -> Vec<(u32, u32, String)> {
        Vec::new()
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

    /// One-line provenance label for per-call logging: which entry is
    /// currently serving calls. Chain wrappers report `primary 'name'`
    /// or `fallback #N 'name'`; plain providers report `'name'`.
    /// Logging/diagnostics only — never used for routing decisions.
    fn provenance_label(&self) -> String {
        format!("'{}'", self.name())
    }

    /// The model the currently-active entry actually ran for `requested`.
    ///
    /// A chain that advanced to an entry which does not carry the requested
    /// model remaps to that entry's default, so the requested name is not
    /// what executed. Logging that name produced a "served" line naming a
    /// model that never ran, which is precisely the question an incident
    /// asks the log (#1254). Diagnostics only, never routing.
    fn served_model(&self, requested: &str) -> String {
        requested.to_string()
    }

    /// True when this provider is itself a chain wrapper (i.e.
    /// `FallbackProvider`). Callers that want to install a provider as
    /// the session's active backend use this to decide whether to wrap
    /// the new provider in a fallback chain themselves. Without the
    /// check, `swap_provider_for_session` would either (a) skip
    /// wrapping and lose cascade coverage entirely — the regression
    /// captured in logs 2026-06-02 02:33:25 where five "fallbacks"
    /// cascaded in ~4s because the session's per-`/models` swap had
    /// stripped the FallbackProvider wrapper — or (b) double-wrap
    /// every re-swap and grow a nested onion of FallbackProviders.
    fn is_fallback_chain(&self) -> bool {
        false
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
        let cost = match crate::usage::pricing::PricingConfig::load() {
            Ok(cfg) => cfg.calculate_cost_with_cache(
                model,
                input_tokens,
                output_tokens,
                cache_creation_tokens,
                cache_read_tokens,
            ),
            Err(e) => {
                tracing::warn!("Pricing config load failed: {}", e);
                0.0
            }
        };
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
