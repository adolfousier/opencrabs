//! Slack channel-history injection (#1620).
//!
//! Slack re-sent its whole 30-message window on every channel turn: no dedup
//! against the live session context, no #682 sender framing, and no thread
//! scoping, so parallel threads in one channel all saw each other's messages.
//!
//! The behaviour lives in `channels::group_history` (covered by
//! `group_history_test`) and the thread-scoping contract in
//! `slack_thread_persistence_test`. What can rot independently here is the
//! WIRING, and the handler needs a live socket to exercise, so it is pinned at
//! source level.

const HANDLER: &str = include_str!("../channels/slack/handler.rs");

#[test]
fn channel_history_is_deduped_through_the_shared_preamble() {
    assert!(
        HANDLER.contains("group_history::build_preamble("),
        "Slack must route channel history through the shared deduping preamble (#1620)"
    );
}

#[test]
fn the_hand_rolled_history_block_is_gone() {
    assert!(
        !HANDLER.contains("[Recent channel history ({} messages):"),
        "the raw un-deduped history block is back; it re-sends the window every turn"
    );
}

#[test]
fn channel_turns_carry_the_sender_label() {
    assert!(
        HANDLER.contains("group_history::current_sender_label("),
        "Slack channel turns must name the current sender so the model does not \
         address them by a name that only appears in the history (#682)"
    );
    assert!(
        HANDLER.contains("\"Slack channel\","),
        "the label must name the surface as a Slack channel"
    );
}

#[test]
fn the_history_fetch_is_scoped_to_the_thread() {
    // A channel-wide fetch pulled every parallel thread into context, the same
    // bleed #226 fixed for Telegram forum topics. The thread_id passed here
    // must be derived from thread_ts exactly as the write path stores it.
    assert!(
        HANDLER.contains("thread_id_str.as_deref()"),
        "the recent() call must pass this turn's thread, not None (#1620)"
    );
    assert!(
        HANDLER.contains("let thread_id_str = thread_ts.as_ref().map(|ts| ts.to_string());"),
        "the fetch key must be derived from thread_ts the same way the write \
         path derives it, or the two never match"
    );
}

#[test]
fn dms_keep_their_own_shape() {
    assert!(
        HANDLER.contains("[Slack DM from {user_name} ({user_id})]"),
        "the DM identity line must survive the channel-path rewrite"
    );
}
