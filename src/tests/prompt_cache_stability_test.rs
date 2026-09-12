//! Prompt-cache stability of the system brain (#657, #658, #681, #693, #153).
//!
//! The cached prefix must stay byte-stable across turns so the provider prompt
//! cache hits. #657 first achieved this by making Runtime Info date-only, but
//! #658 moved the whole Runtime Info block into the UNCACHED suffix (providers
//! stamp cache_control on the stable prefix only). However, #693 disabled the
//! split because the array-shaped 2-part system broke tool calling, sending the
//! entire system prompt as a single string.
//!
//! Under #153, Runtime Info carries `Current date: YYYY-MM-DD (UTC)` (date-only),
//! while live wall-clock awareness is injected into message-tail markers. This keeps
//! the system prompt byte-stable for 24h while preserving temporal grounding.
//!
//! These tests lock the real invariant: both CACHED PREFIX and suffix are byte-stable
//! across builds within the same UTC day, and the split boundary keeps per-session
//! lines in the suffix while per-instance constants (Known paths, compiled
//! features) stay cached.

use crate::brain::prompt_builder::{BrainLoader, RuntimeInfo, split_runtime_suffix};
use tempfile::TempDir;

fn loader() -> (TempDir, BrainLoader) {
    let dir = TempDir::new().expect("tempdir");
    let loader = BrainLoader::new(dir.path().to_path_buf());
    (dir, loader)
}

fn runtime_info() -> RuntimeInfo {
    RuntimeInfo {
        model: Some("test-model".to_string()),
        provider: Some("test-provider".to_string()),
        working_directory: Some("~/srv/rs/opencrabs".to_string()),
    }
}

/// A rough `HH:MM:SS` detector — three colon-separated 2-digit runs.
fn contains_seconds_time(s: &str) -> bool {
    s.split_whitespace().any(|tok| {
        let parts: Vec<&str> = tok.split(':').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit()))
    })
}

#[test]
fn runtime_info_carries_date_only() {
    // #153: system prompt carries date-only. The block must render a YYYY-MM-DD
    // line without volatile time-of-day (seconds absent).
    for brain in [
        loader().1.build_system_brain(Some(&runtime_info())),
        loader().1.build_core_brain(Some(&runtime_info())),
    ] {
        assert!(
            brain.contains("Current date:"),
            "expected date-only line, got:\n{}",
            brain
                .lines()
                .filter(|l| l.contains("date") || l.contains("Runtime"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(
            !brain.contains("Current date & time:"),
            "system prompt should not contain volatile time-of-day"
        );
        assert!(
            !contains_seconds_time(&brain),
            "system prompt must not include HH:MM:SS"
        );
    }
}

#[test]
fn cached_prefix_and_suffix_are_byte_stable_with_date_only() {
    // Under date-only timestamps, both prefix and suffix are byte-identical
    // across builds within the same day.
    let (_dir, loader) = loader();
    let a = loader.build_system_brain(Some(&runtime_info()));
    let b = loader.build_system_brain(Some(&runtime_info()));
    let (prefix_a, suffix_a) = split_runtime_suffix(&a);
    let (prefix_b, suffix_b) = split_runtime_suffix(&b);
    assert_eq!(
        prefix_a, prefix_b,
        "the cached prefix must be byte-stable across builds (#658)"
    );
    assert_eq!(
        suffix_a, suffix_b,
        "the suffix must be byte-stable across builds on the same date (#153)"
    );
    // Neither prefix nor suffix should carry seconds.
    assert!(
        !contains_seconds_time(&prefix_a),
        "seconds timestamp leaked into CACHED prefix"
    );
    assert!(
        !contains_seconds_time(suffix_a.as_deref().unwrap_or("")),
        "seconds timestamp leaked into suffix"
    );
}

#[test]
fn split_boundary_keeps_session_lines_in_suffix_and_constants_cached() {
    // #681 GAP 3: lock the split boundary. Per-SESSION lines (model, provider,
    // working directory, date) ride in the uncached suffix; per-INSTANCE
    // constants (Known paths, compiled features) stay in the cached prefix.
    let (_dir, loader) = loader();
    let brain = loader.build_system_brain(Some(&runtime_info()));
    let (prefix, suffix) = split_runtime_suffix(&brain);
    let suffix = suffix.expect("runtime block present");

    for volatile in [
        "Model: test-model",
        "Provider: test-provider",
        "Working directory:",
        "Current date:",
    ] {
        assert!(
            suffix.contains(volatile),
            "per-session line must be in the uncached suffix: {volatile:?}"
        );
        assert!(
            !prefix.contains(volatile),
            "per-session line must NOT be in the cached prefix: {volatile:?}"
        );
    }
    // Per-instance constants stay cached.
    assert!(
        prefix.contains("Known paths") && prefix.contains("Built-in features"),
        "Known paths + compiled features must stay in the cached prefix"
    );
}
