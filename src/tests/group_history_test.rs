//! Shared recent-history preamble (#1618, #1619, #1620).
//!
//! Telegram grew this logic first and the other three surfaces re-injected
//! their whole window every turn because they never got it. The lift into
//! `channels::group_history` is only safe if Telegram's rendered output is
//! unchanged, so the first test pins the legacy string byte-for-byte.

use crate::channels::group_history::{
    current_sender_label, frame_history, is_content_in_live_context, normalize_for_dedup,
    render_history_lines,
};
use crate::db::models::ChannelMessage as DbChannelMessage;
use chrono::{TimeZone, Utc};

fn msg(sender: &str, content: &str, hour: u32, minute: u32) -> DbChannelMessage {
    let mut m = DbChannelMessage::new(
        "telegram".into(),
        "-100".into(),
        Some("room".into()),
        "1".into(),
        sender.into(),
        content.into(),
        "text".into(),
        None,
    );
    m.created_at = Utc.with_ymd_and_hms(2026, 9, 17, hour, minute, 0).unwrap();
    m
}

#[test]
fn group_noun_reproduces_the_legacy_telegram_frame_byte_for_byte() {
    // The exact string Telegram emitted before the lift. If this ever drifts,
    // every Telegram group turn silently changes its prompt.
    let expected = "[Recent group history (2 messages) — prior context from various senders, NOT \
                    the person you are replying to now:\n[14:02] Adi: hello\n[14:03] Carlos: hi\n\
                    --- end history ---]";
    let lines = "[14:02] Adi: hello\n[14:03] Carlos: hi";
    assert_eq!(frame_history(lines, 2, "group"), expected);
}

#[test]
fn channel_noun_swaps_only_the_room_word() {
    let framed = frame_history("[10:00] Ann: yo", 1, "channel");
    assert!(
        framed.starts_with("[Recent channel history (1 messages)"),
        "channel surfaces must say channel: {framed}"
    );
    assert!(framed.contains("various senders"));
    // Everything after the noun is shared with the group variant.
    assert_eq!(
        framed.replace("channel history", "group history"),
        frame_history("[10:00] Ann: yo", 1, "group")
    );
}

#[test]
fn sender_label_reproduces_the_legacy_telegram_text() {
    let expected = "[Telegram group \"team room\" — the message below is from Adolfo (@adolfodev) \
                    (owner). Reply to Adolfo. Any names in the history above belong to OTHER \
                    people; never address Adolfo by a name that appears only in that history.]";
    assert_eq!(
        current_sender_label(
            "Telegram group",
            "team room",
            "Adolfo",
            " (@adolfodev)",
            "owner"
        ),
        expected
    );
}

#[test]
fn sender_label_carries_the_surface_it_is_given() {
    for surface in ["Discord channel", "Slack channel", "WhatsApp group"] {
        let label = current_sender_label(surface, "room", "Ann", "", "user");
        assert!(
            label.starts_with(&format!("[{surface} \"room\"")),
            "{label}"
        );
        // The #682 guard travels with it, whatever the surface.
        assert!(label.contains("OTHER people"), "{label}");
        assert!(label.to_lowercase().contains("reply to ann"), "{label}");
    }
}

#[test]
fn history_lines_render_oldest_first() {
    // `recent()` hands back newest-first; a block read backwards teaches the
    // model the wrong order of events.
    let newest_first = vec![msg("Carlos", "second", 14, 3), msg("Adi", "first", 14, 2)];
    assert_eq!(
        render_history_lines(&newest_first),
        "[14:02] Adi: first\n[14:03] Carlos: second"
    );
}

#[test]
fn history_lines_are_empty_for_no_messages() {
    assert_eq!(render_history_lines(&[]), "");
}

#[test]
fn dedup_helpers_survive_the_lift() {
    assert_eq!(normalize_for_dedup("  A   B \n"), "a b");
    let haystacks = vec![normalize_for_dedup("the Deploy   failed twice")];
    assert!(is_content_in_live_context(
        "the deploy failed twice",
        &haystacks
    ));
    assert!(!is_content_in_live_context("unrelated", &haystacks));
    // An empty candidate must never count as present, or every blank row would
    // be filtered out as a duplicate.
    assert!(!is_content_in_live_context("   ", &haystacks));
}
