//! Provider health tracking — records success/failure per provider.
//!
//! Persisted to `~/.opencrabs/provider_health.json`. Used for auto-fallback:
//! when the current provider fails, the system can suggest or switch to the
//! last provider that successfully returned a response.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Per-provider health record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    /// Last successful response timestamp (epoch seconds).
    pub last_success: Option<u64>,
    /// Last failure timestamp (epoch seconds).
    pub last_failure: Option<u64>,
    /// Last error message (truncated to 200 chars).
    pub last_error: Option<String>,
    /// Consecutive failure count (resets on success).
    pub consecutive_failures: u32,
}

/// Health state for all providers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthState {
    pub providers: HashMap<String, ProviderHealth>,
}

/// Global in-memory health state (flushed to disk periodically).
static HEALTH: Mutex<Option<HealthState>> = Mutex::new(None);

fn health_path() -> PathBuf {
    super::opencrabs_home().join("provider_health.json")
}

/// Load health state from disk (or initialize empty).
fn ensure_loaded() -> HealthState {
    let mut guard = HEALTH.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref state) = *guard {
        return state.clone();
    }
    let state: HealthState = std::fs::read_to_string(health_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    *guard = Some(state.clone());
    state
}

/// Persist health state to disk. Silently ignores errors.
fn flush(state: &HealthState) {
    let mut guard = HEALTH.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(state.clone());
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(health_path(), json);
    }
}

/// Record a successful provider response.
pub fn record_success(provider_name: &str) {
    let mut state = ensure_loaded();
    let entry = state
        .providers
        .entry(provider_name.to_string())
        .or_insert(ProviderHealth {
            last_success: None,
            last_failure: None,
            last_error: None,
            consecutive_failures: 0,
        });
    entry.last_success = Some(now_epoch());
    entry.consecutive_failures = 0;
    flush(&state);
}

/// Record a provider failure.
pub fn record_failure(provider_name: &str, error: &str) {
    let mut state = ensure_loaded();
    let entry = state
        .providers
        .entry(provider_name.to_string())
        .or_insert(ProviderHealth {
            last_success: None,
            last_failure: None,
            last_error: None,
            consecutive_failures: 0,
        });
    entry.last_failure = Some(now_epoch());
    entry.last_error = Some(error.chars().take(200).collect());
    entry.consecutive_failures += 1;
    flush(&state);
}

/// Get the name of the last provider that succeeded (most recent `last_success`).
/// Returns None if no provider has ever succeeded.
pub fn last_working_provider() -> Option<String> {
    let state = ensure_loaded();
    state
        .providers
        .iter()
        .filter_map(|(name, health)| health.last_success.map(|ts| (name.clone(), ts)))
        .max_by_key(|(_, ts)| *ts)
        .map(|(name, _)| name)
}

/// Get health info for a specific provider.
pub fn get_health(provider_name: &str) -> Option<ProviderHealth> {
    let state = ensure_loaded();
    state.providers.get(provider_name).cloned()
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
