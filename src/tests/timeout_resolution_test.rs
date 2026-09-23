//! Regression guards for #1688: timeout flags resolve through
//! `[providers.<name>]` → `[agent]` → the family's compiled default.
//!
//! Before this, `timeout_secs` and `stream_idle_timeout_secs` were read against
//! one tier only — the per-provider struct — at exactly one call site inside
//! `configure_openai_compatible`. Every other placement was invisible: a key
//! under `[agent]` was parsed, stored, and dropped, and the `anthropic` and
//! `gemini` families dropped both keys at every tier (#1689). The user-visible
//! symptom was two different clocks answering for the same turn depending on
//! which provider family happened to serve it.
//!
//! Precedence is pure logic, so it is tested directly. The wiring is a source
//! scan, because the realistic regression is a call site going back to reading
//! one tier by hand, and no amount of precedence testing catches that.

use crate::config::AgentConfig;
use crate::config::timeout::{TimeoutTier, resolve_timeout};

fn factory_source() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/brain/provider/factory.rs"),
    )
    .expect("factory.rs must be readable")
}

// ---------------------------------------------------------------- precedence

#[test]
fn provider_tier_wins_over_agent_and_default() {
    let r = resolve_timeout(Some(120), Some(60), Some(300));
    assert_eq!(r.effective, Some(120));
    assert_eq!(r.tier, TimeoutTier::Provider);
}

#[test]
fn agent_tier_applies_when_the_provider_names_none() {
    // This is the whole point of #1688: before it, this exact config produced
    // 300s because nothing ever read the `[agent]` tier.
    let r = resolve_timeout(None, Some(60), Some(300));
    assert_eq!(r.effective, Some(60));
    assert_eq!(r.tier, TimeoutTier::Agent);
}

#[test]
fn compiled_default_applies_when_neither_tier_is_set() {
    let r = resolve_timeout(None, None, Some(300));
    assert_eq!(r.effective, Some(300));
    assert_eq!(r.tier, TimeoutTier::Default);
    assert!(!r.provider_zero && !r.agent_zero);
}

/// `0` is skipped, not honoured: a zero-second reqwest timeout is an instant
/// deadline, so a typo must degrade to "the default applies", never to "every
/// call fails". The tier that carried it is still reported so the caller can
/// warn rather than stay silent.
#[test]
fn a_zero_is_skipped_and_reported_never_honoured() {
    let r = resolve_timeout(Some(0), None, Some(300));
    assert_eq!(r.effective, Some(300), "0 must never become the timeout");
    assert_eq!(r.tier, TimeoutTier::Default);
    assert!(r.provider_zero, "the skipped tier must be visible");
}

#[test]
fn a_skipped_provider_zero_falls_through_to_the_agent_tier() {
    let r = resolve_timeout(Some(0), Some(60), Some(300));
    assert_eq!(r.effective, Some(60));
    assert_eq!(r.tier, TimeoutTier::Agent);
    assert!(r.provider_zero);
}

#[test]
fn both_tiers_zero_reports_both() {
    let r = resolve_timeout(Some(0), Some(0), Some(300));
    assert_eq!(r.effective, Some(300));
    assert!(r.provider_zero && r.agent_zero);
    assert_eq!(
        r.zero_note(),
        Some("[providers.<name>] and [agent] both carried 0")
    );
}

/// Stream idle has no compiled floor on purpose: the default is chosen at stream
/// time from whether the target is local/CLI or remote (`helpers.rs` — 3600s
/// there, 20s for remote, 45s for the z.ai host). An unset flag must defer to
/// that runtime decision instead of overriding it with a constant guessed here.
#[test]
fn stream_idle_without_a_compiled_floor_defers_to_the_runtime_default() {
    let r = resolve_timeout(None, None, None);
    assert_eq!(r.effective, None, "must not invent a floor");
    assert_eq!(r.tier, TimeoutTier::Default);
    assert_eq!(r.duration(), None);
}

#[test]
fn duration_maps_the_effective_seconds() {
    let r = resolve_timeout(None, Some(45), None);
    assert_eq!(r.duration(), Some(std::time::Duration::from_secs(45)));
}

