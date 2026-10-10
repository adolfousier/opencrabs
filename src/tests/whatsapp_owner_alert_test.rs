//! #1999: the WhatsApp ban and lock alerts that go to the owner over Telegram.
//! The owner alert is built in the manager and reached only through a live
//! socket, so the wording and the decisions are pinned here; the event arms are
//! pinned source-level in `whatsapp_ban_visibility_test.rs`.

use crate::channels::whatsapp::agent::{ban_alert, lock_alert};
use wacore::types::events::{ConnectFailureReason, TempBanReason};

#[test]
fn a_temporary_ban_names_its_reason_expiry_and_appeal() {
    let msg = ban_alert(
        &TempBanReason::SentTooManySameMessage,
        chrono::Duration::hours(3),
        Some("https://example.invalid/appeal"),
    );
    assert!(msg.contains("SentTooManySameMessage"), "{msg}");
    assert!(msg.contains("about 3 h"), "{msg}");
    assert!(
        msg.contains("Appeal: https://example.invalid/appeal"),
        "{msg}"
    );
}

#[test]
fn a_ban_without_an_appeal_link_says_nothing_about_one() {
    let msg = ban_alert(
        &TempBanReason::BlockedByUsers,
        chrono::Duration::hours(1),
        None,
    );
    assert!(!msg.contains("Appeal"), "{msg}");
}

#[test]
fn an_account_lock_tells_the_owner_not_to_re_pair() {
    let msg = lock_alert(&ConnectFailureReason::AccountLocked).expect("locks are alertable");
    assert!(msg.contains("403"), "{msg}");
    assert!(msg.contains("Do not re-pair"), "{msg}");
}

#[test]
fn a_connect_time_temporary_ban_is_alertable() {
    let msg = lock_alert(&ConnectFailureReason::TempBanned).expect("402 is alertable");
    assert!(msg.contains("402"), "{msg}");
}

#[test]
fn a_routine_logout_is_not_alerted() {
    // A manual unlink arrives as a device_removed conflict and reads as a
    // routine logout; it must not page the owner.
    assert_eq!(lock_alert(&ConnectFailureReason::LoggedOut), None);
    assert_eq!(lock_alert(&ConnectFailureReason::Generic), None);
}
