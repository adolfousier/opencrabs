//! Regression coverage for the pure helpers in
//! `src/brain/agent/service/tool_loop.rs`.
//!
//! Linor's 2026-05-04 repo audit flagged tool_loop.rs as the highest-
//! priority bug-risk hotspot: 3,731 lines, 167 commits, zero dedicated
//! test files. This module pins the small handful of pure helpers
//! exposed for testing — `strip_ansi_output`, `extract_path_for_
//! recent_buffer`, and `is_user_correction` — so that future
//! refactors of the surrounding loop don't silently break the
//! corner-cases they handle.
//!
//! Pure-helper coverage is the entry point for the bigger refactor
//! described in the Linor report (extracting cohesive concerns out of
//! the giant `run_tool_loop_inner`); the loop logic itself is too
//! intertwined with provider streams + DB writes to unit-test in
//! isolation, but at minimum we want every PURE function the loop
//! calls to have characterization tests that prevent regressions.
//!
//! When the bigger refactor lands and more pure functions surface,
//! add their tests to this file rather than splintering into many
//! tiny ones — the project rule (memory: tests live under
//! `src/tests/`) keeps the test surface flat.

use crate::brain::agent::service::tool_loop::{
    DEFAULT_TIME_MARKER_INTERVAL_SECS, RepeatLoopAction, build_tool_result_content,
    check_intra_turn_time_marker, extract_path_for_recent_buffer, is_duplicate_iteration_text,
    is_implausible_token_report, is_user_correction, repeat_loop_action, strip_ansi_output,
};
use serde_json::json;
use std::path::PathBuf;

// ── strip_ansi_output ──────────────────────────────────────────────

#[test]
fn strip_ansi_output_removes_basic_color_codes() {
    // SGR 31 = red; SGR 0 = reset. Any reasonable ANSI stripper should
    // collapse this to the bare text.
    let raw = "\x1b[31merror\x1b[0m: tool failed";
    let cleaned = strip_ansi_output(raw);
    assert_eq!(cleaned, "error: tool failed");
}

#[test]
fn strip_ansi_output_handles_no_ansi() {
    let raw = "plain output, no escapes here";
    let cleaned = strip_ansi_output(raw);
    assert_eq!(cleaned, raw);
}

#[test]
fn strip_ansi_output_handles_empty_string() {
    assert_eq!(strip_ansi_output(""), "");
}

#[test]
fn strip_ansi_output_strips_cursor_movement() {
    // CSI 2J (clear screen), CSI H (cursor home) etc. should not leak
    // into tool-result text rendered to the model.
    let raw = "\x1b[2J\x1b[Hready";
    assert_eq!(strip_ansi_output(raw), "ready");
}

#[test]
fn strip_ansi_output_preserves_unicode() {
    // Non-ANSI multi-byte content must round-trip unchanged. Caught a
    // class of stripper bugs where ESC-detection code accidentally
    // consumed a UTF-8 continuation byte.
    let raw = "🦀 OpenCrabs";
    assert_eq!(strip_ansi_output(raw), "🦀 OpenCrabs");
}

#[test]
fn strip_ansi_output_strips_24bit_truecolor() {
    // SGR 38;2;R;G;B sets 24-bit foreground. Real-world tool output
    // (cargo, ripgrep, --color=always git diff) emits these.
    let raw = "\x1b[38;2;255;100;0morange\x1b[0m";
    assert_eq!(strip_ansi_output(raw), "orange");
}

// ── extract_path_for_recent_buffer ────────────────────────────────

fn cwd() -> PathBuf {
    std::env::temp_dir().join("opencrabs-tool-loop-test")
}

#[test]
fn extract_path_returns_none_for_unrelated_tools() {
    // Only tools that operate on a single file path are tracked.
    // bash / glob / web_search / generate_image have no path arg
    // worth surfacing.
    for tool in &["bash", "glob", "web_search", "generate_image", "task"] {
        let result = extract_path_for_recent_buffer(tool, &json!({}), &cwd());
        assert!(
            result.is_none(),
            "tool '{tool}' should never contribute to recent_paths"
        );
    }
}

#[test]
fn extract_path_returns_none_when_path_missing() {
    // read_file IS path-bearing, but if the agent emitted a malformed
    // call without the field, we must NOT inject a phantom row.
    let result = extract_path_for_recent_buffer("read_file", &json!({}), &cwd());
    assert!(result.is_none());
}

