//! Provider Factory
//!
//! Creates providers based on config.toml settings.
//!
//! ## Registry Pattern
//!
//! All built-in providers are registered in the `REGISTRATIONS` array below.
//! Adding a new provider requires only:
//! 1. Adding a `ProviderRegistration` entry to `REGISTRATIONS`
//! 2. Adding the corresponding field to `ProviderConfigs` in `config/types.rs`
//! 3. Writing a `try_create_*` function
//!
//! The factory functions (`provider_enabled`,
//! `create_provider_by_name`, `create_fallback`, `active_provider_vision`)
//! all iterate the registry automatically.

use super::qwen::{looks_like_qwen_target, qwen_body_transform, qwen_extra_headers};
use super::{
    Provider,
    anthropic::AnthropicProvider,
    claude_cli::ClaudeCliProvider,
    codex_cli::CodexCliProvider,
    codex_oauth::CodexOAuthProvider,
    command_code_cli::CommandCodeCliProvider,
    custom_openai_compatible::{BodyTransformFn, OpenAIProvider},
    gemini::GeminiProvider,
    opencode_cli::OpenCodeCliProvider,
};
use crate::config::timeout::{TimeoutResolution, resolve_timeout};
use crate::config::{AgentConfig, Config, ProviderConfig};
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};

// ── Provider Registry ───────────────────────────────────────────

/// Type alias for async factory functions stored in the registry.
type ProviderFactoryFn = Box<
    dyn Fn(&Config) -> Pin<Box<dyn Future<Output = Result<Option<Arc<dyn Provider>>>> + Send + '_>>
        + Send
        + Sync,
>;

/// Type alias for sync factory functions (used by `sync_factory`).
type SyncProviderFactoryFn = fn(&Config) -> Result<Option<Arc<dyn Provider>>>;

/// Wrap a synchronous factory function into the async registry type.
fn sync_factory(f: SyncProviderFactoryFn) -> ProviderFactoryFn {
    Box::new(move |config| Box::pin(async move { f(config) }))
}

/// Registration entry for a built-in provider.
struct ProviderRegistration {
    /// Display name shown in pickers and logs (e.g. "Anthropic").
    display_name: &'static str,
    /// Session ID used for restoration (e.g. "anthropic").
    session_id: &'static str,
    /// Alternative session IDs (e.g. ["claude-cli", "claude_cli"]).
    aliases: &'static [&'static str],
    /// Check if this provider is enabled in config.
    is_enabled: fn(&Config) -> bool,
    /// Try to create the provider instance.
    factory: ProviderFactoryFn,
    /// Extract the provider config for vision/model lookups.
    config_field: fn(&Config) -> Option<&ProviderConfig>,
}

/// All built-in providers in priority order.
///
/// **IMPORTANT:** This array must stay in sync with `PROVIDER_NAMES`.
/// The index is used by `provider_enabled()`.
static REGISTRATIONS: LazyLock<Vec<ProviderRegistration>> = LazyLock::new(|| {
    vec![
        // Xiaomi MiMo — OpenAI-compatible, keyed (key from platform.xiaomimimo.com).
        ProviderRegistration {
            display_name: "Xiaomi",
            session_id: "xiaomi",
            aliases: &["mimo", "xiaomi-mimo"],
            is_enabled: |c| c.providers.xiaomi.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_xiaomi),
            config_field: |c| c.providers.xiaomi.as_ref(),
        },
        ProviderRegistration {
            display_name: "Claude CLI",
            session_id: "claude-cli",
            aliases: &["claude_cli"],
            is_enabled: |c| c.providers.claude_cli.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_claude_cli),
            config_field: |c| c.providers.claude_cli.as_ref(),
        },
        ProviderRegistration {
            display_name: "OpenCode CLI",
            session_id: "opencode-cli",
            aliases: &["opencode_cli"],
            is_enabled: |c| c.providers.opencode_cli.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_opencode_cli),
            config_field: |c| c.providers.opencode_cli.as_ref(),
        },
        ProviderRegistration {
            display_name: "Codex CLI",
            session_id: "codex-cli",
            aliases: &["codex_cli"],
            is_enabled: |c| c.providers.codex_cli.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_codex_cli),
            config_field: |c| c.providers.codex_cli.as_ref(),
        },
        ProviderRegistration {
            display_name: "Command Code CLI",
            session_id: "command-code-cli",
            aliases: &["command_code_cli", "cmd-cli"],
            is_enabled: |c| {
                c.providers
                    .command_code_cli
                    .as_ref()
                    .is_some_and(|p| p.enabled)
            },
            factory: sync_factory(try_create_command_code_cli),
            config_field: |c| c.providers.command_code_cli.as_ref(),
        },
        ProviderRegistration {
            display_name: "Codex",
            session_id: "codex",
            aliases: &["codex_oauth"],
            is_enabled: |c| c.providers.codex.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_codex_oauth),
            config_field: |c| c.providers.codex.as_ref(),
        },
        ProviderRegistration {
            display_name: "OpenCode",
            session_id: "opencode",
            aliases: &["opencode_api"],
            is_enabled: |c| c.providers.opencode.as_ref().is_some_and(|p| p.enabled),
            factory: Box::new(|config| Box::pin(try_create_opencode(config))),
            config_field: |c| c.providers.opencode.as_ref(),
        },
        ProviderRegistration {
            display_name: "Qwen",
            session_id: "qwen",
            aliases: &[],
            is_enabled: |c| c.providers.qwen.as_ref().is_some_and(|p| p.enabled),
            factory: Box::new(|config| Box::pin(try_create_qwen(config))),
            config_field: |c| c.providers.qwen.as_ref(),
        },
        ProviderRegistration {
            display_name: "Anthropic",
            session_id: "anthropic",
            aliases: &[],
            is_enabled: |c| c.providers.anthropic.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_anthropic),
            config_field: |c| c.providers.anthropic.as_ref(),
        },
        ProviderRegistration {
            display_name: "OpenAI",
            session_id: "openai",
            aliases: &[],
            is_enabled: |c| c.providers.openai.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_openai),
            config_field: |c| c.providers.openai.as_ref(),
        },
        ProviderRegistration {
            display_name: "GitHub Copilot",
            session_id: "github",
            aliases: &[],
            is_enabled: |c| c.providers.github.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_github),
            config_field: |c| c.providers.github.as_ref(),
        },
        ProviderRegistration {
            display_name: "Google Gemini",
            session_id: "gemini",
            aliases: &[],
            is_enabled: |c| c.providers.gemini.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_gemini),
            config_field: |c| c.providers.gemini.as_ref(),
        },
        ProviderRegistration {
            display_name: "OpenRouter",
            session_id: "openrouter",
            aliases: &[],
            is_enabled: |c| c.providers.openrouter.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_openrouter),
            config_field: |c| c.providers.openrouter.as_ref(),
        },
        ProviderRegistration {
            display_name: "Minimax",
            session_id: "minimax",
            aliases: &[],
            is_enabled: |c| c.providers.minimax.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_minimax),
            config_field: |c| c.providers.minimax.as_ref(),
        },
        ProviderRegistration {
            display_name: "z.ai",
            session_id: "zai",
            // Legacy spelling: by-name lookups ("zhipu/glm-5.3", fallback
            // chains, subagent_provider) keep resolving after the #1448 rename.
            aliases: &["zhipu"],
            is_enabled: |c| c.providers.zai.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_zhipu),
            config_field: |c| c.providers.zai.as_ref(),
        },
        ProviderRegistration {
            display_name: "Moonshot AI",
            session_id: "moonshot",
            aliases: &["kimi", "moonshotai"],
            is_enabled: |c| c.providers.moonshot.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_moonshot),
            config_field: |c| c.providers.moonshot.as_ref(),
        },
        ProviderRegistration {
            display_name: "Ollama",
            session_id: "ollama",
            aliases: &[],
            is_enabled: |c| c.providers.ollama.as_ref().is_some_and(|p| p.enabled),
            factory: sync_factory(try_create_ollama),
            config_field: |c| c.providers.ollama.as_ref(),
        },
        ProviderRegistration {
            display_name: "Custom",
            session_id: "custom",
            aliases: &[],
            is_enabled: |c| c.providers.active_custom().is_some(),
            factory: sync_factory(try_create_custom),
            config_field: |_| None, // Custom uses active_custom() instead
        },
    ]
});

/// Provider names in priority order, derived from REGISTRATIONS.
/// Whether `name` names a configured provider: a REGISTRATIONS session_id
/// or alias, or a `[providers.custom.<name>]` key (with or without the
/// `custom:` prefix). Used by non-interactive switch surfaces (#461 family)
/// to reject typos before touching session rows.
pub fn provider_config_by_name<'a>(config: &'a Config, name: &str) -> Option<&'a ProviderConfig> {
    let lookup = name.strip_prefix("custom:").unwrap_or(name);
    if let Some(cfg) = config.providers.custom.as_ref().and_then(|m| m.get(lookup)) {
        return Some(cfg);
    }
    REGISTRATIONS
        .iter()
        .find(|reg| reg.session_id == name || reg.aliases.contains(&name))
        .and_then(|reg| (reg.config_field)(config))
}

pub fn is_known_provider_name(config: &Config, name: &str) -> bool {
    let lookup = name.strip_prefix("custom:").unwrap_or(name);
    if config
        .providers
        .custom
        .as_ref()
        .is_some_and(|m| m.contains_key(lookup))
    {
        return true;
    }
    REGISTRATIONS
        .iter()
        .any(|reg| reg.session_id == name || reg.aliases.contains(&name))
}

pub const PROVIDER_NAMES: &[&str] = &[
    "Xiaomi",
    "Claude CLI",
    "OpenCode CLI",
    "Codex CLI",
    "Command Code CLI",
    "Codex",
    "OpenCode",
    "Qwen",
    "Anthropic",
    "OpenAI",
    "GitHub Copilot",
    "Google Gemini",
    "OpenRouter",
    "Minimax",
    "z.ai",
    "Moonshot AI",
    "Ollama",
    "Custom",
];

/// Whether a provider is enabled in config, by index matching PROVIDER_NAMES.
fn provider_enabled(config: &Config, idx: usize) -> bool {
    REGISTRATIONS
        .get(idx)
        .is_some_and(|reg| (reg.is_enabled)(config))
}

