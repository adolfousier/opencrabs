//! WhatsApp group-history injection (#1618).
//!
//! WhatsApp re-sent its whole 30-message window on every group turn: no dedup
//! against the live session context, and no #682 sender framing, so the model
//! could address the current speaker by a name that only appeared in the
//! injected history.
//!
//! The behaviour lives in `channels::group_history` and is covered by
//! `group_history_test`. What can rot independently is the WIRING, and the
//! handler needs a live WhatsApp socket to exercise, so it is pinned at source
//! level.

const HANDLER: &str = include_str!("../channels/whatsapp/handler.rs");

#[test]
fn group_history_is_deduped_through_the_shared_preamble() {
    assert!(
        HANDLER.contains("group_history::build_preamble("),
        "WhatsApp must route group history through the shared deduping preamble (#1618)"
    );
}

#[test]
fn the_hand_rolled_history_block_is_gone() {
    assert!(
        !HANDLER.contains("[Recent group history ({} messages):"),
        "the raw un-deduped history block is back; it re-sends the window every turn"
    );
}

#[test]
fn group_turns_carry_the_sender_label() {
    assert!(
        HANDLER.contains("group_history::current_sender_label("),
        "WhatsApp group turns must name the current sender so the model does not \
         address them by a name that only appears in the history (#682)"
    );
    assert!(
        HANDLER.contains("\"WhatsApp group\","),
        "the label must name the surface as a WhatsApp group"
    );
}

#[test]
fn one_to_one_chats_keep_their_own_shape() {
    // A 1:1 chat has no other members, so it gets neither history nor the label.
    assert!(
        HANDLER.contains("[WhatsApp message from {}]"),
        "the one-to-one identity line must survive the group-path rewrite"
    );
}
