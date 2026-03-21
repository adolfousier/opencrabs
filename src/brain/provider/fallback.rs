//! Fallback Provider
//!
//! Wraps a primary provider with an ordered list of fallbacks.
//! When the primary fails, each fallback is tried in sequence.

use super::error::Result;
use super::r#trait::{Provider, ProviderStream};
use super::types::{LLMRequest, LLMResponse};
use async_trait::async_trait;
use std::sync::Arc;

/// A provider that tries a chain of providers in order on failure.
pub struct FallbackProvider {
    primary: Arc<dyn Provider>,
    fallbacks: Vec<Arc<dyn Provider>>,
}

impl FallbackProvider {
    pub fn new(primary: Arc<dyn Provider>, fallbacks: Vec<Arc<dyn Provider>>) -> Self {
        Self { primary, fallbacks }
    }
}

#[async_trait]
impl Provider for FallbackProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        match self.primary.complete(request.clone()).await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                tracing::warn!(
                    "Primary provider '{}' failed: {} — trying fallbacks",
                    self.primary.name(),
                    e
                );
                for fb in &self.fallbacks {
                    match fb.complete(request.clone()).await {
                        Ok(resp) => {
                            tracing::info!("Fallback provider '{}' succeeded", fb.name());
                            return Ok(resp);
                        }
                        Err(e) => {
                            tracing::warn!("Fallback provider '{}' failed: {}", fb.name(), e);
                        }
                    }
                }
                Err(e)
            }
        }
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        match self.primary.stream(request.clone()).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                tracing::warn!(
                    "Primary provider '{}' stream failed: {} — trying fallbacks",
                    self.primary.name(),
                    e
                );
                for fb in &self.fallbacks {
                    match fb.stream(request.clone()).await {
                        Ok(stream) => {
                            tracing::info!("Fallback provider '{}' stream succeeded", fb.name());
                            return Ok(stream);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Fallback provider '{}' stream failed: {}",
                                fb.name(),
                                e
                            );
                        }
                    }
                }
                Err(e)
            }
        }
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

    fn name(&self) -> &str {
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

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        self.primary
            .calculate_cost(model, input_tokens, output_tokens)
    }
}