/// All built-in provider session_ids in priority order.
/// Used for cross-checking TUI provider ids against the factory registry.
pub fn provider_session_ids() -> Vec<&'static str> {
    REGISTRATIONS.iter().map(|r| r.session_id).collect()
}

// ── Local URL detection & thinking transform ────────────────────

/// Detect whether a base URL points to a local inference server
/// (llama.cpp, MLX, LM Studio, Ollama, etc.). Used to gate behaviours
/// that only make sense for self-hosted backends — specifically the
/// `chat_template_kwargs` injection for local Qwen, which cloud Qwen
/// (DashScope) rejects because it sets the reasoning flag server-side.
///
/// Matches loopback (`localhost`, `127.0.0.1`, `::1`, `0.0.0.0`),
/// mDNS (`*.local`), and the three RFC1918 private-network ranges
/// (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`). Public hosts
/// always return false so production endpoints keep their existing
/// request shape.
pub(crate) fn is_local_base_url(url: &str) -> bool {
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host_and_port = after_scheme.split(['/', '?']).next().unwrap_or("");
    // IPv6 addresses are bracketed: `[::1]:1234`. Strip brackets first so
    // the port-split logic doesn't mangle the address.
    let bare = if let Some(rest) = host_and_port.strip_prefix('[') {
        rest.split_once(']').map(|(h, _)| h).unwrap_or(rest)
    } else {
        host_and_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_and_port)
    };
    let bare = bare.to_ascii_lowercase();
    if bare == "localhost"
        || bare == "127.0.0.1"
        || bare == "0.0.0.0"
        || bare == "::1"
        || bare.ends_with(".local")
        || bare.starts_with("192.168.")
        || bare.starts_with("10.")
    {
        return true;
    }
    // 172.16.0.0 – 172.31.255.255
    let mut parts = bare.split('.');
    if parts.next() == Some("172")
        && let Some(second) = parts.next()
        && let Ok(n) = second.parse::<u8>()
        && (16..=31).contains(&n)
    {
        return true;
    }
    false
}

/// Build a body transform that injects
/// `chat_template_kwargs: {"enable_thinking": X}` into every request sent
/// to a local llama.cpp / MLX / LM Studio / Ollama server. Mirrors what
/// `llama-server --jinja --chat-template-kwargs '{"enable_thinking":true}'`
/// does — the flags Unsloth Studio launches with — so embedded GGUF/MLX
/// chat templates render `<tool_call>` tags and reasoning blocks the way
/// the model was trained on.
///
/// Generic across thinking-capable models (Qwen3, Kimi-K2, DeepSeek-R1
/// variants, etc.) — we inject when opted in, regardless of the model
/// name, because the mechanism is a llama.cpp jinja feature, not a
/// model-specific one. Callers only install this transform when the
/// user set `enable_thinking` in config (opt-in), so local models whose
/// template doesn't accept the variable stay untouched by default.
fn local_thinking_body_transform(enable: bool) -> BodyTransformFn {
    Arc::new(move |mut body: serde_json::Value| {
        if let Some(obj) = body.as_object_mut()
            && !obj.contains_key("chat_template_kwargs")
        {
            obj.insert(
                "chat_template_kwargs".to_string(),
                serde_json::json!({ "enable_thinking": enable }),
            );
        }
        body
    })
}

/// Body transform for custom providers that auto-applies `qwen_body_transform`
/// when the outgoing request looks Qwen / Alibaba-shaped.
///
/// `base_url` is captured at construction (provider-level, static). `model` is
/// read from each outgoing request body (dynamic — one custom provider may
/// route to many models, only some of which are Qwen). The transform is a
/// no-op for non-matching requests: the body passes through unchanged.
///
/// Composes safely with `local_thinking_body_transform` when both are
/// installed — order matters only insofar as the cache transform expects to
/// see `messages` and `tools` in their final form, which `local_thinking_*`
/// doesn't touch.
///
/// This unlocks Alibaba's explicit cache (90% off on hits, 25% surcharge on
/// create, 5-minute TTL auto-renewed) for any custom provider pointed at a
/// known Qwen endpoint or running a `qwen-*` model — zero user config.
///
/// Logs one `info!` line the first time the cache transform fires for any
/// new `(base_url, model)` pair so users (and post-incident triage) can see
/// in the log when caching auto-engaged. Subsequent requests with the same
/// pair stay silent; switching to a different qwen model on the same
/// provider logs once more.
pub(crate) fn auto_qwen_cache_transform(base_url: String) -> BodyTransformFn {
    use std::collections::HashSet;
    use std::sync::Mutex;
    let seen: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    Arc::new(move |body: serde_json::Value| {
        let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
        if looks_like_qwen_target(&base_url, model) {
            let key = format!("{base_url}|{model}");
            let first_time = {
                let mut guard = seen.lock().unwrap_or_else(|p| p.into_inner());
                guard.insert(key)
            };
            if first_time {
                tracing::info!(
                    "Auto-enabled Qwen ephemeral cache_control for custom provider \
                     (base_url={base_url}, model={model})"
                );
            }
            qwen_body_transform(body)
        } else {
            body
        }
    })
}

/// Compose multiple body transforms left-to-right.
pub(crate) fn chain_body_transforms(a: BodyTransformFn, b: BodyTransformFn) -> BodyTransformFn {
    Arc::new(move |body| b(a(body)))
}

// ── Public factory functions ────────────────────────────────────

/// Create a provider based on config.toml
/// No hardcoded priority - providers are enabled/disabled in config
pub async fn create_provider(config: &Config) -> Result<Arc<dyn Provider>> {
    let (provider, warning) = create_provider_with_warning(config).await?;
    if let Some(msg) = &warning {
        tracing::warn!("{}", msg);
    }
    Ok(provider)
}

/// Like `create_provider` but returns a warning message when a fallback was used.
/// The caller (TUI) should surface this to the user instead of printing to stderr.
pub async fn create_provider_with_warning(
    config: &Config,
) -> Result<(Arc<dyn Provider>, Option<String>)> {
    let mut primary: Option<Arc<dyn Provider>> = None;
    let mut failed_name: Option<&str> = None;
    let mut warning: Option<String> = None;

    // Try enabled providers in priority order
    for (i, reg) in REGISTRATIONS.iter().enumerate() {
        if !provider_enabled(config, i) {
            continue;
        }

        match (reg.factory)(config).await {
            Ok(Some(provider)) => {
                if let Some(failed) = failed_name {
                    let msg = format!(
                        "{} failed to initialize — fell back to {}. Run /onboard:provider to reconfigure.",
                        failed, reg.display_name
                    );
                    tracing::warn!("{}", msg);
                    warning = Some(msg);
                }
                tracing::info!("Using enabled provider: {}", reg.display_name);
                primary = Some(provider);
                break;
            }
            Ok(None) => {
                tracing::debug!(
                    "{} enabled but has no API key — skipping (not actionable, expected on fresh installs with config stubs)",
                    reg.display_name
                );
                if failed_name.is_none() {
                    failed_name = Some(reg.display_name);
                }
            }
            Err(e) => {
                tracing::error!("{} provider error: {}", reg.display_name, e);
                if failed_name.is_none() {
                    failed_name = Some(reg.display_name);
                }
            }
        }
    }

    // Delegate the primary-wrap step to the shared helper. This is the
    // common path: there IS a primary, and the user has configured a
    // fallback chain. Going through the helper means the RSI loop
    // (which calls `create_provider_by_name` + helper) gets identical
    // fallback semantics — no divergence, no RSI-specific bugs.
    //
    // The `None` branch (no primary at all) keeps its own fallback-as-
    // primary logic because there's no primary to wrap: instead we
    // pick the first usable fallback as a last-resort primary.
    match primary {
        Some(provider) => {
            let provider = wrap_with_fallback_chain(config, provider).await?;
            Ok((provider, warning))
        }
        None => {
            // No primary — try fallbacks as primary candidates
            if let Some(fallback) = &config.providers.fallback
                && fallback.enabled
            {
                let chain = normalized_fallback_chain(config);
                for name in &chain {
                    if let Ok(p) = create_fallback(config, name).await {
                        tracing::warn!(
                            "No primary provider enabled, using fallback '{}' as primary",
                            name
                        );
                        return Ok((p, warning));
                    }
                }
            }
            tracing::info!("No provider configured, using placeholder provider");
            Ok((Arc::new(super::PlaceholderProvider), warning))
        }
    }
}

/// Wrap an already-resolved primary provider with the user-configured
/// `[providers.fallback]` chain (if any).
///
/// This is the single producer of `FallbackProvider` wrappers in the
/// factory. Both `create_provider_with_warning` (main session path) and
/// `run_rsi_agent_cycle` (RSI autonomous loop) call it after they've
/// produced a primary, so both code paths honour the same fallback
/// config. Before this existed, RSI silently bypassed the chain — see
/// the regression context in `src/tests/rsi_fallback_wrap_test.rs`.
///
/// Semantics:
/// - If `config.providers.fallback` is missing, disabled, or produces
///   an empty chain, returns `primary` unchanged (no wrap overhead).
/// - Applies a self-name filter: any candidate whose name matches the
///   primary's name is dropped from the chain. Including it would mean
///   a primary failure cascades to the same dead endpoint, defeating
///   the purpose of fallback (the agent would hit the same 429/530
///   twice in a row instead of failing over).
/// - Candidates that fail to construct (missing API key, unknown name,
///   etc.) are logged and skipped — they don't fail the whole wrap.
/// - If at least one usable fallback resolves, wraps the primary in
///   `FallbackProvider::new`. The chain always starts on the user's
///   primary: a provider is never skipped on the strength of past
///   failures, only on a failure observed in the current turn (#1251).
pub(crate) async fn wrap_with_fallback_chain(
    config: &Config,
    primary: Arc<dyn Provider>,
) -> Result<Arc<dyn Provider>> {
    if !config
        .providers
        .fallback
        .as_ref()
        .is_some_and(|f| f.enabled)
    {
        return Ok(primary);
    }

    let chain = normalized_fallback_chain(config);
    let primary_name = primary.name().to_string();
    let mut providers = Vec::new();
    for name in &chain {
        if *name == primary_name {
            // Self-name collision: skip so a primary failure doesn't
            // cascade straight to itself. Logged at debug because in
            // common configs (e.g. fallback = ["minimax"] when primary
            // is opencode) this branch never fires.
            tracing::debug!(
                "Skipping fallback '{}' — same name as primary, would create a self-loop",
                name
            );
            continue;
        }
        match create_fallback(config, name).await {
            Ok(p) => {
                tracing::info!("Fallback provider '{}' ready", name);
                providers.push(p);
            }
            Err(e) => {
                tracing::warn!("Fallback provider '{}' skipped: {}", name, e);
            }
        }
    }

    if providers.is_empty() {
        return Ok(primary);
    }

    tracing::info!(
        "Wrapping primary provider '{}' with {} fallback(s)",
        primary_name,
        providers.len()
    );
    Ok(Arc::new(super::FallbackProvider::new(primary, providers)))
}

