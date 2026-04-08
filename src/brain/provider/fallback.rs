//! Fallback Provider
//!
//! Wraps a primary provider with an ordered list of fallbacks.
//! When a provider returns a rate-limit (or other retryable) error, the
//! next provider in the chain is tried. After a successful fallback the
//! chosen provider becomes **sticky** — subsequent calls skip the dead
//! primary entirely until the process exits, so a single 429 doesn't
//! cost 60s of retries on every following turn.

use super::error::{ProviderError, Result};
use super::r#trait::{Provider, ProviderStream};
use super::types::{LLMRequest, LLMResponse};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Description of a swap that just occurred — consumed once by the
/// caller (typically the agent service) so it can surface a UI alert.
#[derive(Debug, Clone)]
pub struct SwapEvent {
    pub from_name: String,
    pub from_model: String,
    pub to_name: String,
    pub to_model: String,
    pub reason: String,
}

/// A provider that tries a chain of providers in order on failure.
///
/// `active` indexes into the chain: 0 = primary, 1..=fallbacks.len() = the
/// (n-1)-th fallback. After a successful swap, `active` advances and stays
/// there for the rest of the process — there is no automatic recovery
/// back to the original primary.
pub struct FallbackProvider {
    primary: Arc<dyn Provider>,
    fallbacks: Vec<Arc<dyn Provider>>,
    active: AtomicUsize,
    pending_swap: Mutex<Option<SwapEvent>>,
}

impl FallbackProvider {
    pub fn new(primary: Arc<dyn Provider>, fallbacks: Vec<Arc<dyn Provider>>) -> Self {
        Self {
            primary,
            fallbacks,
            active: AtomicUsize::new(0),
            pending_swap: Mutex::new(None),
        }
    }

    /// Get the currently-active provider (primary or a sticky fallback).
    fn active_provider(&self) -> Arc<dyn Provider> {
        let idx = self.active.load(Ordering::Acquire);
        if idx == 0 {
            self.primary.clone()
        } else {
            self.fallbacks[idx - 1].clone()
        }
    }

    /// Promote a fallback to active. Records a swap event for the caller
    /// to surface in the UI.
    fn promote(&self, new_idx: usize, reason: &str) {
        let old_idx = self.active.swap(new_idx, Ordering::AcqRel);
        if old_idx == new_idx {
            return;
        }
        let from = if old_idx == 0 {
            &self.primary
        } else {
            &self.fallbacks[old_idx - 1]
        };
        let to = if new_idx == 0 {
            &self.primary
        } else {
            &self.fallbacks[new_idx - 1]
        };
        let event = SwapEvent {
            from_name: from.name().to_string(),
            from_model: from.default_model().to_string(),
            to_name: to.name().to_string(),
            to_model: to.default_model().to_string(),
            reason: reason.to_string(),
        };
        tracing::warn!(
            "Sticky fallback: '{}/{}' → '{}/{}' (reason: {})",
            event.from_name,
            event.from_model,
            event.to_name,
            event.to_model,
            event.reason
        );
        if let Ok(mut slot) = self.pending_swap.lock() {
            *slot = Some(event);
        }
    }

    /// Build a request for a fallback provider, remapping the model if needed.
    fn remap_request_for_fallback(fb: &dyn Provider, request: &LLMRequest) -> LLMRequest {
        let mut fb_request = request.clone();
        let supported = fb.supported_models();
        if !supported.is_empty() && !supported.iter().any(|m| m == &fb_request.model) {
            let new_model = fb.default_model().to_string();
            tracing::info!(
                "Fallback '{}': model '{}' not supported — remapping to '{}'",
                fb.name(),
                fb_request.model,
                new_model
            );
            fb_request.model = new_model;
        }
        fb_request
    }

    /// Decide whether an error justifies trying the next provider in the
    /// chain. Rate-limit, transient HTTP errors and 5xx warrant a swap.
    /// Hard errors (auth, malformed request) surface directly — trying
    /// a different provider won't fix bad credentials.
    fn should_try_next(err: &ProviderError) -> bool {
        err.is_retryable()
    }
}

