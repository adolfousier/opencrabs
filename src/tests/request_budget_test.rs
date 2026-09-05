//! Request-budget regressions for bounded context windows.
//!
//! A model's context window is a shared input + output budget. Sending the
//! global 65,536-token output cap unchanged on a 200K route leaves only ~134K
//! for input. That is exactly where the observed Astra session began compacting
//! on every tool round and occasionally lost its stream. Keep one policy here:
//! reserve no more than 20% of the active window for output, while preserving
//! the configured cap on large-window providers.

use crate::brain::agent::service::request_budget::bounded_output_tokens;

#[test]
fn two_hundred_k_window_caps_output_at_forty_k() {
    assert_eq!(bounded_output_tokens(65_536, 200_000), 40_000);
}

#[test]
fn one_million_window_keeps_configured_output_cap() {
    assert_eq!(bounded_output_tokens(65_536, 1_000_000), 65_536);
}

#[test]
fn tiny_window_still_leaves_input_headroom() {
    assert_eq!(bounded_output_tokens(65_536, 8_192), 1_638);
}

#[test]
fn zero_window_falls_back_to_configured_cap() {
    assert_eq!(bounded_output_tokens(65_536, 0), 65_536);
}
