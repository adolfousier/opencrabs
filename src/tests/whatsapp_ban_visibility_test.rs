//! #1999: `Event::TemporaryBan`, `Event::ConnectFailure` and the `LoggedOut`
//! reason fell into the debug-only catch-all, so a ban or account lock left
//! no trace for the owner. The event arms need a live socket to fire, so
//! they are pinned source-level (the house pattern from the composing parity
//! test); the formatters run for real here.

use crate::channels::whatsapp::agent::{connect_failure_notice, logout_notice, temp_ban_notice};
use wacore::types::events::{ConnectFailureReason, TempBanReason};

const AGENT: &str = include_str!("../channels/whatsapp/agent.rs");

#[test]
fn logout_notice_keeps_the_reason_and_server_copy() {
    let locked = logout_notice(
        &ConnectFailureReason::AccountLocked,
        Some("account under review"),
    );
    assert!(locked.contains("AccountLocked"), "reason dropped: {locked}");
    assert!(
        locked.contains("account under review"),
        "server copy dropped: {locked}"
    );
    let plain = logout_notice(&ConnectFailureReason::LoggedOut, None);
    assert!(
        !plain.contains("None"),
        "the Option leaked into the notice: {plain}"
    );
}

#[test]
fn temp_ban_notice_shows_minutes_and_appeal_url() {
    let ban = temp_ban_notice(
        &TempBanReason::SentToTooManyPeople,
        chrono::Duration::seconds(3600),
        None,
        Some("https://wa.me/support"),
    );
    assert!(ban.contains("SentToTooManyPeople"), "{ban}");
    assert!(ban.contains("60 min"), "expire lost: {ban}");
    assert!(
        ban.contains("https://wa.me/support"),
        "appeal link lost: {ban}"
    );
}

#[test]
fn a_sub_minute_ban_never_reads_zero_minutes() {
    let ban = temp_ban_notice(
        &TempBanReason::BlockedByUsers,
        chrono::Duration::seconds(40),
        Some("slow down"),
        None,
    );
    assert!(ban.contains("1 min"), "floored to zero: {ban}");
    assert!(ban.contains("slow down"), "server message lost: {ban}");
}

#[test]
fn connect_failure_notice_keeps_the_server_message() {
    let cf = connect_failure_notice(&ConnectFailureReason::TempBanned, Some("spam threshold"));
    assert!(
        cf.contains("TempBanned") && cf.contains("spam threshold"),
        "{cf}"
    );
}

#[test]
fn the_event_arms_stop_discarding_the_payload() {
    assert!(
        !AGENT.contains("Event::LoggedOut(_)"),
        "the old discard arm is back: the LoggedOut reason must reach the notice"
    );
    assert!(
        AGENT.contains("Event::TemporaryBan(ban)"),
        "the ban arm is gone: it would fall into the debug catch-all"
    );
    assert!(
        AGENT.contains("Event::ConnectFailure(cf)"),
        "the connect-failure arm is gone"
    );
    for notice in ["logout_notice", "temp_ban_notice", "connect_failure_notice"] {
        assert!(
            AGENT.matches(&format!("{notice}(")).count() >= 2,
            "{notice} is defined but never wired into the event arms"
        );
    }
}
