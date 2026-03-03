//! Shared approval utilities used across all channel integrations.
//!
//! Centralises the config-level approval policy check and the
//! "always approve" persistence so every channel behaves identically.

/// Check config-level approval policy.
/// Returns `Some((true, true))` when the policy auto-approves, `None` otherwise.
pub fn check_approval_policy() -> Option<(bool, bool)> {
    match crate::config::Config::load() {
        Ok(cfg) => match cfg.agent.approval_policy.as_str() {
            "auto-always" | "auto-session" => {
                tracing::debug!(
                    "Approval policy is '{}' — auto-approving",
                    cfg.agent.approval_policy
                );
                Some((true, true))
            }
            _ => None,
        },
        Err(e) => {
            tracing::warn!("Failed to load config for approval check: {}", e);
            None
        }
    }
}

/// Persist "auto-session" approval policy to config.toml (single source of truth).
pub fn persist_auto_session_policy() {
    match crate::config::Config::write_key("agent", "approval_policy", "auto-session") {
        Ok(_) => tracing::info!("Persisted approval_policy = auto-session to config.toml"),
        Err(e) => tracing::error!("Failed to persist approval_policy to config.toml: {}", e),
    }
}

/// Persist "auto-always" (YOLO) approval policy to config.toml — permanent, survives restarts.
pub fn persist_auto_always_policy() {
    match crate::config::Config::write_key("agent", "approval_policy", "auto-always") {
        Ok(_) => tracing::info!("Persisted approval_policy = auto-always to config.toml"),
        Err(e) => tracing::error!("Failed to persist approval_policy to config.toml: {}", e),
    }
}
