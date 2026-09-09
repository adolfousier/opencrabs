//! Shared approval utilities used across all channel integrations.
//!
//! Centralises the config-level approval policy check and the
//! "always approve" persistence so every channel behaves identically.

/// Whether `policy` grants tool execution without an interactive approval.
///
/// The one place the policy strings are interpreted. Both auto modes are
/// treated the same here: the difference between them is how long the grant
/// is meant to last, not whether it applies to a given call.
///
/// Pure and cheap, so callers that must not touch the disk (the per-tool gate
/// in the tool loop) can resolve the policy once and keep it.
pub fn policy_auto_approves(policy: &str) -> bool {
    matches!(policy, "auto-always" | "auto-session")
}

/// Check config-level approval policy.
/// Returns `Some((true, true))` when the policy auto-approves, `None` otherwise.
///
/// Reads config from disk, so it belongs in approval callbacks, which run once
/// per approval request. It must not be called from the per-tool gate; see
/// [`policy_auto_approves`].
pub fn check_approval_policy() -> Option<(bool, bool)> {
    match crate::config::Config::load() {
        Ok(cfg) => {
            if policy_auto_approves(&cfg.agent.approval_policy) {
                tracing::debug!(
                    "Approval policy is '{}' — auto-approving",
                    cfg.agent.approval_policy
                );
                Some((true, true))
            } else {
                None
            }
        }
        Err(e) => {
            tracing::warn!("Failed to load config for approval check: {}", e);
            None
        }
    }
}

/// Persist "auto-session" approval policy to config.toml (single source of truth).
pub fn persist_auto_session_policy() {
    match crate::config::Config::write_key("agent", "approval_policy", "auto-session") {
        Ok(_written) => tracing::info!("Persisted approval_policy = auto-session to config.toml"),
        Err(e) => tracing::error!("Failed to persist approval_policy to config.toml: {}", e),
    }
}

/// Persist "auto-always" (YOLO) approval policy to config.toml — permanent, survives restarts.
pub fn persist_auto_always_policy() {
    match crate::config::Config::write_key("agent", "approval_policy", "auto-always") {
        Ok(_written) => tracing::info!("Persisted approval_policy = auto-always to config.toml"),
        Err(e) => tracing::error!("Failed to persist approval_policy to config.toml: {}", e),
    }
}
