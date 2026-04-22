//! Unit tests for Claude CLI provider model normalization.
//!
//! Covers the date-suffix stripper and the alias → normalized-id map so
//! renames/release bumps are caught by CI instead of visually in the TUI.

use crate::brain::provider::claude_cli::{ClaudeCliProvider, strip_claude_date_suffix};

#[test]
fn strip_claude_date_suffix_strips_eight_digit_date() {
    assert_eq!(
        strip_claude_date_suffix("claude-opus-4-7-20260115"),
        "claude-opus-4-7"
    );
    assert_eq!(strip_claude_date_suffix("opus-4-7-20260115"), "opus-4-7");
}

#[test]
fn strip_claude_date_suffix_leaves_non_date_alone() {
    assert_eq!(strip_claude_date_suffix("opus-4-7"), "opus-4-7");
    assert_eq!(strip_claude_date_suffix("opus"), "opus");
    assert_eq!(
        strip_claude_date_suffix("sonnet-4-6-beta"),
        "sonnet-4-6-beta"
    );
}

#[test]
fn normalize_model_maps_bare_aliases_to_current_versions() {
    assert_eq!(ClaudeCliProvider::normalize_model("opus"), "opus-4-7");
    assert_eq!(ClaudeCliProvider::normalize_model("sonnet"), "sonnet-4-6");
    assert_eq!(ClaudeCliProvider::normalize_model("haiku"), "haiku-4-5");
}

#[test]
fn normalize_model_strips_claude_prefix_and_date() {
    assert_eq!(
        ClaudeCliProvider::normalize_model("claude-opus-4-7-20260115"),
        "opus-4-7"
    );
    assert_eq!(
        ClaudeCliProvider::normalize_model("claude-sonnet-4-6-20251112"),
        "sonnet-4-6"
    );
}

#[test]
fn normalize_model_passes_through_unknown() {
    assert_eq!(
        ClaudeCliProvider::normalize_model("opus-4-8"),
        "opus-4-8",
        "a future release shorthand should flow through unchanged"
    );
    assert_eq!(
        ClaudeCliProvider::normalize_model("custom-model-id"),
        "custom-model-id"
    );
}
