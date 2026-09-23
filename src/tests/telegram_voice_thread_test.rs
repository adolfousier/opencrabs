//! TTS voice notes must carry the reply thread (#1683).
//!
//! Voice was the one Telegram send path still calling the bare
//! `bot.send_voice`, so in a forum topic the audio landed in General while
//! the text reply above it correctly stayed in the thread. The `*_in_thread`
//! helpers in `send.rs` exist so no call site has to remember the
//! `message_thread_id` wire field itself; voice now joins the family, and
//! these source-scan guards keep it there.

const DELIVERY_SRC: &str = include_str!("../channels/telegram/delivery.rs");
const SEND_SRC: &str = include_str!("../channels/telegram/send.rs");

#[test]
fn test_no_bare_send_voice_calls_in_delivery() {
    let bare = DELIVERY_SRC
        .lines()
        .filter(|l| l.contains(".send_voice("))
        .count();
    assert_eq!(
        bare, 0,
        "#1683: a bare .send_voice( drops the thread; route via send::voice_in_thread"
    );
}

#[test]
fn test_voice_helper_exists_in_the_in_thread_family() {
    assert!(
        SEND_SRC.contains("pub fn voice_in_thread<"),
        "send.rs must keep voice_in_thread alongside the other *_in_thread helpers"
    );
}

#[test]
fn test_tts_path_routes_through_voice_in_thread() {
    assert!(
        DELIVERY_SRC.contains("voice_in_thread(") && !DELIVERY_SRC.contains(".send_voice("),
        "#1683: the TTS voice send must go through send::voice_in_thread, not the bare bot call"
    );
}