/// Force-enable a built-in provider's section in a CLONED config so a
/// by-name creation can honour its "ignoring the `enabled` flag" contract:
/// each `try_create_*` gates on `cfg.enabled` internally, so a session
/// pinned to a provider whose section was disabled (or, for the keyless CLI
/// providers, absent) could never be restored after a restart — a claude-cli
/// session silently landed on another provider (#270). CLI providers get a
/// default section synthesized when missing because they need no API key;
/// keyed providers without a section stay un-creatable (there is no key to
/// use anyway).
pub(crate) fn force_enable_section(config: &mut Config, session_id: &str) -> bool {
    let p = &mut config.providers;
    let slot: Option<&mut Option<ProviderConfig>> = match session_id {
        "xiaomi" => Some(&mut p.xiaomi),
        "claude-cli" => Some(&mut p.claude_cli),
        "opencode-cli" => Some(&mut p.opencode_cli),
        "codex-cli" => Some(&mut p.codex_cli),
        "command-code-cli" => Some(&mut p.command_code_cli),
        "codex" => Some(&mut p.codex),
        "opencode" => Some(&mut p.opencode),
        "qwen" => Some(&mut p.qwen),
        "anthropic" => Some(&mut p.anthropic),
        "openai" => Some(&mut p.openai),
        "github" => Some(&mut p.github),
        "gemini" => Some(&mut p.gemini),
        "openrouter" => Some(&mut p.openrouter),
        "minimax" => Some(&mut p.minimax),
        "zai" => Some(&mut p.zai),
        "moonshot" => Some(&mut p.moonshot),
        "ollama" => Some(&mut p.ollama),
        _ => None,
    };
    let Some(slot) = slot else {
        return false;
    };
    let cli_auth = matches!(
        session_id,
        "claude-cli" | "opencode-cli" | "codex-cli" | "command-code-cli"
    );
    match slot {
        Some(cfg) => {
            cfg.enabled = true;
            true
        }
        None if cli_auth => {
            *slot = Some(ProviderConfig {
                enabled: true,
                ..ProviderConfig::default()
            });
            true
        }
        None => false,
    }
}

/// Create a provider by name, ignoring the `enabled` flag.
/// Used for per-session provider restoration without toggling disk config.
/// Accepts names like "anthropic", "openai", "minimax", "openrouter", or "custom:<name>".
pub async fn create_provider_by_name(config: &Config, name: &str) -> Result<Arc<dyn Provider>> {
    // Custom entries take precedence over built-in names. If the user
    // created a custom provider literally named "opencode" / "anthropic"
    // / anything that collides with a built-in id, the custom entry wins.
    if !name.starts_with("custom:")
        && config
            .providers
            .custom
            .as_ref()
            .is_some_and(|m| m.contains_key(name))
        && let Some(p) = try_create_custom_by_name(config, name)?
    {
        return Ok(p);
    }

    // Try built-in registry by session_id or alias
    for reg in REGISTRATIONS.iter() {
        if reg.session_id == name || reg.aliases.contains(&name) {
            if let Some(provider) = (reg.factory)(config).await? {
                return Ok(provider);
            }
            // Creation by NAME means the user explicitly pinned or listed
            // this provider — the `enabled` startup gate must not veto it.
            // Retry with the section force-enabled in a cloned config (#270).
            let mut forced = config.clone();
            if force_enable_section(&mut forced, reg.session_id)
                && let Some(provider) = (reg.factory)(&forced).await?
            {
                tracing::info!(
                    "{}: created by name with its disabled/absent config section force-enabled",
                    reg.display_name
                );
                return Ok(provider);
            }
            return Err(anyhow::anyhow!(
                "{} not configured (no usable config section, API key, or CLI binary)",
                reg.display_name
            ));
        }
    }

    // Try custom: (colon) or custom. (dotted TOML key) prefix. Fallback
    // chains in config.toml are often written with the dotted form
    // "custom.mimo" (mirroring the [providers.custom.mimo] table path), but
    // the custom map is keyed by the bare name "mimo". Accept either spelling
    // so a natural config resolves instead of failing as "Unknown provider"
    // (#260).
    if let Some(custom_name) = name
        .strip_prefix("custom:")
        .or_else(|| name.strip_prefix("custom."))
    {
        return try_create_custom_by_name(config, custom_name)?.ok_or_else(|| {
            anyhow::anyhow!(
                "custom provider '{name}' is named but could not be built \
                 (missing base_url/api_key, or the entry is absent) — \
                 check [providers.custom.{custom_name}] in config.toml"
            )
        });
    }

    // Try as a custom provider name directly (legacy sessions)
    try_create_custom_by_name(config, name)?.ok_or_else(|| {
        let available = config
            .providers
            .custom
            .as_ref()
            .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "none".to_string());
        anyhow::anyhow!(
            "unknown provider '{name}': not a built-in id and no matching \
             [providers.custom.*] entry. Reference a custom provider as \
             'custom.<name>' or '<name>'. Available custom providers: {available}"
        )
    })
}

/// Try to create a specific named custom provider (ignores enabled flag).
fn try_create_custom_by_name(config: &Config, name: &str) -> Result<Option<Arc<dyn Provider>>> {
    let customs = match &config.providers.custom {
        Some(map) => map,
        None => return Ok(None),
    };

    let custom_config = match customs.get(name) {
        Some(cfg) => cfg.clone(),
        None => {
            tracing::warn!("Custom provider '{}' not found in config", name);
            return Ok(None);
        }
    };

    // API key is optional for local providers (LM Studio, Ollama, etc.)
    let api_key = custom_config.api_key.clone().unwrap_or_default();

    // base_url is REQUIRED. Silently defaulting to localhost:1234 (LM Studio)
    // produced channel-side errors like "failed to connect to localhost:1234"
    // whenever a session had a stale/missing custom provider entry — the user
    // saw the bot trying to reach a server they weren't running. Refuse to
    // construct the provider without an explicit base_url so the caller can
    // fall through to the next option (e.g. the global active provider).
    let Some(mut base_url) = custom_config.base_url.clone() else {
        tracing::warn!(
            "Custom provider '{}' has no base_url configured — skipping (run /onboard:provider)",
            name
        );
        return Ok(None);
    };

    if !base_url.contains("/chat/completions") {
        base_url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    }

    // Redact the key for logs but confirm merger ran. An empty key here
    // means keys.toml wasn't merged into this custom entry — the request
    // will then go out with `Bearer ` (empty) and always 401/403. Surface
    // that loudly instead of silently constructing a provider that can't
    // authenticate.
    let key_status = if api_key.is_empty() {
        "MISSING".to_string()
    } else if crate::config::stored_key::is_stored_marker(&api_key) {
        "SENTINEL(__EXISTING_KEY__ never merged)".to_string()
    } else {
        format!("present (len={})", api_key.len())
    };
    if !crate::config::stored_key::is_real_key(&api_key) {
        tracing::warn!(
            "Custom provider '{}' being constructed without a real api_key ({}). \
             Requests will fail auth. Check keys.toml has [providers.custom.{}] api_key = \"...\" \
             and that the key isn't the literal sentinel string.",
            name,
            key_status,
            name,
        );
    }
    tracing::info!(
        "Creating custom provider '{}' at: {} (api_key: {})",
        name,
        base_url,
        key_status,
    );
    let mut builder =
        OpenAIProvider::with_base_url(api_key.clone(), base_url.clone()).with_name(name);

    // Compose body transforms: thinking flag for local servers, then auto-qwen
    // cache markers for Qwen / Alibaba targets. Both are no-ops on requests
    // that don't match their respective triggers.
    let cache_transform = auto_qwen_cache_transform(base_url.clone());
    let combined_transform = if is_local_base_url(&base_url) {
        let enable = custom_config.enable_thinking.unwrap_or(true);
        chain_body_transforms(local_thinking_body_transform(enable), cache_transform)
    } else {
        cache_transform
    };
    builder = builder.with_body_transform(combined_transform);

    let provider = configure_openai_compatible(builder, &custom_config, &config.agent);
    Ok(Some(Arc::new(provider)))
}

/// Build ordered fallback chain: `providers` array first, legacy `provider` as last resort.
pub(crate) fn fallback_chain(fallback: &crate::config::FallbackProviderConfig) -> Vec<String> {
    let mut chain: Vec<String> = fallback.providers.clone();
    // Append legacy single `provider` if set and not already in the list
    if let Some(ref legacy) = fallback.provider
        && !chain.iter().any(|p| p == legacy)
    {
        chain.push(legacy.clone());
    }
    chain
}

/// One list of provider names, each entry normalised the way the `[agent]`
/// keys are (#1355): a `custom:` prefix is dropped, a `<provider>/<model>`
/// entry becomes its provider (the model has nowhere to go in a list, and
/// the note says where it belongs), and the result is deduplicated in order
/// so `custom:foo` and `foo` collapse. One warning per correction.
fn normalized_names(
    config: &Config,
    key: crate::brain::provider_spec::ProviderKey,
    names: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    for raw in names {
        let pair = crate::brain::provider_spec::normalize_in(config, key, raw, None);
        if let Some(note) = pair.note.as_deref() {
            tracing::warn!(
                "Fallback chain: '{raw}' corrected to '{}': {note}",
                pair.provider
            );
        }
        if !out.contains(&pair.provider) {
            out.push(pair.provider);
        }
    }
    out
}

