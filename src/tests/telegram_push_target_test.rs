//! A push that belongs to a session is delivered to THAT session's address,
//! and the chat-wide lookup is reached only for a session with no binding
//! (#1200, #1319).
//!
//! The defect these cover: `TelegramState::session_topic` returns
//! `Option<i32>`, and its `.flatten()` collapses "bound to General" with "not
//! bound at all". Both reach the delivery path as `None`, which took the
//! chat-wide fallback — `latest_thread_id_for_chat`, i.e. "the topic that
//! spoke last". Measured live on 2026-09-21: the hourly digest of the
//! "Общее / Неразобранное" topic, whose session `ef52656d` carries a durable
//! `thread_id = NULL` on chat `-1002928607356`, was delivered as msg 16795
//! into the Голубицкая topic, because the agent's own reply there had just
//! made that topic the most recent speaker.

use crate::channels::telegram::session_resolve::{PushTarget, push_target};

const CHAT: i64 = -1002928607356;
const OTHER_CHAT: i64 = -1009999999999;

/// The bug, in one assertion: a General-bound session is a DEFINITE address
/// (no thread), so the chat-wide lookup must never be consulted for it.
#[test]
fn general_bound_session_is_not_treated_as_unbound() {
    let target = push_target(Some((CHAT, None)), None, CHAT);
    assert_eq!(
        target,
        PushTarget::Bound(None),
        "#1319: General is an address (the absence of a thread), not a \
         fall-through to whichever topic spoke last"
    );
}

#[test]
fn a_real_topic_keeps_its_thread() {
    assert_eq!(
        push_target(Some((CHAT, Some(1618))), None, CHAT),
        PushTarget::Bound(Some(1618)),
        "#1200: the owning topic is the destination"
    );
}

/// The chat-wide lookup is still right for a session nothing is known about.
#[test]
fn no_binding_at_all_falls_back_to_the_chat_lookup() {
    assert_eq!(push_target(None, None, CHAT), PushTarget::Unbound);
}

/// A binding for another chat says nothing about this one, so it must not be
/// applied — the #116 poisoning shape (a topic id sent into a chat that does
/// not have it is refused with `message thread not found`).
#[test]
fn a_binding_for_another_chat_does_not_apply() {
    assert_eq!(
        push_target(Some((OTHER_CHAT, Some(42))), None, CHAT),
        PushTarget::Unbound
    );
    assert_eq!(
        push_target(None, Some((OTHER_CHAT, Some(42))), CHAT),
        PushTarget::Unbound
    );
}

/// The window this closes: a push can arrive before the connect-time
/// re-registration (#1224) has filled the in-memory maps. The durable row is
/// then the only evidence, and it must be used rather than the chat lookup.
#[test]
fn durable_binding_is_used_when_the_maps_are_empty() {
    assert_eq!(
        push_target(None, Some((CHAT, Some(1618))), CHAT),
        PushTarget::Bound(Some(1618))
    );
    assert_eq!(
        push_target(None, Some((CHAT, None)), CHAT),
        PushTarget::Bound(None),
        "a durable General binding is still a definite address"
    );
}

/// In-memory wins: it is the live binding, refreshed on every ingress message.
#[test]
fn in_memory_binding_takes_precedence_over_the_durable_row() {
    assert_eq!(
        push_target(Some((CHAT, Some(5))), Some((CHAT, Some(1618))), CHAT),
        PushTarget::Bound(Some(5))
    );
}
