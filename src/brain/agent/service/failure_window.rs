//! How recently the primary provider has been failing (#1667).
//!
//! Sticky fallback used to count CONSECUTIVE rescues: any primary first-try
//! success wiped the count. That made the threshold unreachable for the exact
//! pattern the feature exists to absorb, an intermittently failing primary.
//! A `fail, ok, fail, ok` provider oscillated 1, 0, 1, 0 forever and never
//! stuck, while a provider failing four times straight was already surviving
//! on the fallback anyway.
//!
//! The counter now asks "how many rescues in the recent past" instead of
//! "how many in a row". A success no longer erases history; time does. That
//! preserves the concern the old reset defended, that failures from an
//! unrelated outage long ago should not stick the fallback today, without
//! handing a flapping provider a free wipe on every good turn.
//!
//! Pure so the expiry is testable at arbitrary clock positions, with no
//! service, provider or sleeping test.

use std::time::{Duration, Instant};

/// How far back a rescue still counts toward sticking the fallback.
///
/// Long enough to span several turns of a genuinely flaky provider, short
/// enough that a provider which recovers is not still being judged for it an
/// hour later.
pub(crate) const STICKY_FALLBACK_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Failures at or after this instant are still inside the window.
///
/// `Instant::checked_sub` returns `None` when the window reaches back past the
/// clock's origin, which happens in the first minutes of a process. Nothing
/// can be older than that, so every recorded failure counts.
fn cutoff(now: Instant, window: Duration) -> Option<Instant> {
    now.checked_sub(window)
}

/// Drop failures that have aged out, then record one at `now`.
/// Returns how many failures sit inside the window, including the new one.
pub(crate) fn record(failures: &mut Vec<Instant>, now: Instant, window: Duration) -> u32 {
    if let Some(cutoff) = cutoff(now, window) {
        failures.retain(|&at| at >= cutoff);
    }
    failures.push(now);
    failures.len() as u32
}

/// Count the failures inside the window without recording one or mutating.
pub(crate) fn count(failures: &[Instant], now: Instant, window: Duration) -> u32 {
    match cutoff(now, window) {
        Some(cutoff) => failures.iter().filter(|&&at| at >= cutoff).count() as u32,
        None => failures.len() as u32,
    }
}
