//! Pure decision logic for bounded WhatsApp history sync (#1525): request
//! planning, the 90-day capture window, per-chat opt-in matching, the
//! `from_me` anchor flag, and search-knob clamping. The caps here are the
//! decision doc's "constant with a test, not a vibe" clause
//! (~/.opencrabs/research/whatsapp-history-sync-decisions-1525.md); the DB
//! and transport legs they feed are thin enough that compilation covers them.

use crate::channels::whatsapp::history::{
    self, HISTORY_IMPORT_MAX_MESSAGES, from_me_of, in_window, opted_in, plan_import,
};
use chrono::{DateTime, TimeZone, Utc};

const NOW: i64 = 1_800_000_000;

fn ts(sec: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(sec, 0).unwrap()
}

#[test]
fn empty_store_anchors_a_full_page_at_now() {
    let plan = plan_import(None, ts(NOW)).expect("fresh store must ask for the newest head");
    assert_eq!(plan.oldest_msg_id, "");
    assert!(!plan.oldest_from_me);
    assert_eq!(plan.oldest_ts_ms, NOW * 1000);
    assert_eq!(plan.count, HISTORY_IMPORT_MAX_MESSAGES);
}

#[test]
fn in_window_anchor_continues_below_it() {
    let anchor = ts(NOW) - chrono::Duration::days(30);
    let plan = plan_import(Some(("MSG1", true, anchor)), ts(NOW)).unwrap();
    assert_eq!(plan.oldest_msg_id, "MSG1");
    assert!(plan.oldest_from_me);
    assert_eq!(plan.oldest_ts_ms, anchor.timestamp_millis());
    assert_eq!(plan.count, HISTORY_IMPORT_MAX_MESSAGES);
}

#[test]
fn anchor_at_or_beyond_the_90_day_cutoff_stops_the_walk() {
    let edge = ts(NOW) - chrono::Duration::days(history::HISTORY_IMPORT_MAX_AGE_DAYS);
    // At the cutoff the oldest stored row already reaches the bound; going
    // older only imports rows the capture window would drop anyway.
    assert!(plan_import(Some(("MSG1", false, edge)), ts(NOW)).is_none());
    assert!(
        plan_import(
            Some(("MSG1", false, edge - chrono::Duration::days(1))),
            ts(NOW)
        )
        .is_none()
    );
    // A day inside the cutoff still has room to pull.
    assert!(
        plan_import(
            Some(("MSG1", false, edge + chrono::Duration::days(1))),
            ts(NOW)
        )
        .is_some()
    );
}

#[test]
fn capture_window_filter_matches_the_bound() {
    let now = ts(NOW);
    assert!(in_window(now - chrono::Duration::days(89), now));
    assert!(!in_window(now - chrono::Duration::days(91), now));
}

#[test]
fn opt_in_is_exact_key_match() {
    let chats = vec!["123@s.whatsapp.net".to_string()];
    assert!(opted_in(&chats, "123@s.whatsapp.net"));
    // No prefix matching — #1544's trap is loose keys merging partitions.
    assert!(!opted_in(&chats, "1234@s.whatsapp.net"));
    assert!(!opted_in(&chats, "123"));
    assert!(!opted_in(&[], "123@s.whatsapp.net"));
}

#[test]
fn from_me_flag_is_conservative_without_owner() {
    assert!(from_me_of("+351900", Some("+351900")));
    assert!(!from_me_of("+351900", Some("+351911")));
    assert!(!from_me_of("+351900", None));
    assert!(!from_me_of("+351900", Some("")));
}

#[test]
fn search_knobs_clamp_to_the_feature_bounds() {
    assert_eq!(history::clamp_search(3650, 10_000), (90, 100));
    assert_eq!(history::clamp_search(0, 0), (1, 1));
    assert_eq!(history::clamp_search(7, 50), (7, 50));
}
