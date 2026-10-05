//! Regression tests for #1919: cron delivery truncation must never slice a
//! multibyte char in half. Case 1 mirrors the production receipt —
//! `end byte index 4000 is not a char boundary; it is inside '✅' (bytes
//! 3999..4002)` — the 2026-10-04 panic under the `HeyIolo Morning Recap` job.

use crate::cron::scheduler::truncate_for_delivery;

const MARKER: &str = "...\n\n(truncated — full output in session)";

#[test]
fn emoji_straddling_the_limit_truncates_without_panic() {
    let mut s = "a".repeat(3999);
    s.push('✅'); // bytes 3999..4002: byte index 4000 falls mid-char
    s.push_str(&" tail".repeat(100));
    assert!(s.len() > 4000);
    let out = truncate_for_delivery(&s);
    let kept = out.strip_suffix(MARKER).expect("truncation marker present");
    assert_eq!(
        kept.len(),
        3999,
        "cut backs off to the char boundary, never half an emoji"
    );
    assert_eq!(kept, "a".repeat(3999));
}

#[test]
fn ascii_over_the_limit_cuts_at_the_limit() {
    let s = "b".repeat(4500);
    let out = truncate_for_delivery(&s);
    let kept = out.strip_suffix(MARKER).expect("truncation marker present");
    assert_eq!(kept.len(), 4000);
}

#[test]
fn content_at_or_under_the_limit_is_untouched() {
    let short = "morning recap ✅";
    assert_eq!(truncate_for_delivery(short), short);
    let exact = "c".repeat(4000);
    assert_eq!(truncate_for_delivery(&exact), exact);
}
