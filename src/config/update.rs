//! Provider auto-update mechanism
//!
//! This module handles automatic updating of LLM provider information
//! from the Crabrace registry.

use anyhow::{Context, Result};
use crabrace::Provider;
use std::time::{Duration, SystemTime};
use tokio::time;
use tracing::{debug, info, warn};

use super::crabrace::CrabraceIntegration;
use super::{Config, ProviderConfig};

/// Provider update manager
pub struct ProviderUpdater {
    crabrace: CrabraceIntegration,
    last_update: Option<SystemTime>,
}

impl ProviderUpdater {
    /// Create a new ProviderUpdater
    pub fn new(crabrace: CrabraceIntegration) -> Self {
        Self {
            crabrace,
            last_update: None,
        }
    }

    /// Check if an update is needed based on the update interval
    pub fn should_update(&self, config: &Config) -> bool {
        if !config.crabrace.enabled || !config.crabrace.auto_update {
            return false;
        }

        let interval = Duration::from_secs(config.crabrace.update_interval_seconds);

        match self.last_update {
            None => true, // Never updated, should update
            Some(last) => {
                let elapsed = SystemTime::now()
                    .duration_since(last)
                    .unwrap_or(Duration::ZERO);
                elapsed >= interval
            }
        }
    }

    /// Perform a provider update
    pub async fn update(&mut self, config: &mut Config) -> Result<UpdateResult> {
        info!("Starting provider update from Crabrace registry");

        // Check Crabrace health first
        match self.crabrace.health_check().await {
            Ok(healthy) if healthy => {
                debug!("Crabrace health check passed");
            }
            Ok(_) => {
                warn!("Crabrace server is unhealthy, skipping update");
                return Ok(UpdateResult {
                    success: false,
                    providers_updated: 0,
                    error: Some("Crabrace server is unhealthy".to_string()),
                });
            }
            Err(e) => {
                warn!("Failed to connect to Crabrace server: {}", e);
                return Ok(UpdateResult {
                    success: false,
                    providers_updated: 0,
                    error: Some(format!("Connection failed: {}", e)),
                });
            }
        }

        // Fetch providers from Crabrace
        let providers = self
            .crabrace
            .fetch_providers()
            .await
            .context("Failed to fetch providers from Crabrace")?;

        info!("Fetched {} providers from Crabrace", providers.len());

        // Update config with provider information
        let mut updated_count = 0;
        for provider in providers {
            if self.update_provider_config(config, &provider) {
                updated_count += 1;
            }
        }

        // Update last update timestamp
        self.last_update = Some(SystemTime::now());

        info!(
            "Provider update completed: {} providers updated",
            updated_count
        );

        Ok(UpdateResult {
            success: true,
            providers_updated: updated_count,
            error: None,
        })
    }

    /// Update a single provider's configuration
    fn update_provider_config(&self, config: &mut Config, provider: &Provider) -> bool {
        debug!("Updating provider config for: {}", provider.id);

        // Custom providers are a named BTreeMap — handle separately
        if provider.id == "custom" {
            return self.update_custom_provider_config(config, provider);
        }

        let provider_config = match provider.id.as_str() {
            "anthropic" => &mut config.providers.anthropic,
            "openai" => &mut config.providers.openai,
            "openrouter" => &mut config.providers.openrouter,
            "minimax" => &mut config.providers.minimax,
            "gemini" | "google" => &mut config.providers.gemini,
            "bedrock" | "aws-bedrock" => &mut config.providers.bedrock,
            "vertex" | "vertexai" => &mut config.providers.vertex,
            _ => {
                debug!("Unknown provider: {}, skipping", provider.id);
                return false;
            }
        };

        Self::apply_provider_update(provider_config, provider)
    }

