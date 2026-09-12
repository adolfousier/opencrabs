//! Self-reaction construction on completion turns (#1409).

use crate::channels::whatsapp::reaction::{COMPLETION_EMOJI, build_self_reaction};

fn reaction_of(msg: &waproto::whatsapp::Message) -> &waproto::whatsapp::message::ReactionMessage {
    msg.reaction_message.as_ref().expect("reaction present")
}

#[test]
fn the_reaction_targets_the_message_by_id() {
    let msg = build_self_reaction(
        "15551234@s.whatsapp.net",
        "3EB0ABC",
        "✅",
        1_700_000_000_000,
    );
    let key = reaction_of(&msg).key.as_ref().expect("key present");

    assert_eq!(key.id.as_deref(), Some("3EB0ABC"));
    assert_eq!(key.remote_jid.as_deref(), Some("15551234@s.whatsapp.net"));
}

#[test]
fn reacting_to_our_own_message_is_always_from_me() {
    // A false here would address the RECIPIENT's message with the same id,
    // which is either nothing or the wrong message.
    let msg = build_self_reaction("jid@s.whatsapp.net", "ID", "✅", 0);
    let key = reaction_of(&msg).key.as_ref().unwrap();
    assert_eq!(key.from_me, Some(true));
}

#[test]
fn the_emoji_is_carried_verbatim() {
    let msg = build_self_reaction("jid@s.whatsapp.net", "ID", "🔥", 0);
    assert_eq!(reaction_of(&msg).text.as_deref(), Some("🔥"));
}

#[test]
fn an_empty_emoji_encodes_removal_rather_than_an_empty_string() {
    // WhatsApp reads a missing text as "take the reaction off". Sending
    // Some("") instead would be a reaction to nothing.
    let msg = build_self_reaction("jid@s.whatsapp.net", "ID", "", 0);
    assert_eq!(reaction_of(&msg).text, None);
}

#[test]
fn the_sender_timestamp_is_carried() {
    let msg = build_self_reaction("jid@s.whatsapp.net", "ID", "✅", 1_700_000_000_123);
    assert_eq!(
        reaction_of(&msg).sender_timestamp_ms,
        Some(1_700_000_000_123)
    );
}

#[test]
fn the_message_carries_nothing_but_the_reaction() {
    // A reaction message with a conversation body would post text as well.
    let msg = build_self_reaction("jid@s.whatsapp.net", "ID", "✅", 0);
    assert!(msg.conversation.is_none());
    assert!(msg.extended_text_message.is_none());
}

#[test]
fn the_completion_emoji_is_from_the_utilitarian_set() {
    assert!(
        ["👍", "👀", "🔥", "✅"].contains(&COMPLETION_EMOJI),
        "keep acknowledgements to the Telegram-parity set, not decoration"
    );
}