#[test]
fn extract_path_returns_none_for_empty_path() {
    // Whitespace-only paths are agent slop — same outcome as missing.
    for empty in &["", " ", "  \t\n  "] {
        let result = extract_path_for_recent_buffer("read_file", &json!({ "path": empty }), &cwd());
        assert!(
            result.is_none(),
            "empty path '{empty:?}' must not be recorded"
        );
    }
}

#[test]
fn extract_path_resolves_relative_against_cwd() {
    // The agent often returns relative paths. The recent-paths buffer
    // stores absolute paths so re-display next turn doesn't depend on
    // the working directory at lookup time.
    let result =
        extract_path_for_recent_buffer("read_file", &json!({ "path": "src/main.rs" }), &cwd());
    let path = result.expect("relative path must resolve");
    assert!(
        path.is_absolute(),
        "stored path should be absolute, got {path:?}"
    );
    assert!(
        path.ends_with("src/main.rs"),
        "tail must preserve the original path, got {path:?}"
    );
}

#[test]
#[cfg(unix)]
fn extract_path_passes_through_absolute_path() {
    // Already-absolute paths must round-trip unchanged.
    let result =
        extract_path_for_recent_buffer("read_file", &json!({ "path": "/etc/hosts" }), &cwd());
    assert_eq!(result, Some(PathBuf::from("/etc/hosts")));
}

#[test]
#[cfg(windows)]
fn extract_path_passes_through_absolute_path() {
    // On Windows, absolute paths require a drive letter.
    let result = extract_path_for_recent_buffer(
        "read_file",
        &json!({ "path": "C:\\Windows\\System32\\drivers\\etc\\hosts" }),
        &cwd(),
    );
    assert_eq!(
        result,
        Some(PathBuf::from("C:\\Windows\\System32\\drivers\\etc\\hosts"))
    );
}

#[test]
#[cfg(unix)]
fn extract_path_covers_all_documented_tools() {
    // Pin the documented set of path-bearing tools (read_file, edit_file,
    // write_file, ls, grep). If any are removed silently the recent-
    // paths buffer would stop tracking those operations and the next-
    // turn anchor list would go stale.
    for tool in &["read_file", "edit_file", "write_file", "ls", "grep"] {
        let result = extract_path_for_recent_buffer(tool, &json!({ "path": "/abs/file" }), &cwd());
        assert_eq!(
            result,
            Some(PathBuf::from("/abs/file")),
            "tool '{tool}' must contribute to recent_paths"
        );
    }
}

#[test]
#[cfg(windows)]
fn extract_path_covers_all_documented_tools() {
    // On Windows, absolute paths require a drive letter.
    for tool in &["read_file", "edit_file", "write_file", "ls", "grep"] {
        let result =
            extract_path_for_recent_buffer(tool, &json!({ "path": "C:\\abs\\file" }), &cwd());
        assert_eq!(
            result,
            Some(PathBuf::from("C:\\abs\\file")),
            "tool '{tool}' must contribute to recent_paths"
        );
    }
}

// ── is_user_correction ─────────────────────────────────────────────

#[test]
fn user_correction_detects_short_no_phrases() {
    // The most common correction signals — terse user pushback. These
    // should ALL trigger the retry-context injection.
    for msg in &[
        "no, that's wrong",
        "no.",
        "no!",
        "no that's not what I asked",
        "nope",
        "wrong",
        "that's not right",
        "that's wrong",
        "thats wrong",
        "not what i wanted",
        "try again",
        "redo",
        "revert",
        "undo",
        "you broke it",
        "broke it",
        "doesn't work",
        "doesnt work",
        "didn't work",
        "didnt work",
        "not working",
        "stop",
        "don't do that",
        "dont do that",
        "i said no",
        "i asked for X",
        "not correct",
        "fix it",
        "fix this",
    ] {
        assert!(
            is_user_correction(msg),
            "message {msg:?} should be detected as a correction"
        );
    }
}

#[test]
fn user_correction_ignores_long_messages() {
    // Long messages are usually new instructions, not corrections —
    // even when they contain a "no" or "stop" somewhere.
    let long = "I have a new task involving the API surface for our \
                ingestion pipeline. Please don't do the same approach as \
                last time — that hit a deadlock under concurrent load. \
                Instead, write a server in Rust that responds to GET \
                requests with JSON. Use Tokio + axum. Implement /health, \
                /metrics, /readyz, /livez, and /version endpoints. Don't \
                include any frontend assets, TLS termination, or auth \
                middleware. Stop after the server is running locally and \
                report back with the full route table, the listening \
                address, and the binary footprint. Make sure tests pass.";
    assert!(
        long.len() > 500,
        "test fixture must actually be long ({}) for the gate to be exercised",
        long.len()
    );
    assert!(
        !is_user_correction(long),
        "long instruction must NOT be classified as correction"
    );
}

