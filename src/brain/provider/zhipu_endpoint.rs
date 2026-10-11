//! Where a z.ai GLM request goes (#1350).
//!
//! z.ai has two documented hosts that behave differently under a long
//! stream: `api.z.ai` closes an idle streaming connection after roughly 30s,
//! `open.bigmodel.cn` does not. The factory used to hardcode the former while
//! the README named the latter, so a user following the docs got a host they
//! never chose. A configured `base_url` on `[providers.zhipu]` now wins, the
//! same way it does for Moonshot; the default stays on `api.z.ai`, which is
//! what the code has always used.
//!
//! Pure so the resolution is testable without a config or a client.

const DEFAULT_HOST: &str = "https://api.z.ai";

/// The path z.ai serves each endpoint type under. `coding` is the Coding
/// Plan channel; anything else is the general API.
fn default_root(endpoint_type: Option<&str>) -> String {
    match endpoint_type {
        Some("coding") => format!("{DEFAULT_HOST}/api/coding/paas/v4"),
        _ => format!("{DEFAULT_HOST}/api/paas/v4"),
    }
}

/// The API root, from a configured `base_url` or the endpoint-type default.
/// A configured value may be the root or the full chat URL; either way the
/// root is what comes back, without a trailing slash.
fn root(base_url: Option<&str>, endpoint_type: Option<&str>) -> String {
    let configured = base_url.map(str::trim).filter(|s| !s.is_empty());
    let Some(configured) = configured else {
        return default_root(endpoint_type);
    };
    configured
        .trim_end_matches('/')
        .trim_end_matches("/chat/completions")
        .trim_end_matches('/')
        .to_string()
}

/// The chat-completions URL the provider posts to.
pub(crate) fn chat_url(base_url: Option<&str>, endpoint_type: Option<&str>) -> String {
    format!("{}/chat/completions", root(base_url, endpoint_type))
}

/// The `/models` URL the onboarding wizard lists from, on the same host and
/// channel the chat URL resolves to, so the list matches what the key can
/// actually reach.
pub(crate) fn models_url(base_url: Option<&str>, endpoint_type: Option<&str>) -> String {
    format!("{}/models", root(base_url, endpoint_type))
}
