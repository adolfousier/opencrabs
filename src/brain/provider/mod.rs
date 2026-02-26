//! LLM Provider Abstraction Layer
//!
//! Provides a unified interface for interacting with different LLM providers.

pub mod error;
pub mod placeholder;
pub mod retry;
#[allow(clippy::module_inception)]
mod r#trait;
pub mod types;

// Re-exports
pub use error::{ProviderError, Result};
pub use placeholder::PlaceholderProvider;
pub use r#trait::{Provider, ProviderCapabilities, ProviderStream};
pub use types::*;

// Provider implementations
pub mod anthropic;
pub mod custom_openai_compatible;
pub mod factory;

pub use anthropic::AnthropicProvider;
pub use custom_openai_compatible::OpenAIProvider;
pub use factory::create_provider;