/// [`fallback_chain`] with every entry normalised against `config` (#1355).
/// Empty when no fallback is configured.
pub(crate) fn normalized_fallback_chain(config: &Config) -> Vec<String> {
    let Some(fallback) = config.providers.fallback.as_ref() else {
        return Vec::new();
    };
    normalized_names(
        config,
        crate::brain::provider_spec::ProviderKey::FALLBACK_PROVIDERS,
        &fallback_chain(fallback),
    )
}

/// `[providers.fallback] vision` with every entry normalised (#1355).
pub(crate) fn normalized_vision_chain(config: &Config) -> Vec<String> {
    let Some(fallback) = config.providers.fallback.as_ref() else {
        return Vec::new();
    };
    normalized_names(
        config,
        crate::brain::provider_spec::ProviderKey::FALLBACK_VISION,
        &fallback.vision,
    )
}

/// `[providers.fallback] generation` with every entry normalised (#1672).
pub(crate) fn normalized_generation_chain(config: &Config) -> Vec<String> {
    let Some(fallback) = config.providers.fallback.as_ref() else {
        return Vec::new();
    };
    normalized_names(
        config,
        crate::brain::provider_spec::ProviderKey::FALLBACK_GENERATION,
        &fallback.generation,
    )
}

/// Create fallback provider
async fn create_fallback(config: &Config, fallback_type: &str) -> Result<Arc<dyn Provider>> {
    // Custom entries take precedence over built-in names. If the user
    // created a custom provider literally named "opencode" / "ollama" /
    // anything that collides with a built-in id, the custom entry wins.
    // (Same priority as create_provider_by_name.)
    if !fallback_type.starts_with("custom:")
        && config
            .providers
            .custom
            .as_ref()
            .is_some_and(|m| m.contains_key(fallback_type))
    {
        tracing::info!("Using fallback: Custom '{}'", fallback_type);
        return try_create_custom_by_name(config, fallback_type)?
            .ok_or_else(|| anyhow::anyhow!("Custom provider '{}' not configured", fallback_type));
    }

    // Try custom: prefix
    if let Some(custom_name) = fallback_type.strip_prefix("custom:") {
        tracing::info!("Using fallback: Custom '{}'", custom_name);
        return try_create_custom_by_name(config, custom_name)?
            .ok_or_else(|| anyhow::anyhow!("Custom provider '{}' not configured", custom_name));
    }

    // Try built-in registry
    for reg in REGISTRATIONS.iter() {
        if reg.session_id == fallback_type || reg.aliases.contains(&fallback_type) {
            tracing::info!("Using fallback: {}", reg.display_name);
            return (reg.factory)(config)
                .await?
                .ok_or_else(|| anyhow::anyhow!("{} not configured", reg.display_name));
        }
    }

    Err(anyhow::anyhow!(
        "Unknown fallback provider: {}",
        fallback_type
    ))
}

/// Returns `(api_key, base_url, vision_model)` for the first enabled provider
/// that has a `vision_model` configured.
///
/// Resolution order:
/// 1. User-configured `[providers.fallback] vision = [...]` chain (first
///    enabled match with a `vision_model` wins).
/// 2. All enabled built-in providers in REGISTRATIONS priority order.
/// 3. All enabled custom providers.
///
/// The fallback chain lets the user pin a specific provider for vision
/// without affecting the active chat provider.  An empty chain is a
/// no-op and falls straight through to the scan-all path.
///
/// Used to register the provider-native `analyze_image` tool when Gemini
/// vision isn't set up.  See issue #253.
pub fn active_provider_vision(config: &Config) -> Option<(String, String, String)> {
    any_provider_vision(config).into_iter().next()
}

/// Every provider that CAN see, scanned in registration order then customs.
///
/// This answers a capability question — "is vision available at all?" — and
/// is deliberately separate from the ORDER things are tried in (#1318).
/// Conflating the two was the bug: this scan used to feed the roll-through
/// directly, so it decided precedence, and because dedup keeps the first
/// occurrence the configured chain could never reorder what the scan had
/// already found.
///
/// Callers: registration gates for `analyze_image` / `analyze_video`, and
/// `file_extract`'s "can this be read as an image" check. None of them cares
/// about order; they care whether anything exists.
///
/// `enabled = false` is NOT a filter. That flag governs chat; seeing needs
/// only an endpoint and a `vision_model`.
pub fn any_provider_vision(config: &Config) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let push = |cand: (String, String, String), out: &mut Vec<(String, String, String)>| {
        if !out.contains(&cand) {
            out.push(cand);
        }
    };

    for reg in REGISTRATIONS.iter() {
        if let Some(cfg) = (reg.config_field)(config)
            && let Some(vm) = &cfg.vision_model
            && let Some(cand) = builtin_vision_candidate(reg.session_id, cfg, vm)
        {
            push(cand, &mut out);
        }
    }
    if let Some(customs) = &config.providers.custom {
        for cfg in customs.values() {
            if let Some(vm) = &cfg.vision_model
                && let Some(cand) = custom_vision_candidate(cfg, vm)
            {
                push(cand, &mut out);
            }
        }
    }
    for name in normalized_vision_chain(config) {
        if let Some(cand) = vision_by_name(config, &name) {
            push(cand, &mut out);
        }
    }
    out
}

/// Ordered vision candidates `(api_key, base_url, vision_model)`, walked at
/// REQUEST time by `ProviderVisionTool` (#430): if one candidate's call
/// fails, the next is tried, and Gemini `[image.vision]` is the FINAL
/// fallback only — never the primary when any provider can serve vision.
///
/// Order (owner contract, whole app lifetime):
/// 1. Every provider with a `vision_model` AND a usable key, in
///    REGISTRATIONS priority order, then customs. `enabled` is the CHAT
///    gate and is never consulted (#401). "Usable key" = non-empty, or a
///    local endpoint that needs none.
/// 2. The `[providers.fallback] vision` chain, minus entries already
///    collected by the scan.
///
/// Candidates whose endpoint cannot be derived (no explicit `base_url`, no
/// known per-provider default) are SKIPPED: guessing OpenAI sent a Xiaomi
/// token-plan key to api.openai.com and 401'd every call (#430).
/// Ordered vision candidates for a session: current provider, then chain.
///
/// Order is the whole contract (#1318):
///
/// 1. the session's CURRENT provider, when it carries a `vision_model`
/// 2. `[providers.fallback] vision`, in the order written
/// 3. Gemini, applied by the caller after every candidate fails
///
/// What this deliberately does NOT do is scan every provider that happens to
/// carry a `vision_model`. That scan used to run FIRST and, because dedup
/// keeps the first occurrence, a provider it found held its scan position and
/// the configured chain could never reorder anything — the chain only ever
/// contributed entries the scan had missed. Measured cost: four candidates
/// tried and failed before the working one, ~2.5 s on every image, with
/// providers in the chain never reached.
///
/// `enabled = false` is NOT a filter here. That flag governs whether a
/// provider serves CHAT; a vision candidate needs only a reachable endpoint
/// and a `vision_model`.
///
/// Takes `config` per call rather than being computed once at startup: the
/// list must follow a `config.toml` edit without a restart, the same way the
/// Telegram governors read `Config::current()` on every gate evaluation.
pub fn vision_candidates_for(
    config: &Config,
    session_provider: Option<&str>,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let push = |cand: (String, String, String), out: &mut Vec<(String, String, String)>| {
        if !out.contains(&cand) {
            out.push(cand);
        }
    };

    // 1. The session's current provider, if it can see.
    if let Some(name) = session_provider
        && let Some(cand) = vision_by_name(config, name)
    {
        push(cand, &mut out);
    }

    // 2. The configured chain, in order. Dedup keeps position 1 when the
    //    current provider also appears here, which is what we want.
    for name in normalized_vision_chain(config) {
        if let Some(cand) = vision_by_name(config, &name) {
            push(cand, &mut out);
        }
    }
    out
}

/// Chain-only resolution, for callers with no session in hand.
pub fn vision_candidates(config: &Config) -> Vec<(String, String, String)> {
    vision_candidates_for(config, None)
}

/// Vision base URL for a built-in provider, derived the way the chat
/// factory derives it (#430): endpoint_type variants and per-provider
/// defaults, NEVER assuming an empty `base_url` means OpenAI. `None` =
/// endpoint unknown, candidate is skipped.
fn vision_base_url(session_id: &str, cfg: &ProviderConfig) -> Option<String> {
    if session_id == "xiaomi" {
        // endpoint_type takes precedence over base_url (mirrors
        // try_create_xiaomi).
        if cfg.endpoint_type.as_deref() == Some("token-plan") {
            return Some(XIAOMI_TOKEN_PLAN_URL.to_string());
        }
        return Some(
            cfg.base_url
                .clone()
                .unwrap_or_else(|| XIAOMI_DEFAULT_BASE_URL.to_string()),
        );
    }
    if let Some(url) = cfg.base_url.clone() {
        return Some(url);
    }
    match session_id {
        "openai" => Some("https://api.openai.com/v1".to_string()),
        // Anthropic's OpenAI-compatible layer serves /v1/chat/completions.
        "anthropic" => Some("https://api.anthropic.com/v1".to_string()),
        "openrouter" => Some("https://openrouter.ai/api/v1".to_string()),
        "minimax" => Some("https://api.minimax.io/v1".to_string()),
        "qwen" => Some(QWEN_DEFAULT_DASHSCOPE_URL.to_string()),
        "ollama" => Some("http://localhost:11434/v1".to_string()),
        // CLI wrappers, native protocols, and anything else without an
        // explicit base_url: no OpenAI-compatible endpoint to call.
        _ => None,
    }
}

/// Candidate for a built-in provider: derived endpoint + usable key.
fn builtin_vision_candidate(
    session_id: &str,
    cfg: &ProviderConfig,
    vision_model: &str,
) -> Option<(String, String, String)> {
    let base_url = normalize_vision_url(vision_base_url(session_id, cfg)?);
    let api_key = cfg.api_key.clone().filter(|k| !k.is_empty());
    if api_key.is_none() && !is_local_base_url(&base_url) {
        return None;
    }
    Some((
        api_key.unwrap_or_default(),
        base_url,
        vision_model.to_string(),
    ))
}

