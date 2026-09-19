//! Regression guards for #1635.
//!
//! Two defects shipped together and reinforced each other:
//!
//! 1. Every OpenAI-compatible provider built its HTTP client with a 60s total
//!    request timeout. reqwest's `.timeout()` covers the response body read, so
//!    it is a hard wall-clock ceiling on an SSE stream no matter how many chunks
//!    are still arriving. Anthropic and Gemini already used 300s, which is why
//!    only the compat providers (every custom endpoint, i.e. every cheap
//!    thinking-heavy fallback model) died at exactly 60.0s mid-stream.
//! 2. The stream-retry log lines hardcoded `3` while `MAX_STREAM_RETRIES` is 5,
//!    emitting `Stream retry 5/3 failed` and `All 3 stream retries failed`. That
//!    made the retry budget look smaller than it is and sent the first reading
//!    of the incident down the wrong path entirely.

use std::path::Path;

use crate::brain::provider::anthropic::DEFAULT_TIMEOUT as ANTHROPIC_TOTAL_TIMEOUT;
use crate::brain::provider::custom_openai_compatible::DEFAULT_TIMEOUT as COMPAT_TOTAL_TIMEOUT;
use crate::brain::provider::gemini::DEFAULT_TIMEOUT as GEMINI_TOTAL_TIMEOUT;

/// Inter-chunk idle timeout applied to remote HTTP streams
/// (`brain/agent/service/helpers.rs`). This is the intended fast detector for a
/// genuinely dead stream; the total timeout must never creep down towards it.
const STREAM_IDLE_TIMEOUT_SECS: u64 = 20;

#[test]
fn compat_total_timeout_is_not_below_the_native_stream_providers() {
    assert!(
        COMPAT_TOTAL_TIMEOUT >= ANTHROPIC_TOTAL_TIMEOUT,
        "OpenAI-compatible total request timeout ({:?}) is below the Anthropic one ({:?}). \
         reqwest's client timeout covers the streamed body, so a lower value guillotines \
         healthy SSE streams on every custom provider while Anthropic keeps working (#1635).",
        COMPAT_TOTAL_TIMEOUT,
        ANTHROPIC_TOTAL_TIMEOUT,
    );
    assert!(
        COMPAT_TOTAL_TIMEOUT >= GEMINI_TOTAL_TIMEOUT,
        "OpenAI-compatible total request timeout ({:?}) is below the Gemini one ({:?}) (#1635).",
        COMPAT_TOTAL_TIMEOUT,
        GEMINI_TOTAL_TIMEOUT,
    );
}

#[test]
fn compat_total_timeout_stays_far_above_the_idle_detector() {
    assert!(
        COMPAT_TOTAL_TIMEOUT.as_secs() >= STREAM_IDLE_TIMEOUT_SECS * 5,
        "OpenAI-compatible total request timeout is {:?}, too close to the {}s inter-chunk \
         idle timeout. The total clock is meant to catch pathological hangs only; when it \
         approaches the idle window it starts killing streams that are still delivering (#1635).",
        COMPAT_TOTAL_TIMEOUT,
        STREAM_IDLE_TIMEOUT_SECS,
    );
}

#[test]
fn stream_retry_log_lines_never_hardcode_the_retry_budget() {
    let tool_loop =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/brain/agent/service/tool_loop.rs");
    let content = std::fs::read_to_string(&tool_loop)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", tool_loop.display()));

    // Anchors: if the retry log lines are ever reshaped, these fail loudly
    // instead of letting the scan below pass vacuously.
    for anchor in [
        "\"Stream retry {}/{} failed: {}\"",
        "\"All {} stream retries failed",
        "\"5xx retry {}/{} failed: {}\"",
        "\"All {} 5xx retries failed",
    ] {
        assert!(
            content.contains(anchor),
            "expected retry log format {anchor} in tool_loop.rs; the scan below cannot \
             guard what it can no longer find (#1635)"
        );
    }

    let mut violations = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("retry") && !lower.contains("retries") {
            continue;
        }
        // `retry {}/3 failed` — the attempt counter with a literal budget.
        if digit_follows(line, "retry {}/") {
            violations.push((idx + 1, line.trim().to_string()));
            continue;
        }
        // `All 3 stream retries failed` — the exhausted-budget line.
        if digit_follows(line, "All ") {
            violations.push((idx + 1, line.trim().to_string()));
        }
    }

    assert!(
        violations.is_empty(),
        "stream retry log lines must interpolate MAX_STREAM_RETRIES, not a literal count \
         (#1635). Offending lines in src/brain/agent/service/tool_loop.rs:\n{}",
        violations
            .iter()
            .map(|(line_no, text)| format!("  {line_no}: {text}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// True when `marker` occurs in `line` immediately followed by an ASCII digit.
fn digit_follows(line: &str, marker: &str) -> bool {
    line.match_indices(marker).any(|(at, _)| {
        line[at + marker.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    })
}
