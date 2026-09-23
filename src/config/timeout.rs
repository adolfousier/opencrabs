//! Timeout flag resolution (#1688).
//!
//! Every timeout flag resolves through one chain:
//!
//! ```text
//! [providers.<name>]  ->  [agent]  ->  the family's compiled default
//! ```
//!
//! Before this, `timeout_secs` and `stream_idle_timeout_secs` were read in
//! exactly one place — `factory.rs`, inside `configure_openai_compatible` — and
//! against exactly one tier: the per-provider struct. A key set under `[agent]`
//! was parsed, stored, and silently ignored, and the two non-compat families
//! (`anthropic`, `gemini`) ignored both keys at every tier (#1689). The same
//! flag therefore meant three different things depending on which family a
//! provider happened to belong to, which is the shape of bug #1687 was filed
//! against.
//!
//! The resolution is deliberately pure: it takes the two configured values and
//! the family's compiled floor and returns what applies plus which tier said
//! so. No `Config::current()`, no disk, no globals — so precedence is testable
//! without a config file anywhere. The callers in `factory.rs` are the thin
//! edge that feeds it real values.

use std::time::Duration;

/// Which tier supplied an effective timeout value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutTier {
    /// `[providers.<name>]` — the most specific tier, wins over everything.
    Provider,
    /// `[agent]` — the global fallback for providers that name no tier of their
    /// own.
    Agent,
    /// Nothing usable was configured, so `compiled_default` applies — or, when
    /// the flag has no compiled floor, the runtime default applies (see
    /// [`TimeoutResolution::effective`]).
    Default,
}

impl std::fmt::Display for TimeoutTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            TimeoutTier::Provider => "[providers.<name>]",
            TimeoutTier::Agent => "[agent]",
            TimeoutTier::Default => "compiled default",
        };
        f.write_str(label)
    }
}

/// A resolved timeout flag: the value that applies, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutResolution {
    /// The seconds that apply.
    ///
    /// `None` means no tier configured a usable value AND the family has no
    /// compiled floor, so the caller must defer to the runtime default.
    /// Inter-chunk stream idle is that case: the right default depends on
    /// whether the target is local or a CLI provider, which is only known at
    /// stream time (`brain/agent/service/helpers.rs`), not at provider
    /// construction. Guessing a constant here would override a runtime choice
    /// the code has good reasons for.
    pub effective: Option<u64>,
    /// The tier `effective` came from. `TimeoutTier::Default` when it came from
    /// the compiled floor, and also when nothing at all is set.
    pub tier: TimeoutTier,
    /// `true` when `[providers.<name>]` carried `0` and was skipped.
    pub provider_zero: bool,
    /// `true` when `[agent]` carried `0` and was skipped.
    pub agent_zero: bool,
}

impl TimeoutResolution {
    /// `effective` as a `Duration`, for the builder call sites.
    pub fn duration(&self) -> Option<Duration> {
        self.effective.map(Duration::from_secs)
    }

    /// The value to *apply* as an override: `Some` only when a config tier said
    /// so, `None` when the compiled floor won or nothing is set at all.
    ///
    /// The distinction is the `Provider` trait's contract: `request_timeout()`
    /// and `stream_idle_timeout()` report "None when default timeouts apply"
    /// (`brain/provider/trait.rs`), and consumers branch on it — `helpers.rs`
    /// picks its runtime idle table when the accessor is `None`. Handing back
    /// the compiled floor would make a provider claim an override nobody
    /// configured, and would rebuild a client that already carries that exact
    /// number, so the wire behaviour is unchanged while the reported one lies.
    pub fn override_duration(&self) -> Option<Duration> {
        match self.tier {
            TimeoutTier::Provider | TimeoutTier::Agent => self.duration(),
            TimeoutTier::Default => None,
        }
    }

    /// What the caller should log about a skipped `0`, or `None` when no tier
    /// carried one.
    ///
    /// Returned as `&'static str` so every family emits the identical sentence
    /// and a user can grep the log for it. The sentence matters: a `0` that is
    /// silently dropped is the same class of invisibility as a key that is
    /// silently dropped, and the second one is what kept #1689 unnoticed.
    pub fn zero_note(&self) -> Option<&'static str> {
        match (self.provider_zero, self.agent_zero) {
            (true, true) => Some("[providers.<name>] and [agent] both carried 0"),
            (true, false) => Some("[providers.<name>] carried 0"),
            (false, true) => Some("[agent] carried 0"),
            (false, false) => None,
        }
    }
}

/// Resolve one timeout flag across the three tiers.
///
/// A configured `0` is skipped, not honoured. Every existing call site already
/// did this (`filter(|&s| s > 0)`), and for good reason: under reqwest a
/// zero-second request timeout is an instant deadline, so `timeout_secs = 0`
/// read literally turns a typo into "every call fails" instead of "the default
/// applies". Skipping is the safe direction — but skipping *silently* is not, so
/// the skipped tiers are reported on the result for the caller to warn about.
///
/// `compiled_default` is the family's own floor: `Some(300)` for the
/// non-streaming ceiling every family shares (`DEFAULT_TIMEOUT`), or `None` for
/// a flag whose default is decided at runtime.
///
/// ```text
/// resolve_timeout(Some(120), Some(60), Some(300)) -> 120  (Provider)
/// resolve_timeout(None,      Some(60), Some(300)) -> 60   (Agent)
/// resolve_timeout(None,      None,      Some(300)) -> 300 (Default)
/// resolve_timeout(Some(0),   None,      Some(300)) -> 300 (Default, provider_zero)
/// resolve_timeout(None,      None,      None)      -> None (Default, defer to runtime)
/// ```
pub fn resolve_timeout(
    provider_value: Option<u64>,
    agent_value: Option<u64>,
    compiled_default: Option<u64>,
) -> TimeoutResolution {
    let skipped = TimeoutResolution {
        effective: None,
        tier: TimeoutTier::Default,
        provider_zero: provider_value == Some(0),
        agent_zero: agent_value == Some(0),
    };

    if let Some(secs) = usable(provider_value) {
        return TimeoutResolution {
            effective: Some(secs),
            tier: TimeoutTier::Provider,
            ..skipped
        };
    }
    if let Some(secs) = usable(agent_value) {
        return TimeoutResolution {
            effective: Some(secs),
            tier: TimeoutTier::Agent,
            ..skipped
        };
    }
    TimeoutResolution {
        effective: compiled_default,
        ..skipped
    }
}

/// A configured value that can actually act as a timeout: present, and not `0`.
fn usable(value: Option<u64>) -> Option<u64> {
    value.filter(|&s| s > 0)
}