/// Candidate for a custom provider: customs carry their own `base_url`
/// (skipped when missing — never guessed) and may be keyless local
/// endpoints (Ollama, llama.cpp, LM Studio).
fn custom_vision_candidate(
    cfg: &ProviderConfig,
    vision_model: &str,
) -> Option<(String, String, String)> {
    let base_url = normalize_vision_url(cfg.base_url.clone()?);
    Some((
        cfg.api_key.clone().unwrap_or_default(),
        base_url,
        vision_model.to_string(),
    ))
}

fn normalize_vision_url(base_url: String) -> String {
    if base_url.contains("/chat/completions") {
        base_url
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    }
}

/// Look up a single provider by `name` (REGISTRATIONS session_id / alias,
/// or custom map key) and return its `(api_key, base_url, vision_model)`
/// if it has a `vision_model` configured. `enabled` is deliberately NOT
/// checked (#401): it gates chat usage only — vision needs a vision_model
/// and a valid key, which is what `vision_tuple` extracts.
fn vision_by_name(config: &Config, name: &str) -> Option<(String, String, String)> {
    // Custom entries take precedence (same convention as create_fallback).
    if !name.starts_with("custom:")
        && config
            .providers
            .custom
            .as_ref()
            .is_some_and(|m| m.contains_key(name))
    {
        let cfg = config.providers.custom.as_ref()?.get(name)?;
        if let Some(vm) = &cfg.vision_model {
            return custom_vision_candidate(cfg, vm);
        }
        return None;
    }
    if let Some(custom_name) = name.strip_prefix("custom:") {
        let cfg = config.providers.custom.as_ref()?.get(custom_name)?;
        if let Some(vm) = &cfg.vision_model {
            return custom_vision_candidate(cfg, vm);
        }
        return None;
    }
    // Built-in registry — endpoint derived per provider, never guessed.
    for reg in REGISTRATIONS.iter() {
        if reg.session_id == name || reg.aliases.contains(&name) {
            let cfg = (reg.config_field)(config)?;
            if let Some(vm) = &cfg.vision_model {
                return builtin_vision_candidate(reg.session_id, cfg, vm);
            }
            return None;
        }
    }
    None
}

/// Returns `(api_key, base_url, generation_model)` for the active provider
/// when it has `generation_model` set in its config. Used by cli/ui.rs to
/// register `generate_image` against a non-Gemini, OpenAI-compatible images
/// endpoint (`/v1/images/generations`) instead of the global Gemini default.
///
/// `base_url` is normalized so the caller can append the trailing
/// `/images/generations` segment without worrying whether the user typed
/// `https://openrouter.ai/api/v1` or `.../v1/chat/completions`.
pub fn active_provider_generation(config: &Config) -> Option<(String, String, String)> {
    let (name, _) = config.providers.active_provider_and_model();

    let custom_cfg = if !name.starts_with("custom:")
        && config
            .providers
            .custom
            .as_ref()
            .is_some_and(|m| m.contains_key(name.as_str()))
    {
        config
            .providers
            .custom
            .as_ref()
            .and_then(|m| m.get(&name))
            .cloned()
    } else if let Some(custom_name) = name.strip_prefix("custom:") {
        config
            .providers
            .custom
            .as_ref()
            .and_then(|m| m.get(custom_name))
            .cloned()
    } else {
        None
    };

    let native_cfg: Option<&ProviderConfig> = REGISTRATIONS
        .iter()
        .find(|reg| reg.session_id == name || reg.aliases.contains(&name.as_str()))
        .and_then(|reg| (reg.config_field)(config));

    let active_cfg = custom_cfg.as_ref().or(native_cfg);

    if let Some(cfg) = active_cfg
        && let Some(generation_model) = &cfg.generation_model
    {
        // Gate on `generation_model`, not the API key, so keyless and local
        // providers (Ollama, llama.cpp, LM Studio, etc.) still register
        // `generate_image`. Same key resolution as the vision path: a real user
        // key wins; any keyless/local provider gets an empty Bearer.
        let api_key = cfg
            .api_key
            .clone()
            .filter(|k| !k.is_empty())
            .unwrap_or_default();
        // Strip any chat-specific trailing segment so the caller can append
        // `/images/generations` cleanly. Accepts the three URL shapes users
        // typically paste into custom-provider config: `…/v1`,
        // `…/v1/chat/completions`, or the bare host root.
        let raw = cfg
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let base_url = generation_root_url(&raw);
        return Some((api_key, base_url, generation_model.clone()));
    }
    None
}

/// Final model name to hand to `GenerateImageTool` — first resolved
/// generation candidate (session provider's `generation_model`, then the
/// `[providers.fallback] generation` chain) wins, otherwise fall back to the
/// global `image.generation.model`. Keeps the resolution decision next to
/// its vision counterpart so call sites stay plain wiring.
pub fn effective_generation_model(config: &Config) -> String {
    generation_candidates_for(config, None)
        .first()
        .map(|(_, _, m)| m.clone())
        .unwrap_or_else(|| config.image.generation.model.clone())
}

/// Normalise an endpoint for the OpenAI-style `/images/generations` route:
/// strip a chat suffix and trailing slashes so callers can append the
/// images segment. Accepts the three URL shapes users paste into
/// provider config: `…/v1`, `…/v1/chat/completions`, or the bare host root.
fn generation_root_url(url: &str) -> String {
    url.trim_end_matches("/chat/completions")
        .trim_end_matches('/')
        .to_string()
}

/// Marker endpoint for the built-in Gemini provider's `generation_model`.
/// `GenerateImageTool` detects the Google host marker and routes through
/// the native Gemini wire (which builds its own URL); this string exists
/// so the candidate carries the right backend signal, it is never called
/// as-is.
const GEMINI_GENERATION_MARKER_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Generation endpoint for a built-in provider, derived the way the vision
/// path derives it (#430 rule, applied to generation for #1672): explicit
/// `base_url` first, then known per-provider defaults, and NEVER guessing
/// OpenAI for an unknown id. `None` = endpoint unknown, candidate skipped.
fn generation_base_url(session_id: &str, cfg: &ProviderConfig) -> Option<String> {
    let derived = if let Some(url) = cfg.base_url.clone() {
        url
    } else {
        match session_id {
            "openai" => "https://api.openai.com/v1".to_string(),
            // Anthropic's OpenAI-compatible layer.
            "anthropic" => "https://api.anthropic.com/v1".to_string(),
            "openrouter" => "https://openrouter.ai/api/v1".to_string(),
            "minimax" => "https://api.minimax.io/v1".to_string(),
            "qwen" => QWEN_DEFAULT_DASHSCOPE_URL.to_string(),
            "ollama" => "http://localhost:11434/v1".to_string(),
            "gemini" => GEMINI_GENERATION_MARKER_URL.to_string(),
            // CLI wrappers, native protocols, and anything else without an
            // explicit base_url: no derivable images endpoint.
            _ => return None,
        }
    };
    Some(generation_root_url(&derived))
}

/// Candidate for a built-in provider: derived endpoint + usable key.
/// A real key wins; keyless only counts when the endpoint is local
/// (Ollama diffusion and friends). Chat `enabled` is NOT consulted —
/// same contract as the vision path (#401).
fn builtin_generation_candidate(
    session_id: &str,
    cfg: &ProviderConfig,
    generation_model: &str,
) -> Option<(String, String, String)> {
    let base_url = generation_base_url(session_id, cfg)?;
    let api_key = cfg.api_key.clone().filter(|k| !k.is_empty());
    if api_key.is_none() && !is_local_base_url(&base_url) {
        return None;
    }
    Some((
        api_key.unwrap_or_default(),
        base_url,
        generation_model.to_string(),
    ))
}

/// Candidate for a custom provider: customs carry their own `base_url`
/// (skipped when missing — never guessed) and may be keyless local
/// endpoints (Ollama, llama.cpp, LM Studio, SD webui).
fn custom_generation_candidate(
    cfg: &ProviderConfig,
    generation_model: &str,
) -> Option<(String, String, String)> {
    let base_url = generation_root_url(&cfg.base_url.clone()?);
    Some((
        cfg.api_key.clone().unwrap_or_default(),
        base_url,
        generation_model.to_string(),
    ))
}

/// Look up a single provider by `name` (REGISTRATIONS session_id / alias,
/// or custom map key, with or without the `custom:` prefix) and return its
/// `(api_key, base_url, generation_model)` when it carries a
/// `generation_model`. Mirrors `vision_by_name` exactly, including custom
/// entries shadowing same-named built-ins.
fn generation_by_name(config: &Config, name: &str) -> Option<(String, String, String)> {
    // Custom entries take precedence (same convention as create_fallback).
    if !name.starts_with("custom:")
        && config
            .providers
            .custom
            .as_ref()
            .is_some_and(|m| m.contains_key(name))
    {
        let cfg = config.providers.custom.as_ref()?.get(name)?;
        return cfg
            .generation_model
            .as_ref()
            .and_then(|gm| custom_generation_candidate(cfg, gm));
    }
    if let Some(custom_name) = name.strip_prefix("custom:") {
        let cfg = config.providers.custom.as_ref()?.get(custom_name)?;
        return cfg
            .generation_model
            .as_ref()
            .and_then(|gm| custom_generation_candidate(cfg, gm));
    }
    // Built-in registry — endpoint derived per provider, never guessed.
    for reg in REGISTRATIONS.iter() {
        if reg.session_id == name || reg.aliases.contains(&name) {
            let cfg = (reg.config_field)(config)?;
            return cfg
                .generation_model
                .as_ref()
                .and_then(|gm| builtin_generation_candidate(reg.session_id, cfg, gm));
        }
    }
    None
}

