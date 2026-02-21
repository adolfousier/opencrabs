# Adding New Providers

This document describes how to add a new AI provider to OpenCrabs.

## Overview

OpenCrabs supports multiple AI providers through a pluggable architecture. Each provider is configured via `config.toml` and appears in the onboarding UI.

## Provider Architecture

### 1. Config (`src/config/types.rs`)

Add provider config under `ProviderConfigs`:

```rust
pub struct ProviderConfigs {
    pub minimax: Option<ProviderConfig>,   // NEW
    // ... existing providers
}

pub struct ProviderConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}
```

### 2. Provider Factory (`src/brain/provider/factory.rs`)

Add provider creation logic in priority order:

```rust
pub fn create_provider(config: &Config) -> Result<Arc<dyn Provider>> {
    // Check which providers are enabled in config.toml
    
    // NEW: Add your provider
    if config.providers.yourprovider.as_ref().is_some_and(|p| p.enabled) {
        tracing::info!("Using enabled provider: YourProvider");
        return try_create_yourprovider(config)?
            .ok_or_else(|| anyhow::anyhow!("YourProvider enabled but failed"));
    }
    
    // ... existing providers
}
```

Add helper function:
```rust
fn try_create_yourprovider(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let cfg = match &config.providers.yourprovider {
        Some(c) => c,
        None => return Ok(None),
    };
    
    let api_key = match &cfg.api_key {
        Some(k) => k.clone(),
        None => return Ok(None),
    };
    
    let base_url = cfg.base_url.clone()
        .unwrap_or_else(|| "https://api.yourprovider.com/v1".to_string());
    
    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key, base_url),
        cfg,
    );
    Ok(Some(Arc::new(provider)))
}
```

### 3. Onboarding UI (`src/tui/onboarding.rs`)

Add provider to `PROVIDERS` array (index order matters!):

```rust
pub const PROVIDERS: &[ProviderInfo] = &[
    // ... existing
    ProviderInfo {
        name: "YourProvider",
        env_vars: &["YOURPROVIDER_API_KEY"],
        keyring_key: "yourprovider_api_key",
        models: &[],  // Always empty - fetched from API
        key_label: "API Key",
        help_lines: &["Get key from yourprovider.com"],
    },
];
```

Update `supports_model_fetch()` if provider supports model fetching:
```rust
pub fn supports_model_fetch(&self) -> bool {
    matches!(self.selected_provider, 0 | 1 | 3 | 4 | 5) // Add your index
}
```

### 4. Provider Display (`src/config/types.rs`)

Add to `resolve_provider_from_config()`:
```rust
pub fn resolve_provider_from_config(config: &Config) -> (&str, &str) {
    if config.providers.yourprovider.as_ref().is_some_and(|p| p.enabled) {
        let model = config.providers.yourprovider.as_ref()
            .and_then(|p| p.default_model.as_deref())
            .unwrap_or("default");
        return ("YourProvider", model);
    }
    // ...
}
```

### 5. Onboarding Save Logic (`src/tui/onboarding.rs`)

In `save_to_config()`:
```rust
match self.selected_provider {
    0 => { /* Anthropic */ }
    // ...
    N => {  // Your provider index
        config.providers.yourprovider = Some(ProviderConfig {
            enabled: true,
            api_key: Some(self.api_key_input.clone()),
            base_url: Some("https://api.yourprovider.com/v1".to_string()),
            default_model: Some(model),
        });
    }
}
```

### 6. Model Fetching

Providers are categorized by how they get their model list:

#### API Fetch (OpenAI, OpenRouter, Anthropic)
These providers have `/models` endpoints - models are fetched automatically:
- Base URL is modified: `base_url.replace("/chat/completions", "/models")`
- No need to save models in config

#### Config-Based (Minimax, Custom, etc.)
These providers DO NOT have `/models` endpoints:
- Add `models: Vec<String>` to config.toml
- API key goes in keys.toml (chmod 600)
- Example config.toml for Minimax:
```toml
[providers.minimax]
enabled = true
base_url = "https://api.minimax.io/v1"
default_model = "MiniMax-M2.5"
models = ["MiniMax-M2.5", "MiniMax-M2.1", "MiniMax-Text-01"]
```

And in keys.toml:
```toml
[providers.minimax]
api_key = "your-key"
```

In `ProviderConfig` struct:
```rust
pub struct ProviderConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub models: Vec<String>,  // For providers without /models endpoint
}
```

In onboarding UI `all_model_names()`:
```rust
pub fn all_model_names(&self) -> Vec<&str> {
    if !self.fetched_models.is_empty() {
        // API fetch took priority
        self.fetched_models.iter().map(|s| s.as_str()).collect()
    } else if !self.config_models.is_empty() {
        // Use models from config.toml
        self.config_models.iter().map(|s| s.as_str()).collect()
    } else {
        // Fallback to static list
        self.current_provider().models.to_vec()
    }
}
```

## Config.toml Example

### For providers with API model fetch (OpenAI, OpenRouter, Anthropic):
config.toml:
```toml
[providers.openrouter]
enabled = true
base_url = "https://openrouter.ai/api/v1/chat/completions"
default_model = "qwen/qwen3-coder-next"
```

keys.toml:
```toml
[providers.openrouter]
api_key = "sk-or-v1-..."
```

### For providers WITHOUT API model fetch (Minimax, Custom):
```toml
[providers.minimax]
enabled = true
api_key = "your-key"
base_url = "https://api.minimax.io/v1"
default_model = "MiniMax-M2.5"
models = ["MiniMax-M2.5", "MiniMax-M2.1", "MiniMax-Text-01"]
```

## Fallback Provider

To add fallback support (use if primary fails):

```toml
[providers.fallback]
enabled = true
provider = "yourprovider"  # or "minimax", "openrouter", etc.
```

## Provider Index Reference

Current provider indices in onboarding UI:
- 0: Anthropic Claude
- 1: OpenAI  
- 2: Google Gemini
- 3: OpenRouter
- 4: Minimax
- 5: Custom (local Ollama, etc.)
