//! Context-aware request budgets.
//!
//! A model context window is shared by input, hidden reasoning, and output.
//! Reserving the global 65,536-token output cap unchanged on a 200K route
//! leaves barely 134K for the conversation and makes every tool round collide
//! with compaction. Keep this arithmetic pure and provider-agnostic: the caller
//! supplies the active context window, and this module leaves at least 80% for
//! input while respecting the configured output ceiling.

/// Maximum share of a context window one request may reserve for output.
const OUTPUT_WINDOW_PERCENT: u32 = 20;

/// Cap `configured_max` so output cannot consume more than 20% of `context_window`.
///
/// A zero window means "unknown"; preserve the configured value rather than
/// inventing a capacity. Non-zero windows use integer arithmetic deliberately:
/// rounding down leaves the extra fraction to input headroom.
pub(crate) fn bounded_output_tokens(configured_max: u32, context_window: u32) -> u32 {
    if context_window == 0 {
        return configured_max;
    }
    configured_max.min(context_window.saturating_mul(OUTPUT_WINDOW_PERCENT) / 100)
}