/// Ordered generation candidates `(api_key, base_url, generation_model)`,
/// walked at REQUEST time by `GenerateImageTool` (#1672).
///
/// Order is the vision contract copied verbatim (#1318):
/// 1. the session's CURRENT provider when it carries a `generation_model`
///    (no session in hand: the config's active provider, preserving the
///    pre-#1672 behavior for callers without session context);
/// 2. `[providers.fallback] generation`, in the order written, deduped;
/// 3. the global Gemini `[image.generation]` fallback — applied by the
///    caller after every candidate fails, never primary.
pub fn generation_candidates_for(
    config: &Config,
    session_provider: Option<&str>,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let push = |cand: (String, String, String), out: &mut Vec<(String, String, String)>| {
        if !out.contains(&cand) {
            out.push(cand);
        }
    };

    match session_provider {
        Some(name) => {
            if let Some(cand) = generation_by_name(config, name) {
                push(cand, &mut out);
            }
        }
        None => {
            // No session in hand: the config's active provider, resolved
            // through the SAME never-guess path as chain entries —
            // `active_provider_generation`'s api.openai.com default would
            // point a keyless local provider (Ollama diffusion) at a
            // public endpoint and 401 every roll (#430 rule, applied to
            // generation in #1672).
            let (active, _) = config.providers.active_provider_and_model();
            if let Some(cand) = generation_by_name(config, &active) {
                push(cand, &mut out);
            }
        }
    }
    for name in normalized_generation_chain(config) {
        if let Some(cand) = generation_by_name(config, &name) {
            push(cand, &mut out);
        }
    }
    out
}

/// Capability answer — "can ANY provider generate images?" — deliberately
/// unordered, the same split `any_provider_vision` draws (#1318's lesson:
/// capability scan must never feed precedence). Used by the registration
/// gate so a provider-based generation route registers `generate_image`
/// even when the Gemini `[image.generation]` flag is off.
pub fn any_provider_generation(config: &Config) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let push = |cand: (String, String, String), out: &mut Vec<(String, String, String)>| {
        if !out.contains(&cand) {
            out.push(cand);
        }
    };

    for reg in REGISTRATIONS.iter() {
        if let Some(cfg) = (reg.config_field)(config)
            && let Some(gm) = &cfg.generation_model
            && let Some(cand) = builtin_generation_candidate(reg.session_id, cfg, gm)
        {
            push(cand, &mut out);
        }
    }
    if let Some(customs) = &config.providers.custom {
        for cfg in customs.values() {
            if let Some(gm) = &cfg.generation_model
                && let Some(cand) = custom_generation_candidate(cfg, gm)
            {
                push(cand, &mut out);
            }
        }
    }
    for name in normalized_generation_chain(config) {
        if let Some(cand) = generation_by_name(config, &name) {
            push(cand, &mut out);
        }
    }
    out
}

// ── Individual provider factory functions ───────────────────────

/// Try to create GitHub Copilot provider if configured.
/// Uses OAuth token to exchange for short-lived Copilot API tokens.
fn try_create_github(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    use super::copilot::{COPILOT_CHAT_URL, CopilotTokenManager, copilot_extra_headers};

    let github_config = match &config.providers.github {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let oauth_token = github_config.api_key.clone().filter(|k| !k.is_empty());

    let Some(oauth_token) = oauth_token else {
        tracing::warn!(
            "GitHub Copilot enabled but no OAuth token found. \
             Run /onboard:provider to authenticate."
        );
        return Ok(None);
    };

    // Create the token manager — background task does the initial + recurring refresh
    let manager = Arc::new(CopilotTokenManager::new(oauth_token));
    manager.clone().start_background_refresh();

    // Build a token_fn closure that reads the cached Copilot token
    let mgr_clone = manager.clone();
    let token_fn: super::custom_openai_compatible::TokenFn =
        Arc::new(move || mgr_clone.get_cached_token());

    let base_url = github_config
        .base_url
        .clone()
        .unwrap_or_else(|| COPILOT_CHAT_URL.to_string());

    tracing::info!("Using GitHub Copilot at: {}", base_url);

    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url("copilot-managed".to_string(), base_url)
            .with_name("GitHub Copilot")
            .with_token_fn(token_fn)
            .with_extra_headers(copilot_extra_headers()),
        github_config,
        &config.agent,
    );
    Ok(Some(Arc::new(provider)))
}

/// Default DashScope OpenAI-compatible chat-completions URL (China region).
/// Users can override via `[providers.qwen].base_url` for Singapore
/// (`dashscope-intl`), US (`dashscope-us`), or Coding Plan
/// (`coding.dashscope.aliyuncs.com/v1`).
const QWEN_DEFAULT_DASHSCOPE_URL: &str =
    "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";

/// Try to create the **Qwen** provider backed by a DashScope API key.
///
/// OAuth and multi-account rotation were removed after Alibaba discontinued
/// Qwen OAuth. The provider now behaves like any other OpenAI-compatible
/// key-based provider (zhipu, openai, …): `api_key` + `base_url` +
/// `default_model` from `[providers.qwen]`.
async fn try_create_qwen(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let qwen_config = match &config.providers.qwen {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let Some(api_key) = qwen_config.api_key.as_ref().filter(|k| !k.is_empty()) else {
        tracing::warn!("Qwen enabled but no API key configured — run /onboard:provider");
        return Ok(None);
    };

    let base_url = qwen_config
        .base_url
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| QWEN_DEFAULT_DASHSCOPE_URL.to_string());
    let base_url = if base_url.contains("/chat/completions") {
        base_url
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    };

    tracing::info!("Using Qwen (DashScope) at: {}", base_url);

    let qwen_limiter = Arc::clone(&super::rate_limiter::QWEN_OAUTH_LIMITER);

    // `enable_thinking` is NOT injected here. The switch is only the knob some
    // qwen families read, and a body transform cannot tell which family the
    // request targets — it never sees the model. Resolution happens per request
    // in `qwen_reasoning` instead, reached via `configure_openai_compatible`
    // below (#1034).
    let builder = OpenAIProvider::with_base_url(api_key.clone(), base_url)
        .with_name("qwen")
        .with_extra_headers(qwen_extra_headers())
        .with_body_transform(Arc::new(qwen_body_transform))
        .with_rate_limiter(qwen_limiter);

    let provider = configure_openai_compatible(builder, qwen_config, &config.agent);
    Ok(Some(Arc::new(provider)))
}

/// Try to create OpenRouter provider if configured
fn try_create_openrouter(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let openrouter_config = match &config.providers.openrouter {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let Some(api_key) = &openrouter_config.api_key else {
        tracing::warn!("OpenRouter enabled but API key missing — check keys.toml");
        return Ok(None);
    };

    let base_url = openrouter_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://openrouter.ai/api/v1/chat/completions".to_string());

    let model = openrouter_config
        .default_model
        .clone()
        .unwrap_or_else(|| "openai/gpt-4o".to_string());

    let is_free = model.ends_with(":free");

    tracing::info!(
        "Using OpenRouter at: {} (model={}, free={})",
        base_url,
        model,
        is_free
    );
    let mut provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url)
            .with_name("openrouter")
            .with_default_model(model.clone())
            .with_extra_headers(vec![
                ("X-Title".to_string(), "Open Crabs".to_string()),
                (
                    "HTTP-Referer".to_string(),
                    "https://opencrabs.com".to_string(),
                ),
            ]),
        openrouter_config,
        &config.agent,
    );

    // OpenRouter caches by default — turn it on unless the user explicitly opted
    // out, so non-technical users get caching with no flag to discover. (An
    // explicit cache_enabled value was already applied by configure_openai_compatible.)
    if openrouter_config.cache_enabled.is_none() {
        provider = provider.with_cache_enabled(true);
    }

    // Proactive pacing for :free models — per-model rate limiter shared across
    // all provider instances. Enforces 4s between requests per model (~15 req/min,
    // safely under OpenRouter's 20 req/min window with 25% headroom).
    if is_free {
        let limiter = super::rate_limiter::OPENROUTER_FREE_LIMITERS.get(&model);
        provider = provider.with_rate_limiter(limiter);
        tracing::info!("Rate limiter attached for :free model: {}", model);
    }

    Ok(Some(Arc::new(provider)))
}

/// Try to create Minimax provider if configured
/// Default endpoint for Xiaomi MiMo's OpenAI-compatible API. Overridable via
/// `[providers.xiaomi] base_url`. Users get an API key at platform.xiaomimimo.com.
const XIAOMI_DEFAULT_BASE_URL: &str = "https://api.xiaomimimo.com/v1/chat/completions";
const XIAOMI_TOKEN_PLAN_URL: &str = "https://token-plan-ams.xiaomimimo.com/v1/chat/completions";

/// Xiaomi MiMo. OpenAI-compatible, keyed: the user supplies an API key from
/// platform.xiaomimimo.com. With no key the provider stays unconfigured so the
/// user is steered to add one (or pick another provider).
///
/// Supports two endpoint types:
/// - "api"        → https://api.xiaomimimo.com/v1/chat/completions
/// - "token-plan" → https://token-plan-ams.xiaomimimo.com/v1/chat/completions
fn try_create_xiaomi(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let xiaomi_config = match &config.providers.xiaomi {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let api_key = match xiaomi_config.api_key.clone().filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => {
            tracing::debug!(
                "Xiaomi has no api_key; add one in keys.toml ([providers.xiaomi] api_key) to enable it"
            );
            return Ok(None);
        }
    };

    // Determine base URL based on endpoint_type
    // API endpoint: https://api.xiaomimimo.com/v1
    // Token Plan endpoint: https://token-plan-ams.xiaomimimo.com/v1
    let base_url = match xiaomi_config.endpoint_type.as_deref() {
        Some("token-plan") => XIAOMI_TOKEN_PLAN_URL.to_string(),
        _ => {
            // Fall back to explicit base_url or the default
            let url = xiaomi_config
                .base_url
                .clone()
                .unwrap_or_else(|| XIAOMI_DEFAULT_BASE_URL.to_string());
            if url.contains("/chat/completions") {
                url
            } else {
                format!("{}/chat/completions", url.trim_end_matches('/'))
            }
        }
    };

    // Thinking is ON by default — Xiaomi streams reasoning_content and the API
    // defaults to thinking enabled. Only disable it when the user opts out with
    // enable_thinking = false in [providers.xiaomi]; Xiaomi's switch is
    // {"thinking": {"type": "disabled"}} in the request body.
    let enable_thinking = xiaomi_config.enable_thinking.unwrap_or(true);
    tracing::info!("Using Xiaomi at: {base_url} (thinking: {enable_thinking})");
    let mut builder = OpenAIProvider::with_base_url(api_key, base_url).with_name("xiaomi");
    if !enable_thinking {
        builder = builder.with_body_transform(Arc::new(|mut body| {
            if let Some(obj) = body.as_object_mut() {
                obj.insert(
                    "thinking".to_string(),
                    serde_json::json!({ "type": "disabled" }),
                );
            }
            body
        }));
    }

    // Caching: Xiaomi caches prompt prefixes automatically (server-side, surfaced
    // via prompt_tokens_details) — there is no request-side cache parameter, so
    // we deliberately do NOT set cache_enabled (which would send an
    // OpenRouter/Anthropic-style cache_control Xiaomi doesn't accept).
    let provider = configure_openai_compatible(builder, xiaomi_config, &config.agent);
    Ok(Some(Arc::new(provider)))
}