#[test]
fn user_correction_ignores_extremely_short_messages() {
    // Anything under 2 chars can't carry an unambiguous correction
    // signal — must not trigger a retry on a single-keystroke message.
    for msg in &["", "?"] {
        assert!(
            !is_user_correction(msg),
            "extremely short message {msg:?} must not trigger correction path"
        );
    }
}

#[test]
fn user_correction_is_case_insensitive() {
    // Patterns are matched against lowercased input — uppercase user
    // shouting must still be detected.
    for msg in &["NO!", "WRONG", "Try Again", "FIX IT"] {
        assert!(
            is_user_correction(msg),
            "uppercase {msg:?} must be detected"
        );
    }
}

#[test]
fn user_correction_does_not_fire_on_neutral_prose() {
    // Common phrasing in normal user prompts should NOT trigger the
    // retry-context injection; otherwise routine messages get treated
    // as failures.
    for msg in &[
        "what is the capital of France?",
        "show me the diff",
        "let's add a new feature",
        "explain how this works",
        "thanks",
        "ok",
        "got it",
    ] {
        assert!(
            !is_user_correction(msg),
            "neutral message {msg:?} must NOT be classified as correction"
        );
    }
}

#[test]
fn user_correction_only_scans_first_300_chars() {
    // The is_user_correction function explicitly takes only the first
    // 300 chars before lowercasing — pin that so a "stop" word buried
    // deep in a long message doesn't false-trigger.
    let prefix = "x".repeat(300);
    let buried = format!("{prefix} stop please");
    // Must not exceed the 500-char total cap, otherwise the length gate
    // discards before we hit the prefix scan.
    assert!(buried.len() <= 500);
    assert!(
        !is_user_correction(&buried),
        "trigger word past 300-char window must not match"
    );
}

// ── build_tool_result_content ──────────────────────────────────────

#[test]
fn build_tool_result_success_returns_output() {
    let content = build_tool_result_content(true, None, "command output");
    assert_eq!(content, "command output");
}

#[test]
fn build_tool_result_failure_includes_error_and_output() {
    let content = build_tool_result_content(
        false,
        Some("Command exited with code 1".to_string()),
        "stderr: file not found",
    );
    assert!(content.contains("Command exited with code 1"));
    assert!(content.contains("-- output captured before error --"));
    assert!(content.contains("stderr: file not found"));
}

#[test]
fn build_tool_result_failure_strips_ansi_from_error() {
    let content = build_tool_result_content(
        false,
        Some("\x1b[31merror\x1b[0m: build failed".to_string()),
        "",
    );
    assert!(!content.contains("\x1b["));
    assert!(content.contains("error: build failed"));
}

#[test]
fn build_tool_result_failure_strips_ansi_from_output() {
    let content = build_tool_result_content(
        false,
        Some("error".to_string()),
        "\x1b[32msuccess\x1b[0m: compiled",
    );
    assert!(!content.contains("\x1b["));
    assert!(content.contains("success: compiled"));
}

#[test]
fn build_tool_result_failure_caps_output_at_8000_chars() {
    let large_output = "x".repeat(10000);
    let content = build_tool_result_content(false, Some("error".to_string()), &large_output);
    // The captured portion should be truncated
    assert!(content.contains("(output truncated)"));
    // But should still contain some of the output
    assert!(content.contains("xxx"));
}

#[test]
fn build_tool_result_failure_no_truncation_for_small_output() {
    let content = build_tool_result_content(false, Some("error".to_string()), "small output");
    assert!(!content.contains("(output truncated)"));
    assert!(content.contains("small output"));
}

#[test]
fn build_tool_result_failure_default_error_message() {
    let content = build_tool_result_content(false, None, "some output");
    assert!(content.contains("Tool execution failed"));
}

// Sanity guard against a provider/proxy that over-reports input_tokens.
// The zhipu "coding" endpoint added a flat ~20k to every call's reported
// input regardless of content size (a fixed additive overhead, not a
// tokenizer ratio): an 8.4k-token request came back reported as 28.8k, which
// the ctx counter trusted and inflated to 28k for a one-word "yes" reply.
// The guard keeps the local estimate when the provider's number is >2× the
// real content size (system + messages + tool schemas).

