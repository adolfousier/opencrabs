//! A 429 learned anywhere reaches the process-wide cooldown (#1630).
//!
//! The global lock is only worth having if every path that recognises a 429
//! arms it. As merged, `record_global_429` was called from `wait_out` alone,
//! so a throttle discovered on a plan-card write or a queued final stayed
//! private to that one path and every other chat walked into the same wall.
//!
//! These drive the real failure handlers rather than re-deriving their
//! reasoning: each asserts the global deadline moved, which is the fact the
//! rest of the process reads.

use crate::channels::telegram::governor::test_support;
use crate::channels::telegram::plan_card::{
    EditOutcome, handle_create_failure, handle_edit_failure,
};
use crate::channels::telegram::rate_limit::{is_global_cooldown_active, reset_global_cooldown};
use crate::channels::telegram::state::TelegramState;
use teloxide::types::{ChatId, MessageId};
use uuid::Uuid;

/// A plan-card create that comes back throttled must arm the global cooldown,
/// not only suppress this session's card writes.
#[tokio::test]
async fn plan_card_create_throttle_arms_the_global_cooldown() {
    let _guard = test_support::registry_guard().await;
    test_support::reset(0);
    reset_global_cooldown();
    assert!(!is_global_cooldown_active(), "precondition: no cooldown");

    let state = TelegramState::new();
    handle_create_failure("Too Many Requests: retry after 9", &state, Uuid::new_v4()).await;

    assert!(
        is_global_cooldown_active(),
        "a 429 on card create must reach the process-wide deadline"
    );
    reset_global_cooldown();
}

/// Same for the edit path, which returns `Suppressed` and previously stopped
/// at this session's own card.
#[tokio::test]
async fn plan_card_edit_throttle_arms_the_global_cooldown() {
    let _guard = test_support::registry_guard().await;
    test_support::reset(0);
    reset_global_cooldown();
    assert!(!is_global_cooldown_active(), "precondition: no cooldown");

    let state = TelegramState::new();
    let outcome = handle_edit_failure(
        "Too Many Requests: retry after 9",
        &state,
        Uuid::new_v4(),
        ChatId(-100_123),
        None,
        "sig",
        MessageId(7),
    )
    .await;

    assert!(matches!(outcome, EditOutcome::Suppressed));
    assert!(
        is_global_cooldown_active(),
        "a 429 on card edit must reach the process-wide deadline"
    );
    reset_global_cooldown();
}

/// A non-throttle failure must leave the deadline alone: arming it on any
/// error would stall every chat on an unrelated edit failure.
#[tokio::test]
async fn a_non_throttle_card_failure_leaves_the_cooldown_alone() {
    let _guard = test_support::registry_guard().await;
    test_support::reset(0);
    reset_global_cooldown();

    let state = TelegramState::new();
    handle_create_failure("Bad Request: message text is empty", &state, Uuid::new_v4()).await;

    assert!(
        !is_global_cooldown_active(),
        "only a retry-after may arm the global deadline"
    );
}

/// The global permit must be taken before the per-chat `enabled` gate in both
/// pacers. Below it, switching the per-chat pacer off would also delete the
/// process-wide ceiling and the 429 wait, which is the opposite of what a
/// global limiter is for.
///
/// A source check because the ordering is the contract: both pacers return
/// early for DMs and consult live config, so a behavioural test would pin the
/// config plumbing rather than the sequence.
#[test]
fn both_pacers_take_the_global_permit_before_the_per_chat_gate() {
    let src = include_str!("../channels/telegram/governor.rs");

    for pacer in [
        "pub(crate) async fn pace_send(",
        "pub(crate) async fn pace_rich(",
    ] {
        let start = src
            .find(pacer)
            .unwrap_or_else(|| panic!("{pacer} not found — rename?"));
        let body = &src[start..];
        let permit = body
            .find("acquire_global_permit()")
            .unwrap_or_else(|| panic!("{pacer} no longer takes a global permit"));
        let gate = body
            .find("if !lim.enabled")
            .unwrap_or_else(|| panic!("{pacer} no longer has the per-chat enable gate"));
        assert!(
            permit < gate,
            "{pacer}: the global permit must be acquired before the per-chat \
             enable gate, otherwise disabling per-chat pacing also disables \
             the process-wide ceiling and the 429 cooldown"
        );
    }
}
