//! Disappearing-message TTL resolution (#1487).

use crate::channels::whatsapp::ephemeral::{
    TTL_7_DAYS, TTL_24_HOURS, TTL_90_DAYS, TTL_MAX, describe, resolve,
};

#[test]
fn no_request_and_no_default_means_no_expiry() {
    assert_eq!(resolve(None, None), None);
}

#[test]
fn the_channel_default_applies_when_the_caller_says_nothing() {
    assert_eq!(resolve(None, Some(TTL_24_HOURS)), Some(TTL_24_HOURS));
}

#[test]
fn an_explicit_request_beats_the_channel_default() {
    assert_eq!(resolve(Some(3_600), Some(TTL_7_DAYS)), Some(3_600));
}

#[test]
fn an_explicit_zero_turns_expiry_off_despite_the_default() {
    // This is why the resolution is not a plain `or`: "send this one
    // permanently" has to be expressible on a channel that expires by default.
    assert_eq!(resolve(Some(0), Some(TTL_7_DAYS)), None);
}

#[test]
fn a_zero_default_is_treated_as_unset() {
    assert_eq!(resolve(None, Some(0)), None);
}

#[test]
fn an_over_long_request_is_clamped_rather_than_refused() {
    // Refusing the send would be worse than honouring the maximum: the caller
    // asked for "as long as possible".
    assert_eq!(resolve(Some(u64::MAX), None), Some(TTL_MAX));
    assert_eq!(
        resolve(Some(u64::from(TTL_90_DAYS) + 1), None),
        Some(TTL_MAX)
    );
}

#[test]
fn an_over_long_default_is_clamped_too() {
    assert_eq!(resolve(None, Some(u32::MAX)), Some(TTL_MAX));
}

#[test]
fn the_maximum_itself_passes_through() {
    assert_eq!(
        resolve(Some(u64::from(TTL_90_DAYS)), None),
        Some(TTL_90_DAYS)
    );
}

#[test]
fn the_whatsapp_presets_read_back_by_name() {
    assert_eq!(describe(TTL_24_HOURS), "24 hours");
    assert_eq!(describe(TTL_7_DAYS), "7 days");
    assert_eq!(describe(TTL_90_DAYS), "90 days");
}

#[test]
fn other_durations_read_back_in_the_largest_whole_unit() {
    assert_eq!(describe(3_600), "1 hours");
    assert_eq!(describe(172_800), "2 days");
    assert_eq!(describe(300), "5 minutes");
    assert_eq!(describe(45), "45 seconds");
}