#[test]
fn over_reporting_provider_is_rejected() {
    // The exact observed shape: ~8.4k real content, provider claims 28.8k.
    assert!(
        is_implausible_token_report(8439, 0, 28775),
        "a 3.4× over-report (the zhipu '+20k' inflation) must be rejected"
    );
    // Even when the inflation is a smaller ratio on a bigger request, >2× still trips.
    assert!(is_implausible_token_report(10000, 0, 21000));
}

#[test]
fn legitimate_usage_is_trusted() {
    // Real tokenizer differences stay within ~2× — must be accepted.
    assert!(
        !is_implausible_token_report(8439, 0, 11000),
        "a 1.3× tokenizer difference is normal and must be trusted"
    );
    // Tool schemas legitimately inflate the provider's count beyond the
    // system+messages estimate — that overhead is expected, not over-reporting.
    assert!(
        !is_implausible_token_report(8000, 12000, 26000),
        "system+messages(8k) + tool schemas(12k) → ~20k expected; 26k is within 2× and must be trusted"
    );
    // Exactly 2× is the boundary — trusted (must EXCEED 2× to reject).
    assert!(!is_implausible_token_report(5000, 0, 10000));
}

#[test]
fn tiny_requests_are_never_rejected() {
    // Below the 1000-token floor the ratio is noise — never reject (a fresh
    // session's first tiny turn shouldn't trip the guard).
    assert!(!is_implausible_token_report(200, 0, 5000));
}

// ── repeat_loop_action: identical-call loop nudge/break (#507) ───────