fn try_create_minimax(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let minimax_config = match &config.providers.minimax {
        Some(cfg) => {
            tracing::debug!(
                "Minimax config: enabled={}, has_key={}",
                cfg.enabled,
                cfg.api_key.is_some()
            );
            cfg
        }
        None => return Ok(None),
    };

    let Some(api_key) = &minimax_config.api_key else {
        tracing::warn!("Minimax enabled but API key missing — check keys.toml");
        return Ok(None);
    };

    // MiniMax requires specific endpoint path, not just /v1
    let base_url = minimax_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.minimax.io/v1".to_string());

    // Append correct path if not already present
    let full_url = if base_url.contains("minimax.io") && !base_url.contains("/text/") {
        format!("{}/text/chatcompletion_v2", base_url.trim_end_matches('/'))
    } else {
        base_url
    };

    tracing::info!("Using Minimax at: {}", full_url);
    let mut provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), full_url).with_name("minimax"),
        minimax_config,
        &config.agent,
    );

    // MiniMax M2.7/M2.5 doesn't support vision — default to MiniMax-Text-01 in-memory.
    // Do NOT write to config here — this runs inside ConfigWatcher callbacks and
    // writing triggers another reload → infinite loop → crash.
    if minimax_config.vision_model.is_none() {
        provider = provider.with_vision_model("MiniMax-Text-01".to_string());
    }

    Ok(Some(Arc::new(provider)))
}

/// Try to create the z.ai provider if configured
/// Supports two endpoint types: "api" (general) or "coding" (coding-specific)
fn try_create_zhipu(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let zhipu_config = match &config.providers.zai {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let Some(api_key) = &zhipu_config.api_key else {
        tracing::warn!("z.ai enabled but API key missing — check keys.toml");
        return Ok(None);
    };

    // A configured base_url wins (open.bigmodel.cn has no idle cut on long
    // streams, api.z.ai does); otherwise the endpoint_type default (#1350).
    let base_url = super::zhipu_endpoint::chat_url(
        zhipu_config.base_url.as_deref(),
        zhipu_config.endpoint_type.as_deref(),
    );

    tracing::info!(
        "Using z.ai at: {} (endpoint_type: {:?}, base_url configured: {})",
        base_url,
        zhipu_config.endpoint_type,
        zhipu_config.base_url.is_some()
    );
    let mut provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url).with_name("zai"),
        zhipu_config,
        &config.agent,
    );
    // Neither tier configured an idle timeout, so the host-aware default is the
    // only thing standing between z.ai and the generic 20s remote default, which
    // would cut a host the code documents as holding an idle stream to ~30s and
    // blame the connection for our own timer (#1666). Resolved across BOTH tiers
    // (#1688): reading only the per-provider key here would let this default
    // overwrite an explicit `[agent] stream_idle_timeout_secs`.
    let idle_cfg = resolve_timeout(
        zhipu_config.stream_idle_timeout_secs,
        config.agent.stream_idle_timeout_secs,
        None,
    );
    let host_aware_idle = idle_cfg
        .effective
        .is_none()
        .then(|| {
            super::zhipu_endpoint::default_idle_timeout_secs(
                zhipu_config.base_url.as_deref(),
                zhipu_config.endpoint_type.as_deref(),
            )
        })
        .flatten();
    if let Some(secs) = host_aware_idle {
        provider = provider.with_stream_idle_timeout(std::time::Duration::from_secs(secs));
        tracing::info!("z.ai host-aware stream idle timeout: {}s", secs);
    }
    Ok(Some(Arc::new(provider)))
}

/// Try to create Moonshot AI (Kimi) provider if configured.
/// Supports two endpoint types:
/// - `"api"` (default): platform pay-per-token at `api.moonshot.ai`
/// - `"coding"`: Kimi Code token plan at `api.kimi.com/coding/v1`
///   (models: k3, kimi-for-coding, kimi-for-coding-highspeed)
fn try_create_moonshot(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let moonshot_config = match &config.providers.moonshot {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let Some(api_key) = &moonshot_config.api_key else {
        tracing::warn!("Moonshot AI enabled but API key missing — check keys.toml");
        return Ok(None);
    };

    // User-set base_url wins; otherwise pick by endpoint_type.
    let base_url = moonshot_config
        .base_url
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| match moonshot_config.endpoint_type.as_deref() {
            Some("coding") => "https://api.kimi.com/coding/v1/chat/completions".to_string(),
            _ => "https://api.moonshot.ai/v1/chat/completions".to_string(),
        });
    let base_url = if base_url.contains("/chat/completions") {
        base_url
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    };

    tracing::info!(
        "Using Moonshot AI at: {} (endpoint_type: {:?})",
        base_url,
        moonshot_config.endpoint_type
    );
    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url).with_name("moonshot"),
        moonshot_config,
        &config.agent,
    );
    Ok(Some(Arc::new(provider)))
}

