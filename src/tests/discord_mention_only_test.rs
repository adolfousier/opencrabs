//! #1890: a bare @mention must trigger even after the mention tag is stripped.
//!
//! Repro: reply to any message with nothing but `@OpenCrabs`. Old behavior:
//! gate passes, strip empties the content, the empty guard returns in silence.
//! New behavior: dispatch with the replied-to message resolved, or with the
//! visible ping itself as fallback. Empty non-mention messages still drop.

use crate::channels::discord::handler::{
    EmptyContentDecision, bare_mention_content, decide_empty_content, folded_message_text,
};
use serenity::model::channel::MessageSnapshot;

fn snapshot(content: &str, attachments: serde_json::Value) -> MessageSnapshot {
    serde_json::from_value(serde_json::json!({
        "content": content,
        "timestamp": "2026-10-03T04:43:32.000000+00:00",
        "mentions": [],
        "mention_roles": [],
        "attachments": attachments,
        "embeds": [],
        "type": 0
    }))
    .expect("valid MessageSnapshot fixture")
}

#[test]
fn mention_only_dispatches_after_strip() {
    let decision = decide_empty_content(true, None);
    match decision {
        EmptyContentDecision::DispatchWith(text) => {
            assert!(!text.is_empty(), "fallback content must survive the guard");
            assert!(text.contains("mentioned"), "ping must be visible context");
        }
        EmptyContentDecision::Drop(_) => panic!("bare mention must never drop"),
    }
}

#[test]
fn bare_mention_resolves_reply_reference() {
    let replied = "review bro\n\n[forwarded attachment]: bot-pack.zip https://cdn.discordapp.com/attachments/1/2/bot-pack.zip";
    match decide_empty_content(true, Some(replied)) {
        EmptyContentDecision::DispatchWith(text) => {
            assert!(text.contains("review bro"));
            assert!(text.contains("bot-pack.zip"));
        }
        EmptyContentDecision::Drop(_) => panic!("resolved reply must dispatch"),
    }
}

#[test]
fn empty_no_mention_still_dropped() {
    assert!(matches!(
        decide_empty_content(false, None),
        EmptyContentDecision::Drop(_)
    ));
}

#[test]
fn whitespace_reply_falls_back_to_ping() {
    assert_eq!(
        bare_mention_content(Some("   \n ")),
        bare_mention_content(None)
    );
}

#[test]
fn folded_message_text_combines_own_text_and_snapshot() {
    let snapshots = vec![snapshot("secret payload", serde_json::json!([]))];
    let folded = folded_message_text("review bro", &snapshots);
    assert!(folded.starts_with("review bro"));
    assert!(folded.contains("[forwarded message]: secret payload"));
    // Pure forward: own content empty, snapshot carries everything.
    let pure = folded_message_text("", &snapshots);
    assert_eq!(pure, "[forwarded message]: secret payload");
    // No snapshots: own content untouched.
    assert_eq!(folded_message_text("plain", &[]), "plain");
}