#[async_trait]
impl Provider for FallbackProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        let start_idx = self.active.load(Ordering::Acquire);
        let mut last_err: Option<ProviderError>;

        // Try the currently-active provider first
        let active = self.active_provider();
        let active_request = if start_idx == 0 {
            request.clone()
        } else {
            Self::remap_request_for_fallback(active.as_ref(), &request)
        };
        match active.complete(active_request).await {
            Ok(resp) => return Ok(resp),
            Err(e) if !Self::should_try_next(&e) => return Err(e),
            Err(e) => {
                tracing::warn!(
                    "Active provider '{}' failed: {} — trying next in chain",
                    active.name(),
                    e
                );
                last_err = Some(e);
            }
        }

        // Try subsequent fallbacks (skip ones already exhausted by the
        // sticky pointer — start_idx already accounts for them)
        for offset in start_idx..self.fallbacks.len() {
            let fb = &self.fallbacks[offset];
            let fb_request = Self::remap_request_for_fallback(fb.as_ref(), &request);
            match fb.complete(fb_request).await {
                Ok(resp) => {
                    self.promote(
                        offset + 1,
                        last_err
                            .as_ref()
                            .map(|e| format!("{}", e))
                            .unwrap_or_else(|| "unknown".into())
                            .as_str(),
                    );
                    return Ok(resp);
                }
                Err(e) => {
                    tracing::warn!("Fallback provider '{}' failed: {}", fb.name(), e);
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            ProviderError::Internal("FallbackProvider: all providers exhausted".into())
        }))
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        let start_idx = self.active.load(Ordering::Acquire);
        let mut last_err: Option<ProviderError>;

        // Try the currently-active provider first
        let active = self.active_provider();
        let active_request = if start_idx == 0 {
            request.clone()
        } else {
            Self::remap_request_for_fallback(active.as_ref(), &request)
        };
        match active.stream(active_request).await {
            Ok(stream) => return Ok(stream),
            Err(e) if !Self::should_try_next(&e) => return Err(e),
            Err(e) => {
                tracing::warn!(
                    "Active provider '{}' stream failed: {} — trying next in chain",
                    active.name(),
                    e
                );
                last_err = Some(e);
            }
        }

        // Try subsequent fallbacks
        for offset in start_idx..self.fallbacks.len() {
            let fb = &self.fallbacks[offset];
            let fb_request = Self::remap_request_for_fallback(fb.as_ref(), &request);
            match fb.stream(fb_request).await {
                Ok(stream) => {
                    self.promote(
                        offset + 1,
                        last_err
                            .as_ref()
                            .map(|e| format!("{}", e))
                            .unwrap_or_else(|| "unknown".into())
                            .as_str(),
                    );
                    return Ok(stream);
                }
                Err(e) => {
                    tracing::warn!("Fallback provider '{}' stream failed: {}", fb.name(), e);
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            ProviderError::Internal("FallbackProvider: all providers exhausted".into())
        }))
    }

    fn supports_streaming(&self) -> bool {
        self.primary.supports_streaming()
    }

    fn supports_tools(&self) -> bool {
        self.primary.supports_tools()
    }

    fn supports_vision(&self) -> bool {
        self.primary.supports_vision()
    }

    fn cli_handles_tools(&self) -> bool {
        self.primary.cli_handles_tools()
    }

    fn cli_manages_context(&self) -> bool {
        self.primary.cli_manages_context()
    }

    fn name(&self) -> &str {
        // Persistence and config-display name stays as the originally-configured
        // primary, even after a sticky swap. Use `active_subprovider_name()` for
        // the live indicator.
        self.primary.name()
    }

    fn default_model(&self) -> &str {
        self.primary.default_model()
    }

    fn supported_models(&self) -> Vec<String> {
        self.primary.supported_models()
    }

    async fn fetch_models(&self) -> Vec<String> {
        self.primary.fetch_models().await
    }

    fn context_window(&self, model: &str) -> Option<u32> {
        self.primary.context_window(model)
    }

    fn configured_context_window(&self) -> Option<u32> {
        self.primary.configured_context_window()
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        self.primary
            .calculate_cost(model, input_tokens, output_tokens)
    }

    fn take_swap_event(&self) -> Option<SwapEvent> {
        self.pending_swap.lock().ok().and_then(|mut s| s.take())
    }

    fn active_subprovider_name(&self) -> Option<String> {
        let idx = self.active.load(Ordering::Acquire);
        if idx == 0 {
            None
        } else {
            Some(self.fallbacks[idx - 1].name().to_string())
        }
    }

    fn active_subprovider_model(&self) -> Option<String> {
        let idx = self.active.load(Ordering::Acquire);
        if idx == 0 {
            None
        } else {
            Some(self.fallbacks[idx - 1].default_model().to_string())
        }
    }
}
