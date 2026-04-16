//! Rotating Qwen Provider
//!
//! Wraps N `OpenAIProvider` instances (one per Qwen OAuth account) and
//! round-robins between them on rate-limit errors. Only when ALL accounts
//! are exhausted does the error propagate, letting the outer
//! `FallbackProvider` fall to a different provider (e.g. Anthropic).
//!
//! Unlike `FallbackProvider` (which is sticky — once promoted, never goes
//! back), this rotates continuously: account 0 → 1 → 2 → 0 → …

use super::error::{ProviderError, Result};
use super::fallback::SwapEvent;
use super::r#trait::{Provider, ProviderStream};
use super::types::{LLMRequest, LLMResponse};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub struct RotatingQwenProvider {
    accounts: Vec<Arc<dyn Provider>>,
    active: AtomicUsize,
    pending_swap: Mutex<Option<SwapEvent>>,
}

impl RotatingQwenProvider {
    pub fn new(accounts: Vec<Arc<dyn Provider>>) -> Self {
        assert!(
            !accounts.is_empty(),
            "RotatingQwenProvider needs ≥1 account"
        );
        Self {
            accounts,
            active: AtomicUsize::new(0),
            pending_swap: Mutex::new(None),
        }
    }

    fn advance(&self, from: usize) -> usize {
        let next = (from + 1) % self.accounts.len();
        self.active.store(next, Ordering::Release);
        next
    }

    fn record_swap(&self, from_idx: usize, to_idx: usize, reason: &str) {
        let from = &self.accounts[from_idx];
        let to = &self.accounts[to_idx];
        let event = SwapEvent {
            from_name: format!("qwen-account-{}", from_idx),
            from_model: from.default_model().to_string(),
            to_name: format!("qwen-account-{}", to_idx),
            to_model: to.default_model().to_string(),
            reason: reason.to_string(),
        };
        tracing::warn!(
            "Qwen rotation: account {} → {} (reason: {})",
            from_idx,
            to_idx,
            reason
        );
        if let Ok(mut slot) = self.pending_swap.lock() {
            *slot = Some(event);
        }
    }

    fn should_rotate(err: &ProviderError) -> bool {
        if err.is_retryable() {
            return true;
        }
        // Auth errors (401/403) after the per-account refresh hook already
        // failed — the OAuth token is dead. Rotate to the next account.
        matches!(
            err,
            ProviderError::ApiError {
                status: 401 | 403,
                ..
            }
        )
    }
}

#[async_trait]
impl Provider for RotatingQwenProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        let start = self.active.load(Ordering::Acquire);
        let n = self.accounts.len();
        let mut last_err: Option<ProviderError> = None;

        for i in 0..n {
            let idx = (start + i) % n;
            let provider = &self.accounts[idx];
            match provider.complete(request.clone()).await {
                Ok(resp) => {
                    if i > 0 {
                        self.active.store(idx, Ordering::Release);
                        self.record_swap(
                            start,
                            idx,
                            last_err
                                .as_ref()
                                .map(|e| format!("{}", e))
                                .unwrap_or_else(|| "rate_limit".into())
                                .as_str(),
                        );
                    }
                    return Ok(resp);
                }
                Err(e) if !Self::should_rotate(&e) => return Err(e),
                Err(e) => {
                    tracing::warn!("Qwen account {} failed: {} — rotating", idx, e);
                    last_err = Some(e);
                }
            }
        }

        // All accounts exhausted — advance pointer so next call starts
        // from the next account (spread the load).
        self.advance(start);

        Err(last_err.unwrap_or_else(|| {
            ProviderError::Internal("RotatingQwenProvider: all accounts exhausted".into())
        }))
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        let start = self.active.load(Ordering::Acquire);
        let n = self.accounts.len();
        let mut last_err: Option<ProviderError> = None;

        for i in 0..n {
            let idx = (start + i) % n;
            let provider = &self.accounts[idx];
            match provider.stream(request.clone()).await {
                Ok(stream) => {
                    if i > 0 {
                        self.active.store(idx, Ordering::Release);
                        self.record_swap(
                            start,
                            idx,
                            last_err
                                .as_ref()
                                .map(|e| format!("{}", e))
                                .unwrap_or_else(|| "rate_limit".into())
                                .as_str(),
                        );
                    }
                    return Ok(stream);
                }
                Err(e) if !Self::should_rotate(&e) => return Err(e),
                Err(e) => {
                    tracing::warn!("Qwen account {} stream failed: {} — rotating", idx, e);
                    last_err = Some(e);
                }
            }
        }

        self.advance(start);

        Err(last_err.unwrap_or_else(|| {
            ProviderError::Internal("RotatingQwenProvider: all accounts exhausted".into())
        }))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
        false
    }

    fn name(&self) -> &str {
        "qwen"
    }

    fn default_model(&self) -> &str {
        self.accounts[0].default_model()
    }

    fn supported_models(&self) -> Vec<String> {
        self.accounts[0].supported_models()
    }

    async fn fetch_models(&self) -> Vec<String> {
        let idx = self.active.load(Ordering::Acquire);
        self.accounts[idx].fetch_models().await
    }

    fn context_window(&self, model: &str) -> Option<u32> {
        self.accounts[0].context_window(model)
    }

    fn configured_context_window(&self) -> Option<u32> {
        self.accounts[0].configured_context_window()
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        self.accounts[0].calculate_cost(model, input_tokens, output_tokens)
    }

    fn take_swap_event(&self) -> Option<SwapEvent> {
        self.pending_swap.lock().ok().and_then(|mut s| s.take())
    }

    fn active_subprovider_name(&self) -> Option<String> {
        let idx = self.active.load(Ordering::Acquire);
        Some(format!("qwen-account-{}", idx))
    }

    fn active_subprovider_model(&self) -> Option<String> {
        let idx = self.active.load(Ordering::Acquire);
        Some(self.accounts[idx].default_model().to_string())
    }
}