fn seq(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn repeat_below_threshold_continues() {
    // Only 4 of the current signature in the window (need 5 to nudge).
    let recent = seq(&[
        "grep:aa", "read:bb", "grep:aa", "read:bb", "grep:aa", "grep:aa",
    ]);
    assert_eq!(
        repeat_loop_action(&recent, "grep:aa", 8, 5, 7, false),
        RepeatLoopAction::Continue,
    );
}

#[test]
fn dominant_repeat_nudges_even_when_interleaved() {
    // 5 identical greps interleaved with other calls in the last 8 → nudge.
    // The strictly-consecutive check would miss this entirely.
    let recent = seq(&[
        "grep:aa", "read:bb", "grep:aa", "ls:cc", "grep:aa", "read:bb", "grep:aa", "grep:aa",
    ]);
    assert_eq!(
        repeat_loop_action(&recent, "grep:aa", 8, 5, 7, false),
        RepeatLoopAction::Nudge,
    );
}

#[test]
fn nudge_fires_only_once_then_breaks_if_ignored() {
    // Already nudged and still dominating at/above break threshold → break.
    let recent = seq(&[
        "grep:aa", "grep:aa", "grep:aa", "grep:aa", "grep:aa", "grep:aa", "grep:aa",
    ]);
    assert_eq!(
        repeat_loop_action(&recent, "grep:aa", 8, 5, 7, true),
        RepeatLoopAction::Break,
    );
    // Already nudged but repeats dropped below break threshold → let it run.
    let recovered = seq(&[
        "grep:aa", "read:bb", "ls:cc", "edit:dd", "grep:aa", "read:bb",
    ]);
    assert_eq!(
        repeat_loop_action(&recovered, "grep:aa", 8, 5, 7, true),
        RepeatLoopAction::Continue,
    );
}

#[test]
fn window_bounds_the_count() {
    // Old identical calls outside the window must not count. The window is 3,
    // so only the last 3 signatures are considered — one match, no nudge.
    let recent = seq(&[
        "grep:aa", "grep:aa", "grep:aa", "read:bb", "ls:cc", "grep:aa",
    ]);
    assert_eq!(
        repeat_loop_action(&recent, "grep:aa", 3, 2, 3, false),
        RepeatLoopAction::Continue,
    );
}

// ── is_duplicate_iteration_text (#1070) ────────────────────────────

#[test]
fn duplicate_text_catches_verbatim_reemission() {
    // The bug: the model answers, calls one more tool, then restates the
    // SAME answer on the next iteration. Both copies used to land in
    // accumulated_text and ship to the user as one doubled message.
    let answer = "The config key is never merged, so the value is dropped.";
    assert!(is_duplicate_iteration_text(answer, answer));
}

#[test]
fn duplicate_text_allows_the_first_emission() {
    // Nothing accumulated yet — the first answer must always get through.
    assert!(!is_duplicate_iteration_text("", "here is the answer"));
}

#[test]
fn duplicate_text_allows_a_genuine_continuation() {
    // Real follow-up content is not a duplicate and must not be swallowed.
    let accumulated = "Checked the merge arms.";
    assert!(!is_duplicate_iteration_text(
        accumulated,
        "Now filing the issue."
    ));
}

#[test]
fn duplicate_text_ignores_whitespace_only_candidate() {
    // The trap this helper exists to avoid: `contains("")` is trivially
    // true, so a naive guard would classify every blank segment as a
    // duplicate and swallow the separator bookkeeping the call sites rely
    // on. Blank candidates are never duplicates.
    assert!(!is_duplicate_iteration_text("some earlier text", "   "));
    assert!(!is_duplicate_iteration_text("some earlier text", ""));
    assert!(!is_duplicate_iteration_text("", ""));
}

#[test]
fn duplicate_text_matches_despite_surrounding_whitespace() {
    // Providers pad segments with newlines; the padding must not defeat
    // the check or the duplicate slips straight through.
    let accumulated = "Filed as issue #1070.";
    assert!(is_duplicate_iteration_text(
        accumulated,
        "\n\n  Filed as issue #1070.  \n"
    ));
}

#[test]
fn duplicate_text_catches_a_segment_added_earlier_in_the_same_loop() {
    // The CLI path rebuilds accumulated_text by draining ordered segments
    // in a loop, so the guard has to see MID-loop state, not a snapshot
    // taken before it started. Segment two repeats segment one.
    let mut accumulated = String::new();
    for seg in ["Done.", "Done."] {
        if is_duplicate_iteration_text(&accumulated, seg) {
            continue;
        }
        if !accumulated.is_empty() {
            accumulated.push_str("\n\n");
        }
        accumulated.push_str(seg);
    }
    assert_eq!(
        accumulated, "Done.",
        "second identical segment must be skipped"
    );
}

#[test]
fn duplicate_text_does_not_catch_a_reworded_restatement() {
    // Documented limit, asserted so nobody assumes more than it does.
    // Containment catches verbatim re-emission only. Catching a reworded
    // restatement needs similarity scoring, and a false positive there
    // would silently delete real content the user is waiting on — so this
    // deliberately stays a substring check.
    let accumulated = "The key is dropped because there is no merge arm.";
    let reworded = "There is no merge arm, so the key gets dropped.";
    assert!(!is_duplicate_iteration_text(accumulated, reworded));
}

#[test]
fn duplicate_text_catches_a_substring_of_longer_accumulated_text() {
    // A short completion note the turn already emitted verbatim inside a
    // longer block is still a duplicate — this is the "pure completion
    // acknowledgement" shape the phantom exemption lets through.
    let accumulated = "Ran the tests.\n\nAll 112 passed, 0 failed.";
    assert!(is_duplicate_iteration_text(
        accumulated,
        "All 112 passed, 0 failed."
    ));
}

// ── check_intra_turn_time_marker ────────────────────────────────────

#[test]
fn intra_turn_time_marker_defaults_and_suppression() {
    assert_eq!(DEFAULT_TIME_MARKER_INTERVAL_SECS, 900);

    let start = chrono::Utc::now();
    // 0 interval disables the check entirely
    assert!(
        check_intra_turn_time_marker(start, start + chrono::Duration::seconds(1800), 0, None)
            .is_none()
    );

    // Less than interval elapsed -> None
    let under = start + chrono::Duration::seconds(899);
    assert!(check_intra_turn_time_marker(start, under, 900, None).is_none());
}

#[test]
fn intra_turn_time_marker_fires_at_or_above_interval_with_utc_fallback() {
    let start = chrono::Utc::now();
    let exact = start + chrono::Duration::seconds(900);
    let res = check_intra_turn_time_marker(start, exact, 900, None);
    assert!(res.is_some());
    let (notice, updated_time) = res.unwrap();
    assert_eq!(updated_time, exact);
    assert!(notice.starts_with("[System: Current time: "));
    assert!(notice.contains("UTC"));
    assert!(!notice.contains("(user:"));
    assert!(notice.ends_with(']'));
}

#[test]
fn intra_turn_time_marker_resolves_user_timezone_from_brain() {
    use std::fs;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let user_md = dir.path().join("USER.md");
    fs::write(&user_md, "Timezone: UTC+3 (MSK)\n").unwrap();

    let start = chrono::Utc::now();
    let elapsed = start + chrono::Duration::seconds(1200);
    let res = check_intra_turn_time_marker(start, elapsed, 900, Some(dir.path()));
    assert!(res.is_some());
    let (notice, _) = res.unwrap();
    assert!(notice.starts_with("[System: Current time: "));
    assert!(notice.contains("UTC"));
    assert!(notice.contains("MSK"));
    assert!(notice.ends_with(']'));
}
