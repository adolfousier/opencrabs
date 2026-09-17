//! Discord channel-history injection (#1619).
//!
//! Discord re-sent its whole 30-message window on every guild turn: no dedup
//! against the live session context, and no #682 sender framing, so the model
//! could address the current speaker by a name that only appeared in the
//! injected history.
//!
//! The behaviour itself lives in `channels::group_history` and is covered by
//! `group_history_test`. What can rot independently is the WIRING: the handler
//! silently falling back to a hand-rolled block, or dropping the sender label.
//! Those are source-level facts, so they are pinned at source level — the
//! handler needs a live gateway connection to exercise otherwise.

const HANDLER: &str = include_str!("../channels/discord/handler.rs");

#[test]
fn guild_history_is_deduped_through_the_shared_preamble() {
    assert!(
        HANDLER.contains("group_history::build_preamble("),
        "Discord must route channel history through the shared deduping preamble (#1619)"
    );
    assert!(
        HANDLER.contains("\"channel\","),
        "Discord's room noun is \"channel\", not \"group\""
    );
}

#[test]
fn the_hand_rolled_history_block_is_gone() {
    // The pre-#1619 shape: fetched rows formatted straight into the prompt with
    // no live-context filter and no skip path.
    assert!(
        !HANDLER.contains("[Recent channel history ({} messages):"),
        "the raw un-deduped history block is back; it re-sends the window every turn"
    );
}

#[test]
fn guild_turns_carry_the_sender_label() {
    assert!(
        HANDLER.contains("group_history::current_sender_label("),
        "Discord guild turns must name the current sender so the model does not \
         address them by a name that only appears in the history (#682)"
    );
    assert!(
        HANDLER.contains("\"Discord channel\","),
        "the label must name the surface as a Discord channel"
    );
}

#[test]
fn dms_keep_their_own_shape() {
    // A DM has no other members, so it gets neither history nor the #682 label.
    assert!(
        HANDLER.contains("[Discord DM from {name} (ID {uid})]"),
        "the DM identity line must survive the guild-path rewrite"
    );
}