#[test]
fn the_compiled_default_is_never_reported_as_an_override() {
    // `duration()` answers "what ceiling is in force"; `override_duration()`
    // answers "did anybody configure it". The trait accessors must report the
    // second one, or an unconfigured provider claims an override.
    let unset = resolve_timeout(None, None, Some(300));
    assert_eq!(unset.duration(), Some(std::time::Duration::from_secs(300)));
    assert_eq!(unset.override_duration(), None);

    let zeroed = resolve_timeout(Some(0), Some(0), Some(300));
    assert_eq!(zeroed.override_duration(), None);

    for configured in [
        resolve_timeout(Some(120), None, Some(300)),
        resolve_timeout(None, Some(60), Some(300)),
    ] {
        assert_eq!(configured.override_duration(), configured.duration());
        assert_ne!(configured.tier, TimeoutTier::Default);
    }
}

#[test]
fn zero_note_names_only_the_section_that_actually_carried_it() {
    assert_eq!(
        resolve_timeout(Some(0), None, Some(300)).zero_note(),
        Some("[providers.<name>] carried 0")
    );
    assert_eq!(
        resolve_timeout(None, Some(0), Some(300)).zero_note(),
        Some("[agent] carried 0")
    );
    assert_eq!(
        resolve_timeout(Some(30), Some(30), Some(300)).zero_note(),
        None,
        "no skip, no warning"
    );
}

// ----------------------------------------------------------- the config tiers

/// The keys must parse under `[agent]` with exactly these spellings — a typo in
/// a serde name is how a flag becomes invisible again while looking configured.
#[test]
fn agent_config_parses_both_global_timeout_keys() {
    let agent: AgentConfig = toml::from_str(
        r#"
timeout_secs = 120
stream_idle_timeout_secs = 45
"#,
    )
    .expect("[agent] timeout_secs / stream_idle_timeout_secs must parse");
    assert_eq!(agent.timeout_secs, Some(120));
    assert_eq!(agent.stream_idle_timeout_secs, Some(45));
}

#[test]
fn agent_config_leaves_both_global_keys_unset_by_default() {
    // Unset is not 0 and not 300: it is "no opinion", so the per-provider tier
    // or the family default answers. Setting a value here would silently make
    // `[agent]` the winner over every provider section.
    let agent = AgentConfig::default();
    assert_eq!(agent.timeout_secs, None);
    assert_eq!(agent.stream_idle_timeout_secs, None);
}

// ------------------------------------------------------------- the wiring

/// The regression this guards: a call site reverting to reading one tier by
/// hand. `filter(|&s| s > 0)` was exactly that read, at one site, for one tier.
#[test]
fn factory_never_reads_a_timeout_tier_by_hand() {
    let src = factory_source();
    assert!(
        !src.contains("filter(|&s| s > 0)"),
        "a hand-rolled tier read is back in factory.rs — it bypasses the \
         [providers.<name>] -> [agent] -> default chain (#1688)"
    );
}

#[test]
fn factory_resolves_every_timeout_flag_through_the_chain() {
    let src = factory_source();
    for needle in [
        "config.timeout_secs",
        "agent.timeout_secs",
        "config.stream_idle_timeout_secs",
        "agent.stream_idle_timeout_secs",
        "zhipu_config.stream_idle_timeout_secs",
    ] {
        assert!(
            src.contains(needle),
            "factory.rs no longer mentions {needle} — one of the two tiers \
             stopped being consulted for a timeout flag (#1688)"
        );
    }
    // #1689: the two per-flag resolutions now sit behind one shared helper, and
    // that helper is called by all three families. Scanning for the helper keeps
    // the count meaningful: a family that stops calling it is the regression.
    assert!(
        src.matches("resolve_and_report(").count() >= 2,
        "expected resolve_and_report() to cover both flags, found {}",
        src.matches("resolve_and_report(").count()
    );
    assert!(
        src.matches("report_timeout_chain(").count() >= 3,
        "expected report_timeout_chain() in the compat, anthropic and gemini \
         families, found {}",
        src.matches("report_timeout_chain(").count()
    );
}