    /// Update a named custom provider (stored in BTreeMap)
    fn update_custom_provider_config(&self, config: &mut Config, provider: &Provider) -> bool {
        let customs = config
            .providers
            .custom
            .get_or_insert_with(std::collections::BTreeMap::new);
        let entry = customs
            .entry("default".to_string())
            .or_insert_with(|| ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
                models: vec![],
            });
        let mut provider_opt = Some(entry.clone());
        let updated = Self::apply_provider_update(&mut provider_opt, provider);
        if let Some(cfg) = provider_opt {
            *entry = cfg;
        }
        updated
    }

    /// Apply a provider update to an Option<ProviderConfig>
    fn apply_provider_update(
        provider_config: &mut Option<ProviderConfig>,
        provider: &Provider,
    ) -> bool {
        // Create or update provider config
        let mut updated = false;
        let new_config = provider_config.get_or_insert_with(|| {
            updated = true;
            ProviderConfig {
                enabled: true,
                api_key: None,
                base_url: None,
                default_model: None,
                models: vec![],
            }
        });

        // Update base URL if provider specifies one
        if let Some(api_endpoint) = &provider.api_endpoint
            && new_config.base_url.as_ref() != Some(api_endpoint)
        {
            new_config.base_url = Some(api_endpoint.clone());
            updated = true;
        }

        // Update default model if not set and provider has models
        if new_config.default_model.is_none() && !provider.models.is_empty() {
            // Use the first model as default
            new_config.default_model = Some(provider.models[0].id.clone());
            updated = true;
        }

        updated
    }

    /// Start automatic update loop in the background
    pub async fn start_auto_update_loop(mut self, mut config: Config) {
        info!("Starting provider auto-update loop");

        loop {
            if self.should_update(&config) {
                match self.update(&mut config).await {
                    Ok(result) => {
                        if result.success {
                            info!(
                                "Auto-update successful: {} providers updated",
                                result.providers_updated
                            );
                        } else {
                            warn!("Auto-update failed: {:?}", result.error);
                        }
                    }
                    Err(e) => {
                        warn!("Auto-update error: {}", e);
                    }
                }
            }

            // Sleep for a short period before checking again
            time::sleep(Duration::from_secs(60)).await;
        }
    }

    /// Perform a one-time update (for manual updates)
    pub async fn update_once(config: &mut Config) -> Result<UpdateResult> {
        let crabrace = CrabraceIntegration::new(config.crabrace.clone())?;
        let mut updater = Self::new(crabrace);
        updater.update(config).await
    }
}

/// Result of a provider update operation
#[derive(Debug, Clone)]
pub struct UpdateResult {
    /// Whether the update was successful
    pub success: bool,
    /// Number of providers that were updated
    pub providers_updated: usize,
    /// Error message if the update failed
    pub error: Option<String>,
}

impl UpdateResult {
    /// Create a success result
    pub fn success(providers_updated: usize) -> Self {
        Self {
            success: true,
            providers_updated,
            error: None,
        }
    }

    /// Create a failure result
    pub fn failure(error: String) -> Self {
        Self {
            success: false,
            providers_updated: 0,
            error: Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::crabrace::CrabraceConfig;

    #[test]
    fn test_should_update_when_disabled() {
        let crabrace_config = CrabraceConfig {
            enabled: false,
            ..Default::default()
        };
        let crabrace = CrabraceIntegration::new(crabrace_config.clone()).unwrap();
        let updater = ProviderUpdater::new(crabrace);

        let config = Config {
            crabrace: crabrace_config,
            ..Default::default()
        };

        assert!(!updater.should_update(&config));
    }

    #[test]
    fn test_should_update_when_never_updated() {
        let crabrace_config = CrabraceConfig {
            enabled: true,
            auto_update: true,
            ..Default::default()
        };
        let crabrace = CrabraceIntegration::new(crabrace_config.clone()).unwrap();
        let updater = ProviderUpdater::new(crabrace);

        let config = Config {
            crabrace: crabrace_config,
            ..Default::default()
        };

        assert!(updater.should_update(&config));
    }

    #[test]
    fn test_update_result_success() {
        let result = UpdateResult::success(5);
        assert!(result.success);
        assert_eq!(result.providers_updated, 5);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_update_result_failure() {
        let result = UpdateResult::failure("Connection failed".to_string());
        assert!(!result.success);
        assert_eq!(result.providers_updated, 0);
        assert_eq!(result.error, Some("Connection failed".to_string()));
    }
}
