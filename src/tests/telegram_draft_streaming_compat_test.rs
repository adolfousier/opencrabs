//! Tests verifying TelegramConfig backward compatibility with legacy config keys.
//!
//! Specifically, `draft_streaming` was an experimental config flag introduced in v0.3.52
//! for Telegram DM draft streaming (sendRichMessageDraft) and subsequently dropped when
//! the Telegram streaming flow was unified.
//!
//! Because Serde defaults to ignoring undeclared fields on structs without `deny_unknown_fields`,
//! legacy `config.toml` files that still specify `draft_streaming = false` or `draft_streaming = true`
//! must continue deserializing cleanly without error.

use crate::config::{ChannelsConfig, TelegramConfig};

#[test]
fn legacy_draft_streaming_flag_is_safely_ignored_when_deserializing() {
    let toml_str = r#"
enabled = true
token = "test_token"
draft_streaming = false
rich_messages = true
"#;

    let parsed: Result<TelegramConfig, _> = toml::from_str(toml_str);
    assert!(
        parsed.is_ok(),
        "TelegramConfig must deserialize cleanly when legacy draft_streaming is present"
    );

    let config = parsed.unwrap();
    assert!(config.enabled);
    assert_eq!(config.token.as_deref(), Some("test_token"));
    assert!(config.rich_messages);
}

#[test]
fn legacy_channels_telegram_section_with_draft_streaming_deserializes_cleanly() {
    let toml_str = r#"
[telegram]
enabled = true
token = "123456:ABC-DEF"
draft_streaming = true
allowed_users = ["133526395"]
"#;

    let parsed: Result<ChannelsConfig, _> = toml::from_str(toml_str);
    assert!(
        parsed.is_ok(),
        "ChannelsConfig must deserialize cleanly when [telegram] specifies legacy draft_streaming"
    );

    let channels = parsed.unwrap();
    assert!(channels.telegram.enabled);
    assert_eq!(channels.telegram.token.as_deref(), Some("123456:ABC-DEF"));
    assert_eq!(channels.telegram.allowed_users, vec!["133526395"]);
}