/// Try to create Ollama provider if configured.
/// Supports both local (localhost:11434, no API key) and cloud (api.ollama.com, optional key).
fn try_create_ollama(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let ollama_config = match &config.providers.ollama {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    // Default to local Ollama if no base_url specified
    let base_url = ollama_config
        .base_url
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://localhost:11434/v1/chat/completions".to_string());

    let base_url = if base_url.contains("/chat/completions") {
        base_url
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    };

    // API key is optional for local Ollama
    let api_key = ollama_config.api_key.clone().unwrap_or_default();

    tracing::info!(
        "Using Ollama at: {} (has_key={})",
        base_url,
        !api_key.is_empty()
    );

    let mut builder = OpenAIProvider::with_base_url(api_key, base_url.clone()).with_name("ollama");
    if is_local_base_url(&base_url) {
        let enable = ollama_config.enable_thinking.unwrap_or(true);
        builder = builder.with_body_transform(local_thinking_body_transform(enable));
    }
    let provider = configure_openai_compatible(builder, ollama_config, &config.agent);
    Ok(Some(Arc::new(provider)))
}

/// Try to create Custom OpenAI-compatible provider if configured.
/// Picks the first enabled named custom provider from the map.
fn try_create_custom(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let (name, custom_config) = match config.providers.active_custom() {
        Some((n, c)) => (n.to_string(), c.clone()),
        None => {
            tracing::warn!("Custom provider requested but no active custom provider found");
            return Ok(None);
        }
    };

    // API key is optional for local providers (LM Studio, Ollama, etc.)
    let api_key = custom_config.api_key.clone().unwrap_or_default();

    // Same rationale as try_create_custom_by_name: refuse to silently default
    // to localhost:1234 so Discord / other channels don't hit a dead
    // LM Studio URL the user never configured.
    let Some(mut base_url) = custom_config.base_url.clone() else {
        tracing::warn!(
            "Custom provider '{}' has no base_url configured — skipping (run /onboard:provider)",
            name
        );
        return Ok(None);
    };

    // Auto-append /chat/completions if missing — all OpenAI-compatible APIs need it
    if !base_url.contains("/chat/completions") {
        base_url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    }

    tracing::info!(
        "Using Custom OpenAI-compatible '{}' at: {} (has_key={})",
        name,
        base_url,
        !api_key.is_empty()
    );
    let mut builder = OpenAIProvider::with_base_url(api_key, base_url.clone()).with_name(&name);

    // Mirror try_create_custom_by_name: thinking flag for local servers,
    // then auto-qwen cache markers for Qwen / Alibaba targets.
    let cache_transform = auto_qwen_cache_transform(base_url.clone());
    let combined_transform = if is_local_base_url(&base_url) {
        let enable = custom_config.enable_thinking.unwrap_or(true);
        chain_body_transforms(local_thinking_body_transform(enable), cache_transform)
    } else {
        cache_transform
    };
    builder = builder.with_body_transform(combined_transform);

    let provider = configure_openai_compatible(builder, &custom_config, &config.agent);
    Ok(Some(Arc::new(provider)))
}

/// Warn when a resolved timeout flag had to skip a tier that carried `0`.
///
/// `0` is not "no timer" for these two flags: a zero-second reqwest timeout is
/// an instant deadline, and a zero-second idle timer fires before the first
/// chunk arrives. So the resolver skips it and the default applies. Skipping it
/// *silently*, though, is the same invisibility that kept #1689's dropped keys
/// unnoticed long enough to become a documented example in the README, so the
/// skip gets a log line naming the section that carried it.
fn warn_skipped_zero(resolution: &TimeoutResolution, flag: &str) {
    if let Some(note) = resolution.zero_note() {
        tracing::warn!(
            "`{flag} = 0` is skipped ({note}); a zero-second timer would fire \
             immediately, so the default applies instead"
        );
    }
}

/// Configure OpenAI-compatible provider with custom model
fn configure_openai_compatible(
    mut provider: OpenAIProvider,
    config: &ProviderConfig,
    agent: &AgentConfig,
) -> OpenAIProvider {
    tracing::debug!(
        "configure_openai_compatible: default_model = {:?}",
        config.default_model
    );
    if let Some(model) = &config.default_model {
        tracing::info!("Using custom default model: {}", model);
        provider = provider.with_default_model(model.clone());
    }
    if let Some(vm) = &config.vision_model {
        tracing::info!("Vision model configured: {}", vm);
        provider = provider.with_vision_model(vm.clone());
    }
    if let Some(cw) = config.context_window {
        tracing::info!("Context window configured: {} tokens", cw);
        provider = provider.with_context_window(cw);
    } else if let Some(plan) = &config.plan
        // The pay-per-token API plan has no tier; only coding-plan (or a
        // custom Kimi) provider derives its window from the subscription tier.
        && config.endpoint_type.as_deref() != Some("api")
        && let Some(cw) =
            super::kimi_plan::context_window_for_plan(plan, config.default_model.as_deref())
    {
        tracing::info!("Kimi plan '{}' derives context window: {} tokens", plan, cw);
        provider = provider.with_context_window(cw);
    }
    if let Some(reasoning) = &config.reasoning_effort {
        tracing::info!("Kimi reasoning setting configured: {}", reasoning);
        provider = provider.with_reasoning(reasoning.clone());
    }
    if let Some(enable) = config.enable_thinking {
        // Only DashScope targets consume this at request time; local servers
        // take thinking through `chat_template_kwargs` and ignore it.
        provider = provider.with_enable_thinking(enable);
    }
    if !config.models.is_empty() {
        tracing::debug!(
            "Loaded {} configured models for provider",
            config.models.len()
        );
        provider = provider.with_models(config.models.clone());
    }
    // OpenRouter response caching
    if let Some(cache) = config.cache_enabled {
        provider = provider.with_cache_enabled(cache);
        if cache {
            tracing::info!("OpenRouter response caching enabled");
        }
    }
    if let Some(ttl) = config.cache_ttl {
        provider = provider.with_cache_ttl(ttl);
        tracing::info!("OpenRouter cache TTL: {}s", ttl);
    }
    // #1688: both flags resolve through [providers.<name>] -> [agent] -> the
    // family default. Before this the per-provider struct was the only tier
    // read anywhere in the tree, so `[agent] timeout_secs = 120` was parsed,
    // stored, and then quietly ignored.
    let request = resolve_timeout(
        config.timeout_secs,
        agent.timeout_secs,
        Some(super::custom_openai_compatible::DEFAULT_TIMEOUT.as_secs()),
    );
    if let Some(dur) = request.duration() {
        provider = provider.with_timeout(dur);
        tracing::info!(
            "Non-streaming request ceiling: {}s (from {})",
            dur.as_secs(),
            request.tier
        );
    }
    warn_skipped_zero(&request, "timeout_secs");

    // No compiled floor on purpose: the idle default is picked at stream time
    // from whether the target is local/CLI or remote (`helpers.rs`), so a flag
    // unset at both tiers must defer to that runtime choice, not override it.
    let idle = resolve_timeout(
        config.stream_idle_timeout_secs,
        agent.stream_idle_timeout_secs,
        None,
    );
    if let Some(dur) = idle.duration() {
        provider = provider.with_stream_idle_timeout(dur);
        tracing::info!(
            "Stream idle timeout: {}s (from {})",
            dur.as_secs(),
            idle.tier
        );
    }
    warn_skipped_zero(&idle, "stream_idle_timeout_secs");
    provider
}

/// Try to create OpenAI provider if configured
fn try_create_openai(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let openai_config = match &config.providers.openai {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    // Local LLM (LM Studio, Ollama, etc.) - has base_url but NO api_key
    if let Some(base_url) = &openai_config.base_url
        && openai_config.api_key.is_none()
    {
        tracing::info!("Using local LLM at: {}", base_url);
        let mut builder = OpenAIProvider::local(base_url.clone()).with_name("openai");
        if is_local_base_url(base_url) {
            let enable = openai_config.enable_thinking.unwrap_or(true);
            builder = builder.with_body_transform(local_thinking_body_transform(enable));
        }
        let provider = configure_openai_compatible(builder, openai_config, &config.agent);
        return Ok(Some(Arc::new(provider)));
    }

    // Official OpenAI API - has api_key
    if let Some(api_key) = &openai_config.api_key {
        tracing::info!("Using OpenAI provider");
        let provider = configure_openai_compatible(
            OpenAIProvider::new(api_key.clone()).with_name("openai"),
            openai_config,
            &config.agent,
        );
        return Ok(Some(Arc::new(provider)));
    }

    tracing::warn!("OpenAI enabled but no API key and no base_url — check keys.toml");
    Ok(None)
}

/// Try to create Gemini provider if configured
fn try_create_gemini(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let gemini_config = match &config.providers.gemini {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let api_key = match &gemini_config.api_key {
        Some(key) if !key.is_empty() => key.clone(),
        _ => {
            tracing::warn!("Gemini enabled but API key missing — check keys.toml");
            return Ok(None);
        }
    };

    let model = gemini_config
        .default_model
        .clone()
        .unwrap_or_else(|| "gemini-2.0-flash".to_string());

    tracing::info!("Using Gemini provider with model: {}", model);
    let mut provider = GeminiProvider::new(api_key).with_model(model);
    if let Some(cw) = gemini_config.context_window {
        tracing::info!("Gemini context window override: {} tokens", cw);
        provider = provider.with_context_window(cw);
    }
    Ok(Some(Arc::new(provider)))
}

/// Try to create Claude CLI provider if configured and binary is available.
fn try_create_claude_cli(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let cli_config = match &config.providers.claude_cli {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    match ClaudeCliProvider::new() {
        Ok(mut provider) => {
            if let Some(model) = &cli_config.default_model {
                provider = provider.with_default_model(model.clone());
            }
            if let Some(cw) = cli_config.context_window {
                provider = provider.with_context_window(cw);
            }
            tracing::info!("Using Claude CLI provider (Max subscription, no API key needed)");
            Ok(Some(Arc::new(provider)))
        }
        Err(e) => {
            tracing::warn!("Claude CLI enabled but binary not found: {}", e);
            Ok(None)
        }
    }
}

/// Try to create OpenCode CLI provider if configured and binary is available.
fn try_create_opencode_cli(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let cli_config = match &config.providers.opencode_cli {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    match OpenCodeCliProvider::new() {
        Ok(mut provider) => {
            if let Some(model) = &cli_config.default_model {
                provider = provider.with_default_model(model.clone());
            }
            if let Some(cw) = cli_config.context_window {
                provider = provider.with_context_window(cw);
            }
            tracing::info!("Using OpenCode CLI provider (free models, no API key needed)");
            Ok(Some(Arc::new(provider)))
        }
        Err(e) => {
            tracing::warn!("OpenCode CLI enabled but binary not found: {}", e);
            Ok(None)
        }
    }
}

/// Try to create Codex CLI provider if configured and binary is available.
fn try_create_codex_cli(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let cli_config = match &config.providers.codex_cli {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    match CodexCliProvider::new() {
        Ok(mut provider) => {
            if let Some(model) = &cli_config.default_model {
                provider = provider.with_default_model(model.clone());
            }
            if let Some(cw) = cli_config.context_window {
                provider = provider.with_context_window(cw);
            }
            tracing::info!(
                "Using Codex CLI provider (ChatGPT/Codex subscription, no API key needed)"
            );
            Ok(Some(Arc::new(provider)))
        }
        Err(e) => {
            tracing::warn!("Codex CLI enabled but binary not found: {}", e);
            Ok(None)
        }
    }
}

fn try_create_command_code_cli(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let cli_config = match &config.providers.command_code_cli {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    match CommandCodeCliProvider::new() {
        Ok(mut provider) => {
            if let Some(model) = &cli_config.default_model {
                provider = provider.with_default_model(model.clone());
            }
            if let Some(cw) = cli_config.context_window {
                provider = provider.with_context_window(cw);
            }
            tracing::info!(
                "Using Command Code CLI provider (Command Code account, no API key needed)"
            );
            Ok(Some(Arc::new(provider)))
        }
        Err(e) => {
            tracing::warn!("Command Code CLI enabled but binary not found: {}", e);
            Ok(None)
        }
    }
}

/// Try to create Codex OAuth provider if configured and tokens are available.
fn try_create_codex_oauth(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let codex_config = match &config.providers.codex {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    match CodexOAuthProvider::new() {
        Ok(mut provider) => {
            if let Some(model) = &codex_config.default_model {
                provider = provider.with_default_model(model.clone());
            }
            tracing::info!("Using Codex provider (OAuth authenticated, no API key needed)");
            Ok(Some(Arc::new(provider)))
        }
        Err(e) => {
            tracing::warn!("Codex OAuth enabled but not authenticated: {}", e);
            Ok(None)
        }
    }
}

/// Try to create OpenCode API provider if configured (Go/Zen plans)
async fn try_create_opencode(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let opencode_config = match &config.providers.opencode {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    let Some(api_key) = &opencode_config.api_key else {
        tracing::warn!("OpenCode enabled but API key missing — check keys.toml");
        return Ok(None);
    };

    let base_url = opencode_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://opencode.ai/zen/go/v1/chat/completions".to_string());

    let model = opencode_config
        .default_model
        .clone()
        .unwrap_or_else(|| "qwen3.6-plus".to_string());

    tracing::info!("Using OpenCode API at: {} (model={})", base_url, model);

    let provider = configure_openai_compatible(
        OpenAIProvider::with_base_url(api_key.clone(), base_url)
            .with_name("opencode")
            .with_default_model(model.clone()),
        opencode_config,
        &config.agent,
    );

    Ok(Some(Arc::new(provider)))
}

/// Try to create Anthropic provider if configured
fn try_create_anthropic(config: &Config) -> Result<Option<Arc<dyn Provider>>> {
    let anthropic_config = match &config.providers.anthropic {
        Some(cfg) => cfg,
        None => return Ok(None),
    };

    let api_key = match &anthropic_config.api_key {
        Some(key) => key.clone(),
        None => {
            tracing::warn!("Anthropic enabled but API key missing — check keys.toml");
            return Ok(None);
        }
    };

    let mut provider = AnthropicProvider::new(api_key);

    if let Some(model) = &anthropic_config.default_model {
        tracing::info!("Using custom default model: {}", model);
        provider = provider.with_default_model(model.clone());
    }
    if let Some(cw) = anthropic_config.context_window {
        tracing::info!("Anthropic context window override: {} tokens", cw);
        provider = provider.with_context_window(cw);
    }

    tracing::info!("Using Anthropic provider");

    Ok(Some(Arc::new(provider)))
}
