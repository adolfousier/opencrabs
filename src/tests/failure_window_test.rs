//! The pure recent-failure window behind sticky fallback (#1667).
//!
//! Exercised at explicit clock positions so expiry is covered without a
//! sleeping test and without standing up a service or a provider.

use std::time::{Duration, Instant};

use crate::brain::agent::service::failure_window::{STICKY_FALLBACK_WINDOW, count, record};

const WINDOW: Duration = Duration::from_secs(600);

#[test]
fn an_empty_history_counts_zero() {
    assert_eq!(count(&[], Instant::now(), WINDOW), 0);
}

#[test]
fn every_failure_inside_the_window_counts() {
    let start = Instant::now();
    let mut failures = Vec::new();
    for minute in 0..4 {
        record(
            &mut failures,
            start + Duration::from_secs(60 * minute),
            WINDOW,
        );
    }
    assert_eq!(
        count(&failures, start + Duration::from_secs(60 * 4), WINDOW),
        4
    );
}

#[test]
fn a_failure_exactly_at_the_cutoff_still_counts() {
    // The boundary decides whether the gate is off-by-one. Inclusive:
    // a failure exactly `window` ago is the oldest one still inside.
    let start = Instant::now();
    let mut failures = Vec::new();
    record(&mut failures, start, WINDOW);
    assert_eq!(count(&failures, start + WINDOW, WINDOW), 1);
    assert_eq!(
        count(&failures, start + WINDOW + Duration::from_nanos(1), WINDOW),
        0
    );
}

#[test]
fn recording_prunes_what_has_aged_out() {
    // Without pruning inside record, a long session's Vec grows for the
    // lifetime of the process even though nothing old can ever count.
    let start = Instant::now();
    let mut failures = Vec::new();
    for minute in 0..5 {
        record(
            &mut failures,
            start + Duration::from_secs(60 * minute),
            WINDOW,
        );
    }
    assert_eq!(failures.len(), 5);

    let much_later = start + WINDOW + Duration::from_secs(60 * 10);
    assert_eq!(record(&mut failures, much_later, WINDOW), 1);
    assert_eq!(
        failures.len(),
        1,
        "aged-out entries must be dropped, not just ignored"
    );
}

#[test]
fn the_shipped_window_spans_several_turns_without_outliving_a_recovery() {
    // A window shorter than a few turns cannot accumulate; one measured in
    // hours keeps judging a provider that already recovered.
    assert!(STICKY_FALLBACK_WINDOW >= Duration::from_secs(5 * 60));
    assert!(STICKY_FALLBACK_WINDOW <= Duration::from_secs(60 * 60));
}
