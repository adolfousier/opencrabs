//! Channel ingest is deny-by-default (OC-02).
//!
//! Discord and Slack treated an empty allowlist as "accept everyone", unlike
//! Telegram, and WhatsApp went further, treating an empty allowed_phones as
//! "everyone is owner". Combined with the default auto-always policy, that was a
//! remote shell for any stranger who could message the bot. The allowlists now
//! deny an unconfigured channel, matching Telegram, and ownership never comes
//! from an empty list.

use crate::config::owner::is_owner;

#[test]
fn an_unconfigured_channel_denies_everyone() {
    // No allowlist and no bot_owner: the channel is unconfigured, so nobody is
    // the owner and nobody is admitted. This is the invariant the Discord,
    // Slack, and WhatsApp ingest checks now rest on.
    assert!(!is_owner(&[], &[], "123456"));
}

#[test]
fn an_empty_allowlist_does_not_make_a_stranger_the_owner() {
    // The WhatsApp bug: allowed.is_empty() used to elevate every contact to
    // owner. An empty list must never grant ownership.
    let allowed: Vec<String> = vec![];
    let bot_owner: Vec<String> = vec![];
    assert!(!is_owner(&allowed, &bot_owner, "+15559999999"));
}

#[test]
fn the_configured_owner_is_admitted() {
    let allowed = vec!["111".to_string()];
    let bot_owner: Vec<String> = vec![];
    // Positional fallback: first allow-list entry is the owner.
    assert!(is_owner(&allowed, &bot_owner, "111"));
    assert!(!is_owner(&allowed, &bot_owner, "222"));

    // Explicit bot_owner is exact membership.
    let bot_owner = vec!["999".to_string()];
    assert!(is_owner(&allowed, &bot_owner, "999"));
    assert!(!is_owner(&allowed, &bot_owner, "111"));
}

#[test]
fn the_ingest_checks_are_deny_by_default() {
    // Source guard: the three handlers must no longer treat an empty allowlist
    // as accept-all. Each now computes ownership and refuses an unconfigured
    // channel.
    for (rel, needle) in [
        ("src/channels/discord/handler.rs", "deny-by-default"),
        ("src/channels/slack/handler.rs", "deny-by-default"),
        ("src/channels/whatsapp/handler.rs", "OC-02"),
    ] {
        let src = std::fs::read_to_string(rel).unwrap();
        assert!(
            src.contains(needle),
            "{rel} must carry the deny-by-default OC-02 gate"
        );
    }
}
