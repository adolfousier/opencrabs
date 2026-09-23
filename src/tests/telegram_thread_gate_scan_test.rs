//! Class-kill guards for the Telegram thread-id saga (#1319, #1683, #1708).
//!
//! Three incidents, one root cause: ad-hoc thread-id decisions at send and
//! rate-gate call sites. The upstream sources are certified (session-resolved
//! only); these source scans keep it that way mechanically:
//!
//! 1. every content send goes through the `*_in_thread` family — no bare
//!    `bot.send_*` calls that "just forget" the thread;
//! 2. rate-gate callers (`admit_chat_action`, `pace_rich`, `pace_send`) never
//!    receive a raw incoming-message thread id;
//! 3. `forum_seen` is armed only by the gate params (certified input) or the
//!    test helpers — a new marking site is a regression until proven otherwise.

const DELIVERY_SRC: &str = include_str!("../channels/telegram/delivery.rs");
const SEND_SRC: &str = include_str!("../channels/telegram/send.rs");
const RICH_API_SRC: &str = include_str!("../channels/telegram/rich/api.rs");
const RESUME_SRC: &str = include_str!("../channels/telegram/resume.rs");
const GOVERNOR_SRC: &str = include_str!("../channels/telegram/governor.rs");

const BARE_SEND_METHODS: [&str; 8] = [
    ".send_message(",
    ".send_photo(",
    ".send_document(",
    ".send_voice(",
    ".send_poll(",
    ".send_animation(",
    ".send_video(",
    ".send_audio(",
];

fn non_comment_lines(src: &str) -> impl Iterator<Item = &str> {
    src.lines().filter(|l| {
        let t = l.trim_start();
        !(t.starts_with("//") || t.starts_with('*') || t.starts_with("/*"))
    })
}

#[test]
fn test_delivery_has_no_bare_content_send_calls() {
    let offenders: Vec<&str> = non_comment_lines(DELIVERY_SRC)
        .filter(|l| BARE_SEND_METHODS.iter().any(|m| l.contains(m)))
        .collect();
    assert!(
        offenders.is_empty(),
        "#1319/#1683 class: bare content send drops the thread id. \
         Route through the send.rs *_in_thread family instead. Offenders: {offenders:?}"
    );
}

#[test]
fn test_in_thread_family_is_complete() {
    for helper in [
        "pub fn message_in_thread",
        "pub fn photo_in_thread",
        "pub fn document_in_thread",
        "pub fn voice_in_thread",
    ] {
        assert!(
            SEND_SRC.contains(helper),
            "send.rs must keep the *_in_thread family intact; missing `{helper}`"
        );
    }
}

#[test]
fn test_rate_gate_callers_never_receive_raw_incoming_thread_ids() {
    // A raw incoming reply-chain id is NOT proof of a forum. Gate callers
    // must pass session-certified ids only (see #1708: `note_thread_evidence`
    // and the gates armed forum pacing from raw ids in ordinary groups).
    let gate_calls = ["admit_chat_action(", "pace_rich(", "pace_send("];
    for (name, src) in [
        ("send.rs", SEND_SRC),
        ("rich/api.rs", RICH_API_SRC),
        ("resume.rs", RESUME_SRC),
    ] {
        let offenders: Vec<&str> = non_comment_lines(src)
            .filter(|l| gate_calls.iter().any(|g| l.contains(g)))
            .filter(|l| l.contains("msg.thread_id") || l.contains("raw_thread"))
            .collect();
        assert!(
            offenders.is_empty(),
            "#1708 class: {name} feeds a raw incoming thread id into a rate gate; \
             resolve it through the session topic first. Offenders: {offenders:?}"
        );
    }
}

#[test]
fn test_forum_seen_marking_sites_are_a_closed_set() {
    // Four sites, accounted for: admit_chat_action (certified param),
    // pace_rich (certified param), mark_forum + burn_bucket (#[cfg(test)]
    // helpers). A fifth means someone is arming forum behaviour from an
    // unvetted source — stop and justify it in the same PR.
    let markings: Vec<&str> = non_comment_lines(GOVERNOR_SRC)
        .filter(|l| l.contains("forum_seen = true"))
        .collect();
    assert_eq!(
        markings.len(),
        4,
        "#1708 class: unexpected forum_seen marking site appeared in governor.rs: {markings:?}"
    );
}
